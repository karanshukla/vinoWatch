//! vinoWatch: CPU, RAM and NPU use of the OpenVINO apps on this machine.

mod cli;
mod detect;
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
        Ok(Command::Run { mode, interval }) => return run(mode, interval),
        Err(msg) => {
            eprintln!("vinowatch: {msg}\n\n{}", cli::USAGE);
            return ExitCode::from(2);
        }
    }
    ExitCode::SUCCESS
}

fn run(mode: Mode, interval: Duration) -> ExitCode {
    match mode {
        #[cfg(feature = "tray")]
        Mode::Tray => match tray::run(interval) {
            Ok(()) => ExitCode::SUCCESS,
            Err(msg) => {
                eprintln!("vinowatch: {msg}\nWithout a tray host, run `vinowatch --console`.");
                ExitCode::FAILURE
            }
        },
        #[cfg(not(feature = "tray"))]
        Mode::Tray => unreachable!("cli only picks the tray when it is compiled in"),
        Mode::Console { json, once } => {
            console(json, once, interval);
            ExitCode::SUCCESS
        }
    }
}

fn console(json: bool, once: bool, interval: Duration) {
    if !json {
        println!(
            "vinoWatch: monitoring OpenVINO processes every {}s",
            interval.as_secs()
        );
    }
    let mut sampler = Sampler::new();
    // CPU % and NPU % are deltas; this poll only sets the baseline.
    sampler.poll();
    loop {
        sleep(interval);
        let snap = sampler.poll();
        if json {
            println!("{}", snap.to_json());
        } else {
            println!("{}", snap.npu_label());
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
