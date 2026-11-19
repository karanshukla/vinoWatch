# vinoWatch

A lightweight Linux system tray daemon that monitors the resource usage of OpenVINO applications: CPU, RAM, and (on Intel hardware with the `ivpu` driver) NPU load.

## Recognized applications

`vinoWatch` identifies other `vino*` applications by name:

| App | Detected as |
|---|---|
| [vinoWhisper](https://github.com/karanshukla/vinoWhisper) | `vinowhisper-server`, `-caption`, `-dictate`, `-replay` |
| [vinoSpeak](https://github.com/karanshukla/vinoSpeak) | `vinospeak` |
| [vinoPop](https://github.com/karanshukla/vinoPop) | `vinopop` launcher, or `main.py` inside a vinoPop checkout |
| [vinoAuthFace](https://github.com/karanshukla/vinoAuthFace) | `vinoauthface`, `vinoauthface-auth` |
| [vinoSearch](https://github.com/karanshukla/vinoSearch) | `vinosearch` |

Any other process that maps `libopenvino*` is also monitored and listed by its process name.

## System tray

| Element | What it displays |
|---|---|
| Tooltip | Current NPU load, OpenVINO process count, and total RAM used by OpenVINO apps compared to available memory |
| Menu | One row per monitored process, each with its own CPU, RAM, and NPU indicators |
| Icon | Green while an OpenVINO process is running or the NPU is busy; amber when available RAM drops below 15% (`--warn-below PERCENT`) or the NPU driver is stuck; hidden (passive) after the NPU has been idle for 5 minutes |

The tray menu also includes **Check NPU driver** and **Reset NPU driver** actions for handling a [stuck `intel_vpu` driver](docs/npu-driver.md).

`vinoWatch` only updates the tray icon and tooltip; it never displays unsolicited popups or terminates processes automatically. See [What the tray shows](docs/how-it-works.md#what-the-tray-shows) for icon color and meter details.

## Install

```sh
curl -fsSL https://github.com/karanshukla/vinoWatch/releases/latest/download/install.sh | sh
```

The installer downloads the latest release, verifies it against `SHA256SUMS`, and installs everything under your home directory:

| Component | Location |
|---|---|
| `vinowatch` binary | `~/.local/bin` |
| Icons | `~/.local/share/icons/hicolor` |
| NPU reset installer | `~/.local/share/vinowatch` |
| systemd user service | Enabled to start with your graphical session |

**Requirements:** `curl` or `wget`, `tar`, `sha256sum`, and a systemd user session. The tray requires a StatusNotifierItem host (built in on KDE Plasma, available via an extension on GNOME, and provided by Waybar on sway and Hyprland). See [Desktops](docs/desktops.md) for the full compatibility table.

Running `./install.sh` inside a Git checkout builds from source instead of downloading a release. Run `./install.sh --uninstall` to remove `vinoWatch` (if you installed the NPU reset service, run `install-npu-reset.sh --uninstall` first).

### Upgrading

Click **Check for updates** under the version number in the tray menu. `vinoWatch` only queries GitHub when you trigger that action; if a newer release exists, it offers **Update to vN**, which downloads, verifies, installs the new release, and restarts the tray. You can perform the same upgrade from a terminal with `VINOWATCH_VERSION=vN vinowatch-upgrade`.

## Console mode

```sh
vinowatch --console [SECONDS]    # print every SECONDS (default 2)
vinowatch --once                 # run one poll, then exit
vinowatch --json                 # emit one JSON object per poll for scripts
vinowatch --warn-below 25        # warn when available memory falls below 25%
vinowatch --check-npu            # probe the NPU driver (exit 0 OK, 1 stuck, 3 unknown)
```

Example output:

```text
$ vinowatch --once 1
vinoWatch: monitoring OpenVINO processes every 1s
NPU 37%
OpenVINO apps hold 1.4 GiB of 7.6 GiB RAM, 4.9 GiB available
  vinoWhisper (python) pid 4242: CPU 12.5%, RAM 812 MiB, NPU open
  vinoSpeak (python) pid 4311: CPU 0.0%, RAM 640 MiB
```

To build without the tray and its D-Bus dependencies:

```sh
cargo build --release --no-default-features
```

## Stuck NPU driver

After heavy NPU usage, the `intel_vpu` kernel driver can enter a stuck state where `vinoPop` and `vinoAuthFace` crash on NPU operations. You can test for this state on demand via **Check NPU driver** in the tray menu and reload the driver without rebooting via **Reset NPU driver**.

Driver resets require a one-time root setup that `install.sh` does not run automatically:

```sh
sudo ~/.local/share/vinowatch/install-npu-reset.sh
```

See [Stuck NPU driver](docs/npu-driver.md) for details on the probe, the polkit rule, and what can block a reset.

## Limitations

- Processes owned by other users cannot be inspected because their `/proc/<pid>/maps` files are unreadable, so they are omitted.
- Short-lived processes that exit within a single poll interval (such as a single `sudo` face authentication check) may be missed. Lower the poll interval or use `--json` with a shorter interval to catch them.
- GPU and power/energy metrics are not collected yet. The NPU counter is read only from the `ivpu` driver and reports device-wide utilization rather than per-process NPU usage.
- The NPU monitoring code was written against the driver's documented sysfs interface and unit-tested with a simulated sysfs, and has not yet been tested on real NPU hardware. Likewise, the stuck-driver probe, polkit rule, and module reload were tested on a machine without an NPU rather than against a live stuck driver.

## Documentation

| Page | Contents |
|---|---|
| [How it works](docs/how-it-works.md) | Process detection, NPU/RAM/CPU metric sources, and tray icon colors and meters |
| [Desktops](docs/desktops.md) | StatusNotifierItem tray hosts per desktop, sway/Hyprland startup, X11 panels, and notifications |
| [Distros](docs/distros.md) | Install requirements and file paths, immutable distros, SteamOS, and NixOS |
| [Stuck NPU driver](docs/npu-driver.md) | The driver probe, reset service, and polkit setup |
| [Hand-written code](docs/hand-written.md) | Binary-size and dependency comparisons for `serde_json`, `png`, and `lexopt` |
| [CONTRIBUTING.md](CONTRIBUTING.md) | Development setup, tests, adding a new `vino*` app, and releases |
| [SECURITY.md](SECURITY.md) | Security model and vulnerability reporting |

## License

MIT.
