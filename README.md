# vinoWatch

A small tray daemon that shows what the OpenVINO apps on your machine cost: CPU, RAM and, on Intel machines with the `ivpu` driver, NPU load.

It knows the other vino projects by name:

| App | Seen as |
|---|---|
| [vinoWhisper](https://github.com/karanshukla/vinoWhisper) | `vinowhisper-server`, `-caption`, `-dictate`, `-replay` |
| [vinoSpeak](https://github.com/karanshukla/vinoSpeak) | `vinospeak` |
| [vinoPop](https://github.com/karanshukla/vinoPop) | its `vinopop` launcher, or a `main.py` in its checkout |
| [vinoAuthFace](https://github.com/karanshukla/vinoAuthFace) | `vinoauthface`, `vinoauthface-auth` |
| [vinoSearch](https://github.com/karanshukla/vinoSearch) | `vinosearch` |

Anything else that loads `libopenvino*` is listed too, just without an app name.

## The tray

| Where | What it shows |
|---|---|
| Tooltip | the NPU load, a process count, and how much RAM the OpenVINO apps hold against what the machine has free |
| Menu | one row per process, each with its own CPU, RAM and NPU meters |
| Icon | green while OpenVINO runs or the NPU is busy, amber when available RAM drops under 15% (`--warn-below PERCENT`), hidden once the NPU has been idle for 5 minutes |

The menu also has **Check NPU driver** and **Reset NPU driver**, for the [stuck `intel_vpu` driver](docs/npu-driver.md).

It only changes the icon and tooltip: no popups, and nothing is ever stopped for you. The colours and meters are spelled out in [What the tray shows](docs/how-it-works.md#what-the-tray-shows).

## Install

```sh
curl -fsSL https://github.com/karanshukla/vinoWatch/releases/latest/download/install.sh | sh
```

This downloads the latest release, checks it against `SHA256SUMS`, and installs under your home:

| What | Where |
|---|---|
| `vinowatch` | `~/.local/bin` |
| icons | `~/.local/share/icons/hicolor` |
| the NPU reset installer | `~/.local/share/vinowatch` |
| a systemd user unit that starts the tray with your graphical session | enabled |

It needs `curl` or `wget`, `tar`, `sha256sum` and a systemd user session. The tray needs a StatusNotifierItem host: built in on Plasma, an extension on GNOME, Waybar on sway and Hyprland. [Desktops](docs/desktops.md) has the table.

From a checkout, `./install.sh` builds from source instead. `./install.sh --uninstall` removes all of it; run `install-npu-reset.sh --uninstall` first if you installed the reset unit.

### Upgrade

Use **Check for updates** in the tray menu, under the version. It asks GitHub only when you click it, then offers **Update to vN**, which installs that release the same way and restarts the tray. `VINOWATCH_VERSION=vN vinowatch-upgrade` does the same from a terminal.

## Console

```
vinowatch --console [SECONDS]    # print every SECONDS (default 2)
vinowatch --once                 # one poll, then exit
vinowatch --json                 # one JSON object per poll, for scripts
vinowatch --warn-below 25        # flag low memory under 25% available
vinowatch --check-npu            # is the NPU driver stuck? exit 0 OK, 1 stuck, 3 couldn't tell
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

## Stuck NPU driver

After heavy NPU use the `intel_vpu` driver can get stuck, and vinoPop and vinoAuthFace then crash on every NPU action. The tray can check for it (**Check NPU driver**, on demand only) and reload the driver without a reboot (**Reset NPU driver**). The reset needs a one-time root step, which `install.sh` doesn't do:

```sh
sudo ~/.local/share/vinowatch/install-npu-reset.sh
```

[Stuck NPU driver](docs/npu-driver.md) has the probe, the polkit rule it installs and what blocks a reset.

## Limits

- Other users' processes can't be inspected (their `maps` files are unreadable), so they aren't listed.
- A process that lives for less than one poll interval, like a single `sudo` face check, can be missed. Lower the interval, or use `--json` with a short one.
- No GPU or energy numbers yet. The NPU counter is only read from the `ivpu` driver, and it is device-wide: there is no per-process NPU figure.
- The NPU path is written against the driver's documented sysfs file and unit-tested with a fake one; it hasn't been run against a real NPU yet. The same goes for the stuck-driver probe and reset: the polkit rule and the reload were checked on a machine without an NPU, not on a stuck driver.

## Docs

| Page | What's in it |
|---|---|
| [How it works](docs/how-it-works.md) | which processes are listed, where the NPU, RAM and CPU numbers come from, what each icon colour means |
| [Desktops](docs/desktops.md) | tray hosts per desktop, sway and Hyprland start-up, X11 panels, notifications |
| [Distros](docs/distros.md) | what the install needs and writes, immutable distros, SteamOS, NixOS |
| [Stuck NPU driver](docs/npu-driver.md) | the check, the reset and its root step |
| [Hand-written code](docs/hand-written.md) | what `serde_json`, `png` and `lexopt` cost when tried |
| [CONTRIBUTING.md](CONTRIBUTING.md) | setup, tests, adding another vino app, releases |
| [SECURITY.md](SECURITY.md) | security notes |

MIT licensed.
