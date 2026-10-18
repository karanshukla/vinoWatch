//! Command-line arguments.

use std::time::Duration;

pub const USAGE: &str = concat!(
    "vinoWatch ",
    env!("CARGO_PKG_VERSION"),
    "\nShows what the OpenVINO apps on this machine (vinoWhisper, vinoSpeak, vinoPop,\n\
     vinoAuthFace, or anything else that loads libopenvino) cost in CPU, RAM and NPU.\n\n\
     Usage: vinowatch [OPTIONS] [SECONDS]\n\n\
     Options:\n  \
       --console   Print to the terminal instead of showing a tray icon\n  \
       --json      Like --console, one JSON object per poll\n  \
       --once      Poll once and exit (implies --console)\n  \
       -h, --help  Show this help\n  \
       --version   Show the version\n\n\
     SECONDS is the poll interval, 1 to 3600 (default 2)."
);

#[derive(Debug, PartialEq)]
pub enum Mode {
    Tray,
    Console { json: bool, once: bool },
}

#[derive(Debug, PartialEq)]
pub enum Command {
    Help,
    Version,
    Run { mode: Mode, interval: Duration },
}

pub fn parse(args: impl IntoIterator<Item = String>) -> Result<Command, String> {
    let (mut console, mut json, mut once) = (false, false, false);
    let mut interval = None;
    for arg in args {
        match arg.as_str() {
            "-h" | "--help" => return Ok(Command::Help),
            "--version" => return Ok(Command::Version),
            "--console" => console = true,
            "--json" => json = true,
            "--once" => once = true,
            flag if flag.starts_with('-') => return Err(format!("unknown option {flag}")),
            seconds => {
                if interval.is_some() {
                    return Err(format!("unexpected argument {seconds}"));
                }
                match seconds.parse::<u64>() {
                    Ok(s @ 1..=3600) => interval = Some(Duration::from_secs(s)),
                    _ => {
                        return Err(format!(
                            "{seconds} isn't a number of seconds from 1 to 3600"
                        ));
                    }
                }
            }
        }
    }
    let mode = if console || json || once || !cfg!(feature = "tray") {
        Mode::Console { json, once }
    } else {
        Mode::Tray
    };
    Ok(Command::Run {
        mode,
        interval: interval.unwrap_or(Duration::from_secs(2)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(args: &[&str]) -> Result<Command, String> {
        parse(args.iter().map(|s| s.to_string()))
    }

    #[test]
    fn defaults() {
        let Ok(Command::Run { interval, .. }) = p(&[]) else {
            panic!()
        };
        assert_eq!(interval, Duration::from_secs(2));
    }

    #[test]
    fn console_flags() {
        assert_eq!(
            p(&["5", "--json"]),
            Ok(Command::Run {
                mode: Mode::Console {
                    json: true,
                    once: false
                },
                interval: Duration::from_secs(5)
            })
        );
        assert_eq!(
            p(&["--once"]),
            Ok(Command::Run {
                mode: Mode::Console {
                    json: false,
                    once: true
                },
                interval: Duration::from_secs(2)
            })
        );
    }

    #[cfg(feature = "tray")]
    #[test]
    fn tray_is_the_default() {
        assert!(matches!(
            p(&["3"]),
            Ok(Command::Run {
                mode: Mode::Tray,
                ..
            })
        ));
    }

    #[test]
    fn rejects_bad_input() {
        for bad in [
            &["0"][..],
            &["abc"],
            &["-1"],
            &["3601"],
            &["--nope"],
            &["2", "3"],
        ] {
            assert!(p(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn help_and_version_win() {
        // The first bad argument stops parsing before --help is seen.
        assert!(p(&["--nope", "--help"]).is_err());
        assert_eq!(p(&["--help", "--nope"]), Ok(Command::Help));
        assert_eq!(p(&["--version"]), Ok(Command::Version));
    }
}
