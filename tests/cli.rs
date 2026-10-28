//! The binary's contract with scripts: exit codes and what it prints.
//!
//! The unit tests cover the logic behind each flag; these run the built
//! binary, so `main.rs` (dispatch, exit codes, the console loop, the
//! single-instance check) is covered too.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::{Command, Output};

fn vinowatch(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_vinowatch"))
        .args(args)
        .output()
        .expect("run vinowatch")
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

/// A scratch directory per test, so the tests can run in parallel.
fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("vinowatch-cli-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn version_and_help() {
    let out = vinowatch(&["--version"]);
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(
        stdout(&out),
        format!("vinowatch {}\n", env!("CARGO_PKG_VERSION"))
    );
    let out = vinowatch(&["-h"]);
    assert_eq!(out.status.code(), Some(0));
    assert!(stdout(&out).contains("Usage: vinowatch [OPTIONS] [SECONDS]"));
}

#[test]
fn a_bad_argument_exits_2_with_the_usage() {
    for args in [
        &["--nope"][..],
        &["0"],
        &["--warn-below", "101"],
        &["1", "2"],
    ] {
        let out = vinowatch(args);
        assert_eq!(out.status.code(), Some(2), "{args:?}");
        assert!(stdout(&out).is_empty(), "{args:?}");
        let err = stderr(&out);
        assert!(err.starts_with("vinowatch: "), "{args:?}: {err}");
        assert!(err.contains("Usage: vinowatch"), "{args:?}: {err}");
    }
}

/// `--once --json` is the scripting interface: one JSON object, one line.
#[test]
fn once_json_prints_one_object() {
    let out = vinowatch(&["--once", "--json", "1"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let text = stdout(&out);
    assert_eq!(text.lines().count(), 1, "{text}");
    assert!(text.starts_with("{\"npu_percent\":"), "{text}");
    assert!(text.contains("\"processes\":["), "{text}");
    assert!(text.ends_with("}\n"), "{text}");
}

#[test]
fn once_prints_the_console_report() {
    let out = vinowatch(&["--once", "1"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let text = stdout(&out);
    assert!(
        text.starts_with("vinoWatch: monitoring OpenVINO processes every 1s\n"),
        "{text}"
    );
    assert!(text.contains("OpenVINO apps hold "), "{text}");
}

/// `--check-npu` documents 0 OK, 1 stuck, 3 couldn't tell. The probe's
/// verdict only depends on how the interpreter exits, so stand-ins do.
#[test]
fn check_npu_exit_codes() {
    let out = vinowatch(&["--check-npu", "--probe-python", "/bin/true"]);
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(stdout(&out), "NPU driver OK\n");

    let out = vinowatch(&["--check-npu", "--probe-python", "/bin/false"]);
    assert_eq!(out.status.code(), Some(3));
    assert!(stdout(&out).starts_with("Couldn't tell (/bin/false): "));

    let dir = scratch("stuck");
    let segv = dir.join("python3");
    fs::write(&segv, "#!/bin/sh\nkill -SEGV $$\n").unwrap();
    fs::set_permissions(&segv, fs::Permissions::from_mode(0o755)).unwrap();
    let out = vinowatch(&["--check-npu", "--probe-python", segv.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(1));
    assert!(stdout(&out).starts_with("NPU driver stuck: "));
    fs::remove_dir_all(&dir).unwrap();
}

#[cfg(feature = "tray")]
#[test]
fn install_icons_writes_the_theme_files() {
    let dir = scratch("icons");
    let out = vinowatch(&["--install-icons", dir.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let listed: Vec<&str> = std::str::from_utf8(&out.stdout).unwrap().lines().collect();
    assert_eq!(listed.len(), 4, "{listed:?}");
    for path in listed {
        assert!(path.starts_with(dir.join("icons/hicolor").to_str().unwrap()));
        assert!(fs::read_to_string(path).unwrap().starts_with("<svg"));
    }
    fs::remove_dir_all(&dir).unwrap();
}

/// A second tray exits 0, so the unit doesn't restart it in a loop.
#[cfg(feature = "tray")]
#[test]
fn a_second_tray_exits_cleanly() {
    let dir = scratch("lock");
    let lock = fs::File::create(dir.join("vinowatch.lock")).unwrap();
    lock.try_lock().unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_vinowatch"))
        .env("XDG_RUNTIME_DIR", &dir)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(stderr(&out), "vinowatch: the tray is already running\n");
    drop(lock);
    fs::remove_dir_all(&dir).unwrap();
}

#[cfg(not(feature = "tray"))]
#[test]
fn a_console_build_refuses_install_icons() {
    let out = vinowatch(&["--install-icons", "/nonexistent"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(stderr(&out).contains("this build has no tray"));
}
