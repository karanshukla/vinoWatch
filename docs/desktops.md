# Desktops

`vinoWatch` uses the StatusNotifierItem protocol, which requires a compatible tray host on your desktop environment or panel.

## Tray host support

| Desktop | Tray host | Left click | Hides when idle |
|---|---|---|---|
| KDE Plasma | Built in | Opens menu | Yes |
| GNOME | [AppIndicator extension](https://extensions.gnome.org/extension/615/appindicator-support/) (enabled by default on Ubuntu); no tooltip | Opens menu (after double-click delay) | Yes |
| COSMIC | Built in; no tooltip | Opens menu | No |
| XFCE | Status Tray plugin (in the default panel) | Opens menu | Yes |
| Cinnamon | Built in (`xapp-sn-watcher`) | No action; right click for menu | Yes |
| MATE | Notification Area applet | No action; right click for menu | Yes |
| LXQt | Status Notifier plugin; tooltip shows title only | No action; right click for menu | No |
| Budgie | Built in | Opens menu | No |
| sway, Hyprland | Waybar's `tray` module | Opens menu | Yes |

This matrix is compiled from reading each tray host's source code; KDE Plasma and sway with Waybar have been tested directly.

On tray hosts that render themed symbolic icons without recoloring them (LXQt, MATE, and swaybar), `vinoWatch` sends its own light-colored icons because those panels typically use dark backgrounds.

## Starting with your session

The systemd user unit starts with `graphical-session.target`. For desktop sessions that do not activate `graphical-session.target`, `install.sh` also installs an XDG autostart entry (`~/.config/autostart/vinowatch.desktop`) that starts the unit.

If `vinoWatch` launches before your panel is ready, it waits for a tray host to appear rather than exiting.

## App launcher

`install.sh` installs an application launcher entry at `~/.local/share/applications/vinowatch.desktop`. Launching `vinoWatch` from your app menu restarts the tray service, starting it if it was stopped or making an idle-hidden icon visible again for five minutes.

## sway and Hyprland

Both compositors only activate `graphical-session.target` when launched through a session wrapper such as [uwsm](https://github.com/Vladimir-csp/uwsm) or Fedora's `sway-systemd`. Without a session wrapper, start the service from your compositor configuration:

```text
exec systemctl --user start vinowatch.service             # sway
exec-once = systemctl --user start vinowatch.service      # Hyprland
```

`install.sh` prints the matching line automatically when run inside a sway or Hyprland session. If you start your compositor via `dbus-run-session`, its D-Bus session bus differs from the one visible to systemd user units; in that case, run `exec vinowatch` directly instead.

Note that `swaybar`'s built-in tray displays the icon but does not support menus or tooltips, so Waybar is recommended on sway.

## X11 panels

Panels that only implement the legacy XEmbed tray protocol (`i3bar`, `tint2`, `polybar`) require [snixembed](https://git.sr.ht/~steef/snixembed) as a bridge. Through `snixembed`, right-click opens the menu, and the icon neither hides when idle nor turns amber.

## Notifications

Desktop notifications from **Check NPU driver** require a notification daemon (such as `mako`, `dunst`, or `swaync` on wlroots compositors). Without one, the check result still appears in the tray menu and tooltip.
