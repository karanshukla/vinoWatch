//! "Check for updates" and "Update to vN" from the tray menu.
//!
//! The tray's unit allows Unix sockets only (`RestrictAddressFamilies=AF_UNIX`),
//! so neither step runs inside it: both go to the user manager through
//! `systemd-run --user`, which is a Unix socket away. The check is one
//! `curl` to GitHub's releases API; the update is `vinowatch-upgrade`, the
//! release installer install.sh leaves behind, in a unit of its own, so it
//! survives the restart of `vinowatch.service` it ends with.

use std::path::PathBuf;
use std::process::{Command, Stdio};

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
    let curl = [
        "curl",
        "-fsSL",
        "--max-time",
        "10",
        "--max-filesize",
        "1048576",
        "-H",
        "Accept: application/vnd.github+json",
        url.as_str(),
    ];
    // Outside the unit's sandbox when there is a user manager to ask; run
    // directly otherwise (a tray started by hand, or no systemd).
    let mut outside = Command::new("systemd-run");
    outside
        .args(["--user", "--quiet", "--collect", "--pipe", "--wait", "--"])
        .args(curl);
    let output = match run(&mut outside) {
        Some(output) => output,
        None => run(Command::new(curl[0]).args(&curl[1..])).ok_or("curl isn't installed")?,
    };
    if !output.status.success() {
        return Err("couldn't reach GitHub".into());
    }
    let latest =
        latest_tag(&String::from_utf8_lossy(&output.stdout)).ok_or("GitHub sent no release")?;
    Ok(if is_newer(current, &latest) {
        Found::Newer(latest)
    } else {
        Found::UpToDate
    })
}

/// Installs `tag` with `upgrade` in a unit of its own. Blocking until it
/// finishes; on success the installer restarts this tray, so this only
/// returns to say it failed.
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

fn run(command: &mut Command) -> Option<std::process::Output> {
    command
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()
}
