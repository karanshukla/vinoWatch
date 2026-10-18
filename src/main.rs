//! vinoWatch: CPU, RAM and NPU use of the OpenVINO apps on this machine.

mod cli;
mod detect;
#[cfg(feature = "tray")]
mod instance;
mod npu;
mod sample;
#[cfg(feature = "tray")]
mod tray;

use std::process::ExitCode;
use std::thread::sleep;
use std::time::Duration;

use cli::{Command, Mode};
use sample::Sampler;

fn main() -> ExitCode {
    match cli::parse(std::env::args().skip(1)) {
        Ok(Command::Help) => println!("{}", cli::USAGE),
        Ok(Command::Version) => println!("vinowatch {}", env!("CARGO_PKG_VERSION")),
        Ok(Command::Run {
            mode,
            interval,
            warn_below,
        }) => return run(mode, interval, warn_below),
        Err(msg) => {
            eprintln!("vinowatch: {msg}\n\n{}", cli::USAGE);
            return ExitCode::from(2);
        }
    }
    ExitCode::SUCCESS
}

fn run(mode: Mode, interval: Duration, warn_below: u8) -> ExitCode {
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
            tray_main(interval, warn_below)
        }
        #[cfg(not(feature = "tray"))]
        Mode::Tray => unreachable!("cli only picks the tray when it is compiled in"),
        Mode::Console { json, once } => {
            console(json, once, interval, warn_below);
            ExitCode::SUCCESS
        }
    }
}

#[cfg(feature = "tray")]
fn tray_main(interval: Duration, warn_below: u8) -> ExitCode {
    match tray::run(interval, warn_below) {
        Ok(()) => ExitCode::SUCCESS,
        Err(msg) => {
            eprintln!("vinowatch: {msg}\nWithout a tray host, run `vinowatch --console`.");
            ExitCode::FAILURE
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
