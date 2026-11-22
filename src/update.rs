//! "Check for updates" and "Update to vN" from the tray menu. The unit allows
//! Unix sockets only, so both run outside it through `systemd-run --user`: one
//! `curl` for the check, and `vinowatch-upgrade` in a unit of its own, which
//! survives the restart of `vinowatch.service` it ends with.

use std::path::PathBuf;
use std::process::{Command, Output, Stdio};

use vino_ui::App;
use vino_ui::update::{is_newer, latest_tag};

/// The transient unit the update runs in, for `journalctl --user -u`.
pub const UNIT: &str = "vinowatch-update";

/// What a check found.
#[derive(Clone, Debug, PartialEq)]
pub enum Found {
    UpToDate,
    Newer(String),
}

/// The installer install.sh copies next to the binary.
pub fn upgrade_command() -> Option<PathBuf> {
    let path = PathBuf::from(std::env::var_os("HOME")?).join(".local/bin/vinowatch-upgrade");
    path.is_file().then_some(path)
}

/// Asks GitHub for the newest release. Blocking: run it off the tray's thread.
pub fn check(current: &str) -> Result<Found, String> {
    let url = format!(
        "https://api.github.com/repos/{}/releases/latest",
        App::Watch.repo()
    );
    let curl = fetch_command(&url);
    let output = outside_the_unit(&curl)
        .or_else(|| directly(&curl))
        .ok_or("curl isn't installed")?;
    if !output.status.success() {
        return Err("couldn't reach GitHub".into());
    }
    found_in(current, &String::from_utf8_lossy(&output.stdout))
}

/// Through the user manager, outside the unit's sandbox; `None` with no systemd.
fn outside_the_unit(curl: &[&str]) -> Option<Output> {
    run(Command::new("systemd-run")
        .args(["--user", "--quiet", "--collect", "--pipe", "--wait", "--"])
        .args(curl))
}

fn directly(curl: &[&str]) -> Option<Output> {
    run(Command::new(curl[0]).args(&curl[1..]))
}

/// `curl` for one small JSON answer: it gives up after 10 s or 1 MiB.
fn fetch_command(url: &str) -> [&str; 9] {
    [
        "curl",
        "-fsSL",
        "--max-time",
        "10",
        "--max-filesize",
        "1048576",
        "-H",
        "Accept: application/vnd.github+json",
        url,
    ]
}

/// What GitHub's `releases/latest` answer means for a tray at `current`.
fn found_in(current: &str, release_json: &str) -> Result<Found, String> {
    let latest = latest_tag(release_json).ok_or("no release found on GitHub")?;
    Ok(if is_newer(current, &latest) {
        Found::Newer(latest)
    } else {
        Found::UpToDate
    })
}

/// Installs `tag` with `upgrade` in a unit of its own. On success the installer
/// restarts this tray, so this only returns to say it failed.
pub fn install(upgrade: &std::path::Path, tag: &str) -> Result<(), String> {
    let status = Command::new("systemd-run")
        .args(["--user", "--quiet", "--collect", "--wait", "--unit", UNIT])
        .arg(format!("--setenv=VINOWATCH_VERSION={tag}"))
        .arg("--")
        .arg(upgrade)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|_| {
            "systemd-run isn't available; run vinowatch-upgrade in a terminal".to_string()
        })?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("see journalctl --user -u {UNIT}"))
    }
}

fn run(command: &mut Command) -> Option<Output> {
    command
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_dev_build_is_never_offered_an_update() {
        assert_eq!(
            found_in("dev", r#"{"tag_name":"v9.9.9"}"#),
            Ok(Found::UpToDate)
        );
    }

    #[test]
    fn a_later_release_is_offered_by_its_tag() {
        let answer = r#"{"url":"https://api.github.com/x","tag_name":"v9.9.9","name":"9.9.9"}"#;
        assert_eq!(found_in("0.6.0", answer), Ok(Found::Newer("v9.9.9".into())));
    }

    #[test]
    fn the_running_release_and_older_ones_are_up_to_date() {
        assert_eq!(
            found_in("0.6.0", r#"{"tag_name":"v0.6.0"}"#),
            Ok(Found::UpToDate)
        );
        assert_eq!(
            found_in("0.6.0", r#"{"tag_name":"v0.5.9"}"#),
            Ok(Found::UpToDate)
        );
    }

    #[test]
    fn an_answer_with_no_release_is_an_error_not_up_to_date() {
        for answer in ["", "{}", r#"{"message":"Not Found"}"#] {
            assert_eq!(
                found_in("0.6.0", answer),
                Err("no release found on GitHub".into()),
                "{answer:?}"
            );
        }
    }

    #[test]
    fn the_check_asks_only_the_url_it_is_given_and_bounds_the_answer() {
        let curl = fetch_command("https://api.github.com/repos/o/r/releases/latest");
        assert_eq!(curl[0], "curl");
        assert_eq!(curl[8], "https://api.github.com/repos/o/r/releases/latest");
        assert!(curl.windows(2).any(|w| w == ["--max-time", "10"]));
        assert!(curl.windows(2).any(|w| w == ["--max-filesize", "1048576"]));
    }
}
