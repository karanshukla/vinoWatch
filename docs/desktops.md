# Desktops

The tray is a StatusNotifierItem, so it needs a host for those.

| Desktop | Tray host | Left click | Hides when idle |
|---|---|---|---|
| KDE Plasma | built in | menu | yes |
| GNOME | the [AppIndicator extension](https://extensions.gnome.org/extension/615/appindicator-support/) (Ubuntu ships it on); no tooltip | menu, after the double-click delay | yes |
| COSMIC | built in; no tooltip | menu | no |
| XFCE | the Status Tray plugin (in the default panel) | menu | yes |
| Cinnamon | built in (`xapp-sn-watcher`) | nothing; right click for the menu | yes |
| MATE | the Notification Area applet | nothing; right click for the menu | yes |
| LXQt | the Status Notifier plugin; tooltip shows the title only | nothing; right click for the menu | no |
| Budgie | built in | menu | no |
| sway, Hyprland | Waybar's `tray` module | menu | yes |

These come from reading each host's source, not from running every desktop; Plasma and sway with Waybar are the ones tested.

Where a host draws themed symbolic icons without recolouring them (LXQt, MATE, swaybar), vinoWatch sends its own light icons instead, as those panels are usually dark.

## Starting with the session

The unit starts with `graphical-session.target`. For sessions that never start that target, install.sh also writes an XDG autostart entry (`~/.config/autostart/vinowatch.desktop`) that starts the unit.

If vinoWatch starts before the panel does, it waits for a tray host instead of exiting.

## App launcher

install.sh adds vinoWatch to the app launcher too (`~/.local/share/applications/vinowatch.desktop`). Picking it there restarts the tray: that starts it if it was stopped, and brings an icon that hid while idle back into view for five minutes.

## sway and Hyprland

Both only start `graphical-session.target` through a session wrapper: [uwsm](https://github.com/Vladimir-csp/uwsm), or Fedora's `sway-systemd`. Without one, start the unit from the compositor's config:

```
exec systemctl --user start vinowatch.service             # sway
exec-once = systemctl --user start vinowatch.service      # Hyprland
```

`install.sh` prints the right line when it's run in such a session. If you start the compositor with `dbus-run-session`, its session bus isn't the one systemd's user units see; run `exec vinowatch` instead.

swaybar's own tray shows the icon but has no menus or tooltips, so use Waybar.

## X11 panels

Panels that only know the older XEmbed tray (i3bar, tint2, polybar) need [snixembed](https://git.sr.ht/~steef/snixembed) in between; there, right click opens the menu, and the icon neither hides nor turns amber.

## Notifications

The driver check's notification needs a notification daemon (mako, dunst, swaync on wlroots); without one the result still shows in the menu and tooltip.
