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

/// The process name of whoever owns the tray watcher now, with its unique bus
/// name so a change of host is noticed.
pub(super) async fn watcher_host(
    dbus: &zbus::fdo::DBusProxy<'_>,
) -> Option<(String, Option<String>)> {
    let name = zbus::names::BusName::try_from("org.kde.StatusNotifierWatcher").ok()?;
    let owner = dbus.get_name_owner(name).await.ok()?;
    let pid = dbus
        .get_connection_unix_process_id(owner.as_ref().into())
        .await
        .ok();
    let comm = pid.and_then(|pid| std::fs::read_to_string(format!("/proc/{pid}/comm")).ok());
    Some((owner.to_string(), comm))
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

    #[test]
    fn menu_labels_keep_their_underscores() {
        assert_eq!(menu_text("vino_speak pid 1"), "vino__speak pid 1");
        assert_eq!(menu_text("no underscores"), "no underscores");
    }
}
