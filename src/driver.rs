//! The stuck `intel_vpu` state, and the driver reload that clears it.
//!
//! After heavy NPU use the driver can get into a state where a second
//! OpenVINO Core in a process breaks the first one's NPU requests: the next
//! inference segfaults in `libze_loader` (`zeMemAllocHost`). vinoPop and
//! vinoAuthFace crash on every NPU action until the driver is reloaded. The
//! probe recreates the trigger in a throwaway process, as vinoPop's
//! `vino_ai/npu_canary.py` does; the reload is a root oneshot unit that
//! `install-npu-reset.sh` installs, with a polkit rule that lets the active
//! local user start it.

use std::io::Read;
use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant};

/// The system unit `install-npu-reset.sh` installs.
pub const RESET_UNIT: &str = "vino-npu-reset.service";
#[cfg(feature = "tray")]
const RESET_UNIT_PATH: &str = "/etc/systemd/system/vino-npu-reset.service";

/// A tiny model on the NPU, a second Core listing devices, then the model
/// again: the second inference is the one that segfaults. vinoPop's probe,
/// with telemetry blocked the same way.
const PROBE: &str = r#"
import sys
sys.modules["openvino_telemetry"] = None
import numpy as np
import openvino as ov
import openvino.opset13 as op

x = op.parameter([1, 64], np.float32)
w = op.constant(np.ones((64, 16), np.float32))
model = ov.Model([op.relu(op.matmul(x, w, False, False))], [x])
request = ov.Core().compile_model(model, "NPU").create_infer_request()
data = np.ones((1, 64), np.float32)
request.infer([data])
ov.Core().available_devices
request.infer([data])
"#;

/// Well over the ~0.3 s a healthy probe takes, for a cold NPU compile.
const PROBE_TIMEOUT: Duration = Duration::from_secs(60);

const SIGSEGV: i32 = 11;

#[derive(Clone, Debug, PartialEq)]
pub enum Verdict {
    Healthy,
    Stuck,
    /// The probe failed some other way, which says nothing about this state:
    /// no openvino in that Python, no NPU, a timeout.
    Unknown(String),
}

/// Runs the probe with `python`. Blocks for up to a minute.
pub fn probe(python: &Path) -> Verdict {
    probe_with(python, &["-I", "-c", PROBE], PROBE_TIMEOUT)
}

fn probe_with(program: &Path, args: &[&str], timeout: Duration) -> Verdict {
    let mut child = match Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(e) => return Verdict::Unknown(format!("couldn't run {}: {e}", program.display())),
    };
    // Drained on its own thread, so a chatty failure can't fill the pipe and
    // hang the probe until the timeout.
    let mut stderr = child.stderr.take().expect("stderr is piped");
    let reader = thread::spawn(move || {
        let mut text = String::new();
        let _ = stderr.read_to_string(&mut text);
        text
    });
    let status = match wait_timeout(&mut child, timeout) {
        Some(status) => status,
        None => {
            let _ = child.kill();
            let _ = child.wait();
            return Verdict::Unknown(format!("timed out after {} s", timeout.as_secs()));
        }
    };
    let stderr = reader.join().unwrap_or_default();
    verdict(status, &stderr)
}

fn wait_timeout(child: &mut std::process::Child, timeout: Duration) -> Option<ExitStatus> {
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Some(status),
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(50)),
            _ => return None,
        }
    }
}

fn verdict(status: ExitStatus, stderr: &str) -> Verdict {
    if status.success() {
        Verdict::Healthy
    } else if status.signal() == Some(SIGSEGV) {
        Verdict::Stuck
    } else {
        // A Python traceback ends with the line that says what went wrong.
        let last = stderr.lines().rev().find(|l| !l.trim().is_empty());
        Verdict::Unknown(match last {
            Some(line) => line.trim().to_string(),
            None => format!("probe failed ({status})"),
        })
    }
}

/// The interpreter of a running Python OpenVINO app: `argv[0]` when it is an
/// absolute path to a `python*`, which for the vino apps is their venv's.
/// `/proc/<pid>/exe` would be the base interpreter, outside the venv, which
/// can't import openvino.
pub fn python_of(cmdline: &[String]) -> Option<PathBuf> {
    let argv0 = Path::new(cmdline.first()?);
    let name = argv0.file_name()?.to_str()?;
    (argv0.is_absolute() && name.starts_with("python")).then(|| argv0.to_path_buf())
}

/// `--probe-python` if given, else the newest interpreter seen running an
/// OpenVINO app that still exists, else `python3` from `PATH`.
pub fn pick_python(explicit: Option<&Path>, seen: &[PathBuf]) -> PathBuf {
    explicit
        .map(Path::to_path_buf)
        .or_else(|| seen.iter().rev().find(|p| p.is_file()).cloned())
        .unwrap_or_else(|| "python3".into())
}

/// Whether `install-npu-reset.sh` has been run.
#[cfg(feature = "tray")]
pub fn reset_installed() -> bool {
    Path::new(RESET_UNIT_PATH).is_file()
}

/// Starts the reload unit and waits for it. `modprobe -r` refuses while
/// anything holds `/dev/accel`, so this fails rather than pull the NPU out
/// from under a running app.
#[cfg(feature = "tray")]
pub fn reset() -> Result<(), String> {
    let out = Command::new("systemctl")
        .args(["--no-ask-password", "start", RESET_UNIT])
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("couldn't run systemctl: {e}"))?;
    if out.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&out.stderr);
    Err(stderr
        .lines()
        .find(|l| !l.trim().is_empty())
        .map(|l| l.trim().to_string())
        .unwrap_or_else(|| format!("systemctl failed ({})", out.status)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sh(script: &str) -> Verdict {
        probe_with(
            Path::new("/bin/sh"),
            &["-c", script],
            Duration::from_secs(10),
        )
    }

    #[test]
    fn a_segfault_means_stuck() {
        assert_eq!(sh("kill -SEGV $$"), Verdict::Stuck);
        assert_eq!(sh("exit 0"), Verdict::Healthy);
    }

    #[test]
    fn other_failures_say_why() {
        assert_eq!(
            sh(
                "echo Traceback >&2; echo \"ModuleNotFoundError: No module named 'openvino'\" >&2; exit 1"
            ),
            Verdict::Unknown("ModuleNotFoundError: No module named 'openvino'".into())
        );
        assert!(matches!(sh("kill -ABRT $$"), Verdict::Unknown(_)));
        assert!(matches!(
            probe_with(Path::new("/nonexistent/python3"), &[], Duration::from_secs(1)),
            Verdict::Unknown(m) if m.starts_with("couldn't run")
        ));
    }

    #[test]
    fn a_hung_probe_times_out() {
        let t0 = Instant::now();
        let got = probe_with(Path::new("/bin/sleep"), &["30"], Duration::from_millis(200));
        assert!(matches!(got, Verdict::Unknown(m) if m.starts_with("timed out")));
        assert!(t0.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn a_noisy_probe_does_not_block_on_its_pipe() {
        // Far more than a pipe buffer holds.
        assert_eq!(
            sh("head -c 1000000 /dev/zero | tr '\\0' x >&2; exit 0"),
            Verdict::Healthy
        );
    }

    #[test]
    fn the_interpreter_comes_from_an_absolute_argv0() {
        let cmd = |args: &[&str]| args.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(
            python_of(&cmd(&["/home/k/vinoWhisper/.venv/bin/python3", "-m", "x"])),
            Some("/home/k/vinoWhisper/.venv/bin/python3".into())
        );
        assert_eq!(python_of(&cmd(&["python3", "main.py"])), None);
        assert_eq!(python_of(&cmd(&["/usr/bin/vinosearch"])), None);
        assert_eq!(python_of(&[]), None);
    }

    #[test]
    fn an_explicit_python_wins_then_the_newest_one_that_exists() {
        let sh = PathBuf::from("/bin/sh");
        let gone = PathBuf::from("/nonexistent/python3");
        assert_eq!(
            pick_python(Some(Path::new("/opt/py")), std::slice::from_ref(&sh)),
            PathBuf::from("/opt/py")
        );
        assert_eq!(pick_python(None, &[sh.clone(), gone.clone()]), sh);
        assert_eq!(pick_python(None, &[gone]), PathBuf::from("python3"));
    }
}
