# vinoWatch

A small tray daemon that shows what the OpenVINO apps on your machine cost: CPU, RAM and, on Intel machines with the `ivpu` driver, NPU load. It knows the other vino projects by name:

| App | Seen as |
|---|---|
| [vinoWhisper](https://github.com/karanshukla/vinoWhisper) | `vinowhisper-server`, `-caption`, `-dictate`, `-replay` |
| [vinoSpeak](https://github.com/karanshukla/vinoSpeak) | `vinospeak` |
| [vinoPop](https://github.com/karanshukla/vinoPop) | the `main.py` run from its checkout |
| [vinoAuthFace](https://github.com/karanshukla/vinoAuthFace) | `vinoauthface`, `vinoauthface-auth` |

Anything else that loads `libopenvino*` is listed too, just without an app name.

Hover the tray icon for the NPU load and a process count; click it for one row per process. The tray needs a StatusNotifierItem host: KDE Plasma and Waybar have one, GNOME needs the AppIndicator extension.

## Install

```sh
curl -fsSL https://raw.githubusercontent.com/karanshukla/vinoWatch/main/install.sh | sh
```

This downloads the latest release, checks it against `SHA256SUMS`, puts `vinowatch` in `~/.local/bin` and enables a systemd user unit that starts the tray with your graphical session. From a checkout, `./install.sh` builds from source instead. `./install.sh --uninstall` removes both.

## Console

```
vinowatch --console [SECONDS]    # print every SECONDS (default 2)
vinowatch --once                 # one poll, then exit
vinowatch --json                 # one JSON object per poll, for scripts
```

```
$ vinowatch --once 1
vinoWatch: monitoring OpenVINO processes every 1s
NPU 37%
  vinoWhisper (python) pid 4242: CPU 12.5%, RAM 812 MiB, NPU open
  vinoSpeak (python) pid 4311: CPU 0.0%, RAM 640 MiB
```

Build without the tray (and its D-Bus dependency) with `cargo build --release --no-default-features`.

## How it works

- **Which processes.** A process is listed if it has a `libopenvino*` file mapped (`/proc/<pid>/maps`), or `openvino` in its name. Names are matched on the file name only. Apps are told apart by command line, because the Python ones all show up as `python3`; vinoPop is recognised by its working directory.
- **NPU load.** The `ivpu` driver's `npu_busy_time_us` counter, as busy time over wall-clock time between two polls. It is a device-wide number: the driver has no per-process counter. A process holding `/dev/accel/*` open is marked `NPU open`, which tells you who is probably using it, not how much.
- **CPU and RAM.** From `sysinfo`. CPU is a delta, so the first poll only sets a baseline and nothing is shown for it.

## Limits

- Other users' processes can't be inspected (their `maps` files are unreadable), so they aren't listed.
- A process that lives for less than one poll interval, like a single `sudo` face check, can be missed. Lower the interval, or use `--json` with a short one.
- No GPU or energy numbers yet. The NPU counter is only read from the `ivpu` driver.
- The NPU path is written against the driver's documented sysfs file and unit-tested with a fake one; it hasn't been run against a real NPU yet.

## Development

See [CONTRIBUTING.md](CONTRIBUTING.md). Security notes are in [SECURITY.md](SECURITY.md).

MIT licensed.
