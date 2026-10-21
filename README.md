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

Hover the tray icon for the NPU load, a process count and how much RAM the OpenVINO apps hold against what the machine has free; click it for one row per process. The icon is three meters in the vino apps' style: green while an OpenVINO app runs or the NPU is busy, and amber if available RAM drops under 15% of the total (`--warn-below PERCENT` changes that, `0` turns it off). With nothing to watch it goes passive, as vinoWhisper's does, so Plasma tucks it under the expander and Waybar hides it. It only changes the icon and tooltip: no popups (the one notification is the answer to a driver check you asked for), and nothing is ever stopped for you. The tray needs a StatusNotifierItem host: KDE Plasma and Waybar have one, GNOME needs the AppIndicator extension.

## Stuck NPU driver

After heavy NPU use the `intel_vpu` driver can get stuck: any process with two OpenVINO Cores touching the NPU segfaults in `libze_loader`, so vinoPop crashes on every AI action and vinoAuthFace on every scan. Reloading the driver clears it without a reboot.

With an NPU present the tray menu has **Check NPU driver**. It runs a ~0.3 s probe (vinoPop's: a toy model on the NPU, a second Core, the model again; a segfault means stuck) on demand, never on a timer, and shows the result as a desktop notification as well as in the menu and tooltip. A stuck driver turns the icon amber and keeps it in view. The probe needs a Python that can import openvino: `--probe-python PATH`, else the venv interpreter of a vino app vinoWatch has seen running, else `python3`. `vinowatch --check-npu` runs the same probe from a terminal (exit 0 OK, 1 stuck, 3 couldn't tell).

**Reset NPU driver** reloads it (`modprobe -r intel_vpu && modprobe intel_vpu`) through a root oneshot unit, and checks again afterwards. That needs a one-time root step, which `install.sh` doesn't do:

```sh
sudo ./install-npu-reset.sh              # from a checkout or an unpacked release
sudo ./install-npu-reset.sh --uninstall
```

It installs `vino-npu-reset.service` and a polkit rule that lets the active local user start that unit, and nothing else, without a password. `modprobe -r` refuses while anything has `/dev/accel` open, so the item lists the apps holding the NPU and stays disabled until you close them. Nothing is stopped for you. Holders that belong to other users aren't visible; then the reset fails and says so.

## Install

```sh
curl -fsSL https://raw.githubusercontent.com/karanshukla/vinoWatch/main/install.sh | sh
```

This downloads the latest release, checks it against `SHA256SUMS`, puts `vinowatch` in `~/.local/bin`, its icons in `~/.local/share/icons/hicolor`, and enables a systemd user unit that starts the tray with your graphical session. From a checkout, `./install.sh` builds from source instead. `./install.sh --uninstall` removes both.

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

## How it works

- **Which processes.** A process is listed if it has a `libopenvino*` file mapped (`/proc/<pid>/maps`), or `openvino` in its name. Names are matched on the file name only. Apps are told apart by command line, because the Python ones all show up as `python3`; vinoPop is recognised by its checkout path, in the command line or the working directory.
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
