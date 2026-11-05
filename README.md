# vinoWatch

A small tray daemon that shows what the OpenVINO apps on your machine cost: CPU, RAM and, on Intel machines with the `ivpu` driver, NPU load. It knows the other vino projects by name:

| App | Seen as |
|---|---|
| [vinoWhisper](https://github.com/karanshukla/vinoWhisper) | `vinowhisper-server`, `-caption`, `-dictate`, `-replay` |
| [vinoSpeak](https://github.com/karanshukla/vinoSpeak) | `vinospeak` |
| [vinoPop](https://github.com/karanshukla/vinoPop) | its `vinopop` launcher, or a `main.py` in its checkout |
| [vinoAuthFace](https://github.com/karanshukla/vinoAuthFace) | `vinoauthface`, `vinoauthface-auth` |
| [vinoSearch](https://github.com/karanshukla/vinoSearch) | `vinosearch` |

Anything else that loads `libopenvino*` is listed too, just without an app name.

Hover the tray icon for the NPU load, a process count and how much RAM the OpenVINO apps hold against what the machine has free; click it for one row per process. Each row has its own little meter icon: the NPU row fills with the NPU load, the RAM row with how full RAM is, and a process row shows that process's CPU (full is one core), RAM (full is a fifth of the machine's) and whether it holds the NPU. A row is white while that thing idles, green while it works, and amber for the process holding a fifth of RAM or more, or the biggest one while RAM is low. The icon is three meters in the vino apps' style: green while a process that loads OpenVINO runs or the NPU is busy (a vino app's helper alone, like its tray, doesn't count), and amber if available RAM drops under 15% of the total (`--warn-below PERCENT` changes that, `0` turns it off). Once the NPU has been idle for 5 minutes it goes passive, as vinoWhisper's does, so Plasma tucks it under the expander and Waybar hides it; NPU load brings it back. Apps left running with the NPU idle don't keep it in view, a stuck driver does. It only changes the icon and tooltip: no popups (the one notification is the answer to a driver check you asked for), and nothing is ever stopped for you. The tray needs a StatusNotifierItem host; see [Desktops](#desktops).

## Stuck NPU driver

After heavy NPU use the `intel_vpu` driver can get stuck: any process with two OpenVINO Cores touching the NPU segfaults in `libze_loader`, so vinoPop crashes on every AI action and vinoAuthFace on every scan. Reloading the driver clears it without a reboot.

With an NPU present the tray menu has **Check NPU driver**. It runs a ~0.3 s probe (vinoPop's: a toy model on the NPU, a second Core, the model again; a segfault means stuck) on demand, never on a timer, and shows the result as a desktop notification as well as in the menu and tooltip. A stuck driver turns the icon amber and keeps it in view. The probe needs a Python that can import openvino: `--probe-python PATH`, else the venv interpreter of a vino app vinoWatch has seen running, else `python3`. `vinowatch --check-npu` runs the same probe from a terminal (exit 0 OK, 1 stuck, 3 couldn't tell).

**Reset NPU driver** reloads it (`modprobe -r intel_vpu && modprobe intel_vpu`) through a root oneshot unit, and checks again afterwards. That needs a one-time root step, which `install.sh` doesn't do:

```sh
sudo ~/.local/share/vinowatch/install-npu-reset.sh
sudo ~/.local/share/vinowatch/install-npu-reset.sh --uninstall
```

`install.sh` keeps that script there, with the unit and rule it installs, from the same checked release (or checkout). `sudo ./install-npu-reset.sh` from a checkout or an unpacked release does the same.

It installs `vino-npu-reset.service` and a polkit rule that lets the active local user start that unit, and nothing else, without a password. `modprobe -r` refuses while anything has `/dev/accel` open, so the item lists the apps holding the NPU and stays disabled until you close them. Nothing is stopped for you. Holders that belong to other users aren't visible; then the reset fails and says so.

## Install

```sh
curl -fsSL https://github.com/karanshukla/vinoWatch/releases/latest/download/install.sh | sh
```

This downloads the latest release, checks it against `SHA256SUMS`, puts `vinowatch` in `~/.local/bin`, its icons in `~/.local/share/icons/hicolor` and the NPU reset installer in `~/.local/share/vinowatch`, and enables a systemd user unit that starts the tray with your graphical session. From a checkout, `./install.sh` builds from source instead. `./install.sh --uninstall` removes all of it; run `install-npu-reset.sh --uninstall` first if you installed the reset unit.

To upgrade, use **Check for updates** in the tray menu, under the version: it asks GitHub only when you click it, then offers **Update to vN**, which installs that release the same way and restarts the tray. `VINOWATCH_VERSION=vN vinowatch-upgrade` does the same from a terminal.

## Desktops

The tray is a StatusNotifierItem. The unit starts with `graphical-session.target`, and for sessions that never start that target, install.sh also writes an XDG autostart entry (`~/.config/autostart/vinowatch.desktop`) that starts the unit.

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

These come from reading each host's source, not from running every desktop; Plasma and sway with Waybar are the ones tested. Where a host draws themed symbolic icons without recolouring them (LXQt, MATE, swaybar), vinoWatch sends its own light icons instead, as those panels are usually dark.

sway and Hyprland only start `graphical-session.target` through a session wrapper: [uwsm](https://github.com/Vladimir-csp/uwsm), or Fedora's `sway-systemd`. Without one, start the unit from the compositor's config:

```
exec systemctl --user start vinowatch.service             # sway
exec-once = systemctl --user start vinowatch.service      # Hyprland
```

`install.sh` prints the right line when it's run in such a session. If you start the compositor with `dbus-run-session`, its session bus isn't the one systemd's user units see; run `exec vinowatch` instead.

swaybar's own tray shows the icon but has no menus or tooltips, so use Waybar. X11 panels that only know the older XEmbed tray (i3bar, tint2, polybar) need [snixembed](https://git.sr.ht/~steef/snixembed) in between; there, right click opens the menu, and the icon neither hides nor turns amber.

If vinoWatch starts before the panel does, it waits for a tray host instead of exiting. The driver check's notification needs a notification daemon (mako, dunst, swaync on wlroots); without one the result still shows in the menu and tooltip.

## Console

```
vinowatch --console [SECONDS]    # print every SECONDS (default 2)
vinowatch --once                 # one poll, then exit
vinowatch --json                 # one JSON object per poll, for scripts
vinowatch --warn-below 25        # flag low memory under 25% available
```

```
$ vinowatch --once 1
vinoWatch: monitoring OpenVINO processes every 1s
NPU 37%
OpenVINO apps hold 1.4 GiB of 7.6 GiB RAM, 4.9 GiB available
  vinoWhisper (python) pid 4242: CPU 12.5%, RAM 812 MiB, NPU open
  vinoSpeak (python) pid 4311: CPU 0.0%, RAM 640 MiB
```

Build without the tray (and its D-Bus dependency) with `cargo build --release --no-default-features`.

## Distros

The release binaries are static (musl), so the `curl | sh` install works the same on Fedora, Ubuntu/Debian, Arch and openSUSE. It needs `curl` or `wget` (`wget -qO- <url> | sh` works too), `tar` and `sha256sum`, and a systemd user session for the autostart. Building from a checkout needs a Rust toolchain at `rust-version` (1.95) or newer: rustup, or the distro's `cargo` where it's that new.

Everything `install.sh` writes is under your home, so it never touches `/usr`. `install-npu-reset.sh` writes only to `/etc/systemd/system` and `/etc/polkit-1/rules.d`, and runs `/usr/sbin/modprobe`, which is where Fedora, Debian/Ubuntu, Arch and openSUSE all have it.

## Immutable distros

On Fedora Atomic (Silverblue, Kinoite, Bazzite) and openSUSE Aeon, use the `curl | sh` install; nothing needs layering with `rpm-ostree` or `transactional-update`. `sudo ~/.local/share/vinowatch/install-npu-reset.sh` works too, since `/etc` is writable on all of them.

To build from source there, build in a toolbox (or distrobox) and install from the host, where `systemctl --user` reaches your session:

```sh
toolbox run cargo build --release --locked   # after: toolbox run sudo dnf install cargo
./install.sh                                  # no cargo on the host: installs target/release/vinowatch
```

On openSUSE Aeon and Kalpa there's no toolbox: use distrobox with a Tumbleweed image (`distrobox create -i registry.opensuse.org/opensuse/tumbleweed`), then `distrobox enter -- sudo zypper install cargo` and build with `distrobox enter -- cargo build --release --locked`.

GNOME needs the AppIndicator extension for the tray, and stock Silverblue and Aeon don't ship it. Installing it per user from [extensions.gnome.org](https://extensions.gnome.org/extension/615/appindicator-support/) works with no layering; Bluefin ships it.

`install-npu-reset.sh` can't install on SteamOS, since an update resets `/etc`. That doesn't matter there: Steam Deck hardware has no NPU. On NixOS `/etc` is read-only and there's no `/usr/sbin/modprobe`, so the script stops and says so; declare the unit in `configuration.nix` instead.

## How it works

- **Which processes.** A process is listed if it has a `libopenvino*` file mapped (`/proc/<pid>/maps`), or `openvino` in its name, or if it is one of the vino apps' own programs, so helpers that never load OpenVINO count toward their app: vinoWhisper's tray, settings window and idle dictation process, vinoAuthFace's tray. Those are marked `helper` (`"openvino": false` in `--json`), add to the RAM total, and don't turn the icon green or count as OpenVINO processes in the tooltip. A program is its executable's file name, or for Python the script's file name or `-m` module; directories, arguments and the working directory don't count, so `pytest` in a vino checkout or an editor opening a vino file isn't listed. Names are matched on the file name only. Apps are told apart by command line, because the Python ones all show up as `python3`; vinoPop is recognised by its checkout path, in the command line or the working directory.
- **NPU load.** The `ivpu` driver's `npu_busy_time_us` counter, as busy time over wall-clock time between two polls. It is a device-wide number: the driver has no per-process counter. A process holding `/dev/accel/*` open is marked `NPU open`, which tells you who is probably using it, not how much.
- **RAM.** Per process this is PSS from `/proc/<pid>/smaps_rollup`: shared pages, like the mapped OpenVINO libraries, are split between the processes using them, so the figures add up to what the apps really hold. Where that file can't be read it falls back to resident size, which counts shared pages in every process (`--json` says which with `ram_kind`). "Available" is `MemAvailable` from `/proc/meminfo`, the kernel's own estimate of what can be used without swapping. `/proc/meminfo` ignores container and `MemoryMax` limits, so vinoWatch also reads the cgroup v2 limit above each listed process (`memory.max` against `memory.current`, minus inactive file cache as `docker stats` does) and reports whichever has the smaller share left. Then the line says `cgroup limit` instead of `RAM`, and the low-memory warning is judged against that limit (`--json` has `ram_limit`: `cgroup` or `host`). Only cgroup v2 is read.
- **CPU.** From `sysinfo`. It is a delta, so the first poll only sets a baseline and nothing is shown for it.

## Limits

- Other users' processes can't be inspected (their `maps` files are unreadable), so they aren't listed.
- A process that lives for less than one poll interval, like a single `sudo` face check, can be missed. Lower the interval, or use `--json` with a short one.
- No GPU or energy numbers yet. The NPU counter is only read from the `ivpu` driver.
- The NPU path is written against the driver's documented sysfs file and unit-tested with a fake one; it hasn't been run against a real NPU yet. The same goes for the stuck-driver probe and reset: the polkit rule and the reload were checked on a machine without an NPU, not on a stuck driver.

## Development

See [CONTRIBUTING.md](CONTRIBUTING.md). Security notes are in [SECURITY.md](SECURITY.md).

MIT licensed.
