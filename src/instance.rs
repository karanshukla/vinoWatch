//! One tray per session: the systemd unit and a hand-started `vinowatch`
//! would otherwise both put an icon in the panel.

use std::fs::{File, OpenOptions, TryLockError};
use std::io;
use std::path::{Path, PathBuf};

/// Held for the life of the process; the kernel drops the lock when it dies.
pub struct Instance(#[allow(dead_code)] File);

pub enum Claim {
    Held(#[allow(dead_code)] Instance),
    /// Exit 0, or the unit restarts it (pinned by `tests/cli.rs::a_second_tray_exits_cleanly`).
    Taken,
    /// No runtime dir, or the lock failed.
    Unguarded,
}

pub fn claim() -> Claim {
    let Some(path) = lock_path() else {
        return Claim::Unguarded;
    };
    match acquire(&path) {
        Ok(Some(held)) => Claim::Held(held),
        Ok(None) => Claim::Taken,
        Err(e) => {
            eprintln!("vinowatch: couldn't take the single-instance lock: {e}");
            Claim::Unguarded
        }
    }
}

/// `$XDG_RUNTIME_DIR/vinowatch.lock`, or `None` outside a session with a
/// runtime dir (then there is no guard).
pub fn lock_path() -> Option<PathBuf> {
    let dir = std::env::var_os("XDG_RUNTIME_DIR").filter(|d| !d.is_empty())?;
    Some(Path::new(&dir).join("vinowatch.lock"))
}

/// `Ok(None)` when another tray holds the lock.
pub fn acquire(path: &Path) -> io::Result<Option<Instance>> {
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(path)?;
    match file.try_lock() {
        Ok(()) => Ok(Some(Instance(file))),
        Err(TryLockError::WouldBlock) => Ok(None),
        Err(TryLockError::Error(e)) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_second_tray_is_refused_while_the_first_runs() {
        let scratch = tempfile::tempdir().unwrap();
        let path = scratch.path().join("vinowatch.lock");
        let first = acquire(&path).unwrap();
        assert!(first.is_some());
        assert!(acquire(&path).unwrap().is_none());
        drop(first);
        assert!(acquire(&path).unwrap().is_some(), "not released on drop");
    }
}
