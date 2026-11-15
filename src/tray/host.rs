//! What differs between tray hosts: who recolours a symbolic icon, and how
//! tooltip and menu text must be escaped. docs/desktops.md has the table.

/// The spec lets a tooltip's description carry markup, and XFCE, Cinnamon,
/// MATE, Budgie and Plasma parse it as such: a `<` in a driver error or a
/// process name would otherwise cut the tooltip short.
pub(super) fn markup_escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Tray hosts that draw a themed `-symbolic` icon as the SVG has it instead of
/// in the panel's text colour, by the watcher's process name (`comm`, cut to
/// 15 characters). The glyph is near-black, and these panels are usually dark.
const NO_RECOLOUR_HOSTS: &[&str] = &[
    "lxqt-panel",
    "mate-panel",
    // mate-panel's notification area when it runs out of process.
    "notification-ar",
    "swaybar",
];

/// Whether the tray host recolours `-symbolic` icons. `watcher` is the process
/// name owning `org.kde.StatusNotifierWatcher`, `desktop` is
/// `XDG_CURRENT_DESKTOP`; the desktop covers a watcher we couldn't name.
pub(super) fn host_recolours(watcher: Option<&str>, desktop: Option<&str>) -> bool {
    if watcher.is_some_and(|name| NO_RECOLOUR_HOSTS.contains(&name.trim())) {
        return false;
    }
    let desktop = desktop.unwrap_or_default();
    watcher.is_some() || !desktop.split(':').any(|d| d == "LXQt" || d == "MATE")
}

/// Who owns `org.kde.StatusNotifierWatcher`: its unique bus name, and its
/// process name where that could be read.
pub(super) struct Watcher {
    pub(super) owner: String,
    pub(super) comm: Option<String>,
}

/// Remembers the tray host the icon names were last chosen for. A host can be
/// replaced at any time, so the loop asks every poll.
#[derive(Default)]
pub(super) struct HostTracker {
    owner: Option<String>,
}

impl HostTracker {
    /// `Some(recolours)` when the watcher's owner is not who it was at the
    /// last call, appearing and going away included; `None` while it stays.
    pub(super) fn observe(&mut self, now: Option<Watcher>, desktop: Option<&str>) -> Option<bool> {
        let owner = now.as_ref().map(|watcher| watcher.owner.as_str());
        if owner == self.owner.as_deref() {
            return None;
        }
        let comm = now.as_ref().and_then(|watcher| watcher.comm.as_deref());
        let recolours = host_recolours(comm, desktop);
        self.owner = now.map(|watcher| watcher.owner);
        Some(recolours)
    }
}

/// Asks the bus who owns the tray watcher now.
pub(super) async fn watcher_host(dbus: &zbus::fdo::DBusProxy<'_>) -> Option<Watcher> {
    let name = zbus::names::BusName::try_from("org.kde.StatusNotifierWatcher").ok()?;
    let owner = dbus.get_name_owner(name).await.ok()?;
    let pid = dbus
        .get_connection_unix_process_id(owner.as_ref().into())
        .await
        .ok();
    let comm = pid.and_then(|pid| std::fs::read_to_string(format!("/proc/{pid}/comm")).ok());
    Some(Watcher {
        owner: owner.to_string(),
        comm,
    })
}

/// ksni treats "_" as an access-key marker and hides it; a doubled one is a
/// literal underscore. Process names like `vino_speak` have them.
pub(super) fn menu_text(label: &str) -> String {
    label.replace('_', "__")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tooltip_text_is_escaped() {
        assert_eq!(markup_escape("a <b> & c"), "a &lt;b&gt; &amp; c");
    }

    #[test]
    fn hosts_that_dont_recolour_are_told_apart_by_the_watchers_name() {
        assert!(host_recolours(Some("waybar\n"), None));
        assert!(host_recolours(Some("plasmashell"), Some("KDE")));
        assert!(!host_recolours(Some("swaybar\n"), Some("sway")));
        assert!(!host_recolours(Some("lxqt-panel"), None));
        assert!(!host_recolours(Some("notification-ar"), None));
    }

    #[test]
    fn an_unnamed_watcher_goes_by_the_desktop() {
        assert!(!host_recolours(None, Some("X-Ubuntu:MATE")));
        assert!(host_recolours(None, Some("XFCE")));
        assert!(host_recolours(None, None));
    }

    fn watcher(owner: &str, comm: &str) -> Option<Watcher> {
        Some(Watcher {
            owner: owner.into(),
            comm: Some(comm.into()),
        })
    }

    #[test]
    fn the_icon_names_are_chosen_again_only_when_the_host_changes() {
        let mut tracker = HostTracker::default();
        assert_eq!(
            tracker.observe(watcher(":1.7", "waybar\n"), None),
            Some(true)
        );
        assert_eq!(tracker.observe(watcher(":1.7", "waybar\n"), None), None);
        // Another process took the name over.
        assert_eq!(
            tracker.observe(watcher(":1.9", "swaybar\n"), None),
            Some(false)
        );
        assert_eq!(tracker.observe(watcher(":1.9", "swaybar\n"), None), None);
    }

    #[test]
    fn no_host_yet_is_not_a_change() {
        let mut tracker = HostTracker::default();
        assert_eq!(tracker.observe(None, Some("MATE")), None);
    }

    #[test]
    fn a_host_going_away_falls_back_to_the_desktop() {
        let mut tracker = HostTracker::default();
        tracker.observe(watcher(":1.7", "waybar\n"), Some("MATE"));
        assert_eq!(tracker.observe(None, Some("MATE")), Some(false));
        assert_eq!(tracker.observe(None, Some("MATE")), None);
        assert_eq!(
            tracker.observe(watcher(":1.8", "mate-panel\n"), Some("MATE")),
            Some(false)
        );
    }

    #[test]
    fn menu_labels_keep_their_underscores() {
        assert_eq!(menu_text("vino_speak pid 1"), "vino__speak pid 1");
        assert_eq!(menu_text("no underscores"), "no underscores");
    }
}
