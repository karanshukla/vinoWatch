//! Where each of the menu's actions is: the driver check and reset, and the
//! update check and install.

use vino_ui::tray::MenuIcon;

use crate::driver::Verdict;

/// The menu's actions that run off the tray's thread: the driver probe takes
/// up to a minute, the reload waits for `modprobe`, and the update ones go to
/// the network.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Job {
    Driver(DriverJob),
    Update(UpdateJob),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum DriverJob {
    Check,
    Reset,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum UpdateJob {
    Check,
    /// The release `Update::Available` names.
    Install,
}

/// Where "Check for updates" is. Nothing goes online until it is clicked.
#[derive(Clone, Debug, Default, PartialEq)]
pub(super) enum Update {
    #[default]
    Unchecked,
    Checking,
    UpToDate,
    Available(String),
    Installing(String),
    Failed(String),
}

impl Update {
    pub(super) fn busy(&self) -> bool {
        matches!(self, Update::Checking | Update::Installing(_))
    }

    pub(super) fn label(&self) -> String {
        match self {
            Update::Unchecked => "Check for updates".into(),
            Update::Checking => "Checking for updates…".into(),
            Update::UpToDate => "Up to date".into(),
            Update::Available(tag) => format!("Update to {tag}"),
            Update::Installing(tag) => format!("Installing {tag}…"),
            Update::Failed(why) => format!("Update failed: {} — Retry", clip(why)),
        }
    }
}

/// Where the stuck-driver check is. Nothing runs until it is asked for.
#[derive(Clone, Debug, Default, PartialEq)]
pub(super) enum Driver {
    #[default]
    Unchecked,
    Checking,
    Resetting,
    Checked {
        verdict: Verdict,
        after_reset: bool,
    },
    ResetFailed(String),
}

impl Driver {
    pub(super) fn busy(&self) -> bool {
        matches!(self, Driver::Checking | Driver::Resetting)
    }

    pub(super) fn stuck(&self) -> bool {
        matches!(
            self,
            Driver::Checked {
                verdict: Verdict::Stuck,
                ..
            }
        )
    }

    pub(super) fn icon_name(&self) -> &'static str {
        match self {
            Driver::Unchecked | Driver::Checking | Driver::Resetting => MenuIcon::Refresh.name(),
            Driver::Checked {
                verdict: Verdict::Healthy,
                ..
            } => MenuIcon::Ok.name(),
            Driver::Checked {
                verdict: Verdict::Stuck,
                ..
            } => MenuIcon::Warning.name(),
            Driver::Checked {
                verdict: Verdict::Unknown(_),
                ..
            } => MenuIcon::Question.name(),
            Driver::ResetFailed(_) => MenuIcon::Error.name(),
        }
    }

    pub(super) fn label(&self) -> Option<String> {
        Some(match self {
            Driver::Unchecked => return None,
            Driver::Checking => "Checking NPU driver…".into(),
            Driver::Resetting => "Reloading NPU driver…".into(),
            Driver::Checked {
                verdict: Verdict::Healthy,
                after_reset,
            } => if *after_reset {
                "NPU driver reloaded and OK"
            } else {
                "NPU driver OK"
            }
            .into(),
            Driver::Checked {
                verdict: Verdict::Stuck,
                after_reset,
            } => if *after_reset {
                "NPU driver still stuck — reboot needed"
            } else {
                "NPU driver stuck"
            }
            .into(),
            Driver::Checked {
                verdict: Verdict::Unknown(why),
                after_reset,
            } => {
                if *after_reset {
                    format!("Driver reloaded — check failed: {}", clip(why))
                } else {
                    format!("NPU check failed: {}", clip(why))
                }
            }
            Driver::ResetFailed(why) => format!("NPU reset failed: {}", clip(why)),
        })
    }
}

/// A Python error can name a long library path; a menu row shouldn't.
fn clip(text: &str) -> String {
    const MAX: usize = 100;
    match text.char_indices().nth(MAX) {
        Some((at, _)) => format!("{}…", &text[..at]),
        None => text.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_errors_are_clipped() {
        let driver = Driver::Checked {
            verdict: Verdict::Unknown("x".repeat(500)),
            after_reset: true,
        };
        let label = driver.label().unwrap();
        assert!(label.starts_with("Driver reloaded — check failed: xxx"));
        assert!(label.ends_with("x…"));
        assert!(label.chars().count() < 200);
    }

    #[test]
    fn a_short_error_is_shown_whole() {
        assert_eq!(clip("modprobe failed"), "modprobe failed");
        assert_eq!(
            Driver::ResetFailed("modprobe failed".into()).label(),
            Some("NPU reset failed: modprobe failed".into())
        );
    }
}
