//! vinoWatch: CPU, RAM and NPU use of the OpenVINO apps on this machine.

mod cli;
mod detect;
mod driver;
#[cfg(feature = "tray")]
mod icon;
#[cfg(feature = "tray")]
mod instance;
mod npu;
#[cfg(feature = "tray")]
mod png;
#[cfg(feature = "tray")]
mod raster;
mod sample;
#[cfg(feature = "tray")]
mod tray;

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::thread::sleep;
use std::time::Duration;

use cli::{Command, Mode};
use sample::Sampler;

fn main() -> ExitCode {
    match cli::parse(std::env::args().skip(1)) {
        Ok(Command::Help) => println!("{}", cli::USAGE),
        Ok(Command::Version) => println!("vinowatch {}", env!("CARGO_PKG_VERSION")),
        #[cfg(feature = "tray")]
        Ok(Command::InstallIcons(dir)) => match icon::install(&dir) {
            Ok(paths) => paths.iter().for_each(|p| println!("{}", p.display())),
            Err(e) => {
                eprintln!(
                    "vinowatch: couldn't write the icons under {}: {e}",
                    dir.display()
                );
                return ExitCode::FAILURE;
            }
        },
        #[cfg(not(feature = "tray"))]
        Ok(Command::InstallIcons(_)) => unreachable!("cli refuses it without the tray"),
        Ok(Command::CheckNpu { python }) => return check_npu(python.as_deref()),
        Ok(Command::Run {
            mode,
            interval,
            warn_below,
            probe_python,
        }) => return run(mode, interval, warn_below, probe_python),
        Err(msg) => {
            eprintln!("vinowatch: {msg}\n\n{}", cli::USAGE);
            return ExitCode::from(2);
        }
    }
    ExitCode::SUCCESS
}

fn run(mode: Mode, interval: Duration, warn_below: u8, probe_python: Option<PathBuf>) -> ExitCode {
    match mode {
        #[cfg(feature = "tray")]
        Mode::Tray => {
            // Keeps the lock until the tray exits. A second copy exits
            // cleanly, so the unit doesn't restart it in a loop.
            let _instance = match instance::lock_path().map(|p| instance::acquire(&p)) {
                Some(Ok(None)) => {
                    eprintln!("vinowatch: the tray is already running");
                    return ExitCode::SUCCESS;
                }
                Some(Ok(held)) => held,
                Some(Err(e)) => {
                    eprintln!("vinowatch: couldn't take the single-instance lock: {e}");
                    None
                }
                None => None,
            };
            tray_main(interval, warn_below, probe_python)
        }
        #[cfg(not(feature = "tray"))]
        Mode::Tray => unreachable!("cli only picks the tray when it is compiled in"),
        Mode::Console { json, once } => {
            // Only the tray offers the probe.
            let _ = probe_python;
            console(json, once, interval, warn_below);
            ExitCode::SUCCESS
        }
    }
}

#[cfg(feature = "tray")]
fn tray_main(interval: Duration, warn_below: u8, probe_python: Option<PathBuf>) -> ExitCode {
    match tray::run(interval, warn_below, probe_python) {
        Ok(()) => ExitCode::SUCCESS,
        Err(msg) => {
            eprintln!("vinowatch: {msg}\nWithout a tray host, run `vinowatch --console`.");
            ExitCode::FAILURE
        }
    }
}

/// `--check-npu`: 0 healthy, 1 stuck, 3 couldn't tell.
fn check_npu(explicit: Option<&Path>) -> ExitCode {
    // One poll, for the interpreters of any vino apps running right now.
    let snap = Sampler::new(0).poll();
    let python = driver::pick_python(explicit, &snap.pythons);
    match driver::probe(&python) {
        driver::Verdict::Healthy => {
            println!("NPU driver OK");
            ExitCode::SUCCESS
        }
        driver::Verdict::Stuck => {
            println!(
                "NPU driver stuck: close the apps using the NPU, then reload it with\n  \
                 systemctl start {} (after sudo ./install-npu-reset.sh), or\n  \
                 sudo modprobe -r intel_vpu && sudo modprobe intel_vpu",
                driver::RESET_UNIT
            );
            ExitCode::FAILURE
        }
        driver::Verdict::Unknown(why) => {
            println!("Couldn't tell ({}): {why}", python.display());
            ExitCode::from(3)
        }
    }
}

fn console(json: bool, once: bool, interval: Duration, warn_below: u8) {
    if !json {
        println!(
            "vinoWatch: monitoring OpenVINO processes every {}s",
            interval.as_secs()
        );
    }
    let mut sampler = Sampler::new(warn_below);
    // CPU % and NPU % are deltas; this poll only sets the baseline.
    sampler.poll();
    loop {
        sleep(interval);
        let snap = sampler.poll();
        if json {
            println!("{}", snap.to_json());
        } else {
            println!("{}", snap.npu_label());
            println!("{}", snap.ram_label());
            if snap.procs.is_empty() {
                println!("  no OpenVINO processes found");
            }
            for p in &snap.procs {
                println!("  {}", p.label());
            }
        }
        if once {
            return;
        }
    }
}
