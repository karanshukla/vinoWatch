//! One tray per session: the systemd unit and a hand-started `vinowatch`
//! would otherwise both put an icon in the panel.

use std::fs::{File, OpenOptions, TryLockError};
use std::io;
use std::path::{Path, PathBuf};

/// Held for the life of the process; the kernel drops the lock when it dies.
pub struct Instance(#[allow(dead_code)] File);

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

    fn lock_file(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("vinowatch-{name}-{}.lock", std::process::id()))
    }

    #[test]
    fn a_second_tray_is_refused_while_the_first_runs() {
        let path = lock_file("second");
        let first = acquire(&path).unwrap();
        assert!(first.is_some());
        assert!(acquire(&path).unwrap().is_none());
        drop(first);
        assert!(acquire(&path).unwrap().is_some(), "not released on drop");
        let _ = std::fs::remove_file(path);
    }
}
