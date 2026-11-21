//! The stuck `intel_vpu` state, and the driver reload that clears it: a second
//! OpenVINO Core in a process breaks the first one's NPU requests (the next
//! inference segfaults in `libze_loader`). The probe recreates that in a
//! throwaway process, as vinoPop's `npu_canary.py` does; the reload is the root
//! unit `install-npu-reset.sh` installs, behind a polkit rule.

use std::fs;
use std::io::Read;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
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
/// again: the second inference segfaults. vinoPop's probe.
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

/// A sticky directory (like /tmp) lets others add entries but not replace ours.
const STICKY: u32 = 0o1000;
const GROUP_WRITE: u32 = 0o020;
const OTHER_WRITE: u32 = 0o002;

#[derive(Clone, Debug, PartialEq)]
pub enum Verdict {
    Healthy,
    Stuck,
    /// The probe failed some other way: no openvino in that Python, no NPU, a timeout.
    Unknown(String),
}

/// Runs the probe with `python`. Blocks for up to a minute.
pub fn probe(python: &Path) -> Verdict {
    probe_with(python, &["-I", "-c", PROBE], PROBE_TIMEOUT)
}

/// Drains stderr on its own thread so a chatty failure can't fill the pipe
/// (pinned by `tests::a_noisy_probe_does_not_block_on_its_pipe`).
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
        Verdict::Unknown(match last_nonblank_line(stderr) {
            Some(line) => line.trim().to_string(),
            None => format!("probe failed ({status})"),
        })
    }
}

fn last_nonblank_line(text: &str) -> Option<&str> {
    text.lines().rev().find(|l| !l.trim().is_empty())
}

/// `argv[0]` of a running Python OpenVINO app when it is an absolute path to a
/// `python*`: their venv's, where `/proc/<pid>/exe` is the base interpreter,
/// which can't import openvino.
pub fn python_of(cmdline: &[String]) -> Option<PathBuf> {
    let argv0 = Path::new(cmdline.first()?);
    let name = argv0.file_name()?.to_str()?;
    (argv0.is_absolute() && name.starts_with("python")).then(|| argv0.to_path_buf())
}

/// `--probe-python`, else the newest trusted interpreter seen running an
/// OpenVINO app, else `python3` (pinned by `tests::a_venv_nobody_else_can_write_is_trusted`,
/// `tests::a_world_writable_interpreter_is_refused`).
pub fn pick_python(explicit: Option<&Path>, seen: &[PathBuf]) -> PathBuf {
    explicit
        .map(Path::to_path_buf)
        .or_else(|| {
            let me = effective_ids()?;
            seen.iter()
                .rev()
                .find(|p| p.is_file() && trusted(p, me))
                .cloned()
        })
        .unwrap_or_else(|| "python3".into())
}

fn effective_ids() -> Option<(u32, u32)> {
    let meta = fs::metadata("/proc/self").ok()?;
    Some((meta.uid(), meta.gid()))
}

/// Whether nobody but us and root could have put `python` there. Python reads
/// `pyvenv.cfg` from next to a venv's link (or one directory above), so the
/// binary's own directory and its immediate parent must not be world-writable
/// even when sticky (`/tmp/python3` or `/tmp/bin/python3` would read `/tmp/pyvenv.cfg`).
fn trusted(python: &Path, me: (u32, u32)) -> bool {
    let Ok(target) = python.canonicalize() else {
        return false;
    };
    [python, &target].iter().all(|p| {
        p.ancestors()
            .enumerate()
            .all(|(depth, dir)| safe_owner(dir, me, depth >= 3))
    })
}

/// Owned by us or root, and not writable by anyone else; our own group may
/// (Fedora's `umask 002`).
fn safe_owner(path: &Path, (uid, gid): (u32, u32), allow_sticky: bool) -> bool {
    let Ok(meta) = fs::symlink_metadata(path) else {
        return false;
    };
    let ours_or_root = meta.uid() == uid || meta.uid() == 0;
    let mode_is_always_0777 = meta.file_type().is_symlink();
    ours_or_root && (mode_is_always_0777 || others_cannot_replace(&meta, gid, allow_sticky))
}

fn others_cannot_replace(meta: &fs::Metadata, our_gid: u32, allow_sticky: bool) -> bool {
    let mode = meta.permissions().mode();
    let sticky_dir = allow_sticky && meta.is_dir() && mode & STICKY != 0;
    let group_may_write = mode & GROUP_WRITE != 0 && meta.gid() != our_gid;
    !group_may_write && (sticky_dir || mode & OTHER_WRITE == 0)
}

/// Whether `install-npu-reset.sh` has been run.
#[cfg(feature = "tray")]
pub fn reset_installed() -> bool {
    Path::new(RESET_UNIT_PATH).is_file()
}

/// Starts the reload unit and waits. `modprobe -r` refuses while anything holds
/// `/dev/accel`, so this fails rather than pull the NPU from a running app.
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
    }

    #[test]
    fn a_clean_exit_means_healthy() {
        assert_eq!(sh("exit 0"), Verdict::Healthy);
    }

    #[test]
    fn a_failure_says_the_last_line_of_its_traceback() {
        assert_eq!(
            sh(
                "echo Traceback >&2; echo \"ModuleNotFoundError: No module named 'openvino'\" >&2; exit 1"
            ),
            Verdict::Unknown("ModuleNotFoundError: No module named 'openvino'".into())
        );
    }

    #[test]
    fn a_crash_other_than_a_segfault_is_unknown() {
        assert!(matches!(sh("kill -ABRT $$"), Verdict::Unknown(_)));
    }

    #[test]
    fn a_missing_interpreter_is_unknown() {
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
        const FAR_MORE_THAN_A_PIPE_HOLDS: usize = 1_000_000;
        assert_eq!(
            sh(&format!(
                "head -c {FAR_MORE_THAN_A_PIPE_HOLDS} /dev/zero | tr '\\0' x >&2; exit 0"
            )),
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

    fn chmod(path: &Path, mode: u32) {
        fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
    }

    struct Venv {
        _scratch: tempfile::TempDir,
        root: PathBuf,
        python: PathBuf,
    }

    impl Venv {
        fn new() -> Self {
            let scratch = tempfile::tempdir().unwrap();
            let root = scratch.path().to_path_buf();
            let bin = root.join("venv/bin");
            fs::create_dir_all(&bin).unwrap();
            for dir in [&root, &root.join("venv"), &bin] {
                chmod(dir, 0o755);
            }
            let python = bin.join("python3");
            fs::write(&python, "").unwrap();
            chmod(&python, 0o755);
            Venv {
                _scratch: scratch,
                root,
                python,
            }
        }

        fn picked(&self) -> PathBuf {
            pick_python(None, std::slice::from_ref(&self.python))
        }

        fn link_python_to(&self, target: &Path) {
            fs::remove_file(&self.python).unwrap();
            std::os::unix::fs::symlink(target, &self.python).unwrap();
        }
    }

    #[test]
    fn a_venv_nobody_else_can_write_is_trusted() {
        let venv = Venv::new();
        assert_eq!(venv.picked(), venv.python);
    }

    #[test]
    fn a_world_writable_interpreter_is_refused() {
        let venv = Venv::new();
        chmod(&venv.python, 0o757);
        assert_eq!(venv.picked(), PathBuf::from("python3"));
    }

    #[test]
    fn a_world_writable_bin_dir_is_refused() {
        let venv = Venv::new();
        chmod(venv.python.parent().unwrap(), 0o777);
        assert_eq!(venv.picked(), PathBuf::from("python3"));
    }

    #[test]
    fn a_world_writable_venv_dir_is_refused() {
        let venv = Venv::new();
        chmod(&venv.root.join("venv"), 0o757);
        assert_eq!(venv.picked(), PathBuf::from("python3"));
    }

    #[test]
    fn a_sticky_world_writable_venv_or_bin_dir_is_refused() {
        let venv = Venv::new();
        chmod(&venv.root.join("venv"), 0o1777);
        assert_eq!(venv.picked(), PathBuf::from("python3"));

        let venv2 = Venv::new();
        chmod(venv2.python.parent().unwrap(), 0o1777);
        assert_eq!(venv2.picked(), PathBuf::from("python3"));
    }

    #[test]
    fn a_link_to_a_root_owned_interpreter_is_trusted() {
        let venv = Venv::new();
        venv.link_python_to(Path::new("/bin/sh"));
        assert_eq!(venv.picked(), venv.python);
    }

    #[test]
    fn a_link_into_a_world_writable_directory_is_refused() {
        let venv = Venv::new();
        let open = venv.root.join("open");
        fs::create_dir_all(&open).unwrap();
        chmod(&open, 0o777);
        fs::write(open.join("python3"), "").unwrap();
        chmod(&open.join("python3"), 0o755);
        venv.link_python_to(&open.join("python3"));
        assert_eq!(venv.picked(), PathBuf::from("python3"));
    }

    #[test]
    fn roots_files_are_taken_as_given() {
        let sh = PathBuf::from("/bin/sh");
        assert_eq!(pick_python(None, std::slice::from_ref(&sh)), sh);
    }

    #[test]
    fn an_explicit_probe_python_is_taken_as_given() {
        let venv = Venv::new();
        chmod(&venv.python, 0o777);
        assert_eq!(pick_python(Some(&venv.python), &[]), venv.python);
    }
}
