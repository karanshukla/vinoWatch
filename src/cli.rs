//! Command-line arguments.

use std::path::PathBuf;
use std::time::Duration;

pub const USAGE: &str = concat!(
    "vinoWatch ",
    env!("CARGO_PKG_VERSION"),
    "\nShows what the OpenVINO apps on this machine (vinoWhisper, vinoSpeak, vinoPop,\n\
     vinoAuthFace, vinoSearch, or anything else that loads libopenvino) cost in CPU, RAM and NPU.\n\n\
     Usage: vinowatch [OPTIONS] [SECONDS]\n\n\
     Options:\n  \
       --console   Print to the terminal instead of showing a tray icon\n  \
       --json      Like --console, one JSON object per poll\n  \
       --once      Poll once and exit (implies --console)\n  \
       --warn-below PERCENT\n              Flag low memory when available RAM falls under PERCENT of\n              the total (0 turns it off, default 15)\n  \
       --install-icons DIR\n              Write the tray and app icons under DIR/icons/hicolor (DIR is\n              usually ~/.local/share) and exit; install.sh runs this\n  \
       -h, --help  Show this help\n  \
       -V, --version\n              Show the version\n\n\
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
    InstallIcons(PathBuf),
    Run {
        mode: Mode,
        interval: Duration,
        warn_below: u8,
    },
}

pub const DEFAULT_WARN_BELOW: u8 = 15;

pub fn parse(args: impl IntoIterator<Item = String>) -> Result<Command, String> {
    let (mut console, mut json, mut once) = (false, false, false);
    let mut interval = None;
    let mut warn_below = DEFAULT_WARN_BELOW;
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => return Ok(Command::Help),
            "-V" | "--version" => return Ok(Command::Version),
            "--install-icons" => {
                let dir = args.next().ok_or("--install-icons needs a directory")?;
                if !cfg!(feature = "tray") {
                    return Err("this build has no tray, so it has no icons".into());
                }
                return Ok(Command::InstallIcons(dir.into()));
            }
            "--console" => console = true,
            "--json" => json = true,
            "--once" => once = true,
            "--warn-below" => {
                let value = args.next().ok_or("--warn-below needs a percentage")?;
                warn_below = match value.parse::<u8>() {
                    Ok(p @ 0..=100) => p,
                    _ => return Err(format!("{value} isn't a percentage from 0 to 100")),
                };
            }
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
        warn_below,
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
                interval: Duration::from_secs(5),
                warn_below: 15,
            })
        );
        assert_eq!(
            p(&["--once"]),
            Ok(Command::Run {
                mode: Mode::Console {
                    json: false,
                    once: true
                },
                interval: Duration::from_secs(2),
                warn_below: 15,
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
    fn warn_below() {
        let Ok(Command::Run { warn_below, .. }) = p(&["--warn-below", "25"]) else {
            panic!()
        };
        assert_eq!(warn_below, 25);
        let Ok(Command::Run { warn_below, .. }) = p(&["--warn-below", "0", "3"]) else {
            panic!()
        };
        assert_eq!(warn_below, 0);
        for bad in [
            &["--warn-below"][..],
            &["--warn-below", "101"],
            &["--warn-below", "x"],
        ] {
            assert!(p(bad).is_err(), "{bad:?}");
        }
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
        assert_eq!(p(&["-V"]), Ok(Command::Version));
    }

    #[test]
    fn install_icons_takes_a_directory() {
        assert!(p(&["--install-icons"]).is_err());
        let got = p(&["--install-icons", "/tmp/share"]);
        if cfg!(feature = "tray") {
            assert_eq!(got, Ok(Command::InstallIcons("/tmp/share".into())));
        } else {
            assert!(got.is_err());
        }
    }
}
