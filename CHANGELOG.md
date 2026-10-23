# Changelog

All notable changes to vinoWatch are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.3.0] - 2026-10-23

### Changed

- The tray icon hides once the NPU has been under 1% for 5 minutes, as the other vino trays do, instead of staying in view whenever an OpenVINO app is resident. NPU load brings it back; a stuck driver or a running driver check keeps it in view.

### Fixed

- The empty track of each meter, in the tray icon and every menu row, was 2.1:1 against its tile, so a meter's length didn't show. It is now 3:1, as are the symbolic icons' tracks on Breeze Light.

## [0.2.0] - 2026-10-20

### Added

- **Check NPU driver** in the tray menu: a short probe on demand (vinoPop's canary) that tells whether the `intel_vpu` driver is stuck. A stuck driver turns the icon amber and keeps it in view. `vinowatch --check-npu` runs the same probe from a terminal, and `--probe-python PATH` picks its interpreter.
- **Reset NPU driver** reloads `intel_vpu` through a root oneshot unit and checks again afterwards. It lists the apps holding the NPU and waits for them to close. Needs a one-time `sudo ./install-npu-reset.sh`.
- The result of a driver check or reset is also sent as a desktop notification.
- Every tray menu row has an icon. The NPU and RAM rows show their level; a process row shows that process's CPU, RAM and whether it holds the NPU, white while idle, green while working and amber when it holds 20% of RAM or more.
- vinoSearch's runner is recognised.

### Changed

- The panel icon uses vinoWhisper's and vinoPop's frame, with the meters drawn in the same line style.
- The app icon is cropped to a 2-unit margin, so it draws the same size as other apps' icons.
- `install.sh` makes Plasma reload icons after installing them, so an updated icon shows without logging out.

### Fixed

- Under the systemd unit the tray couldn't read the other apps' `/proc` maps, PSS or open files: the unit's `ProtectSystem=`/`ProtectHome=`/`ProtectControlGroups=` put it in a user namespace. It listed processes that don't use OpenVINO, reported RSS, never showed `NPU open`, and couldn't take the single-instance lock. Those lines are gone.
- The unit failed every start on systemd 255 (Ubuntu 24.04) with 218/CAPABILITIES; `ProtectKernelModules=` and `ProtectKernelLogs=` are gone.
- NPU load from a process vinoWatch can't list now turns the icon green, as the menu already said.

## [0.1.0] - 2026-10-18

### Added

- System tray icon (StatusNotifierItem): NPU load and process count in the tooltip, one menu row per process.
- Recognises vinoWhisper, vinoSpeak, vinoPop and vinoAuthFace processes by command line; other `libopenvino` users are listed without an app name.
- `--console`, `--once` and `--json` modes. The poll interval is validated (1 to 3600 seconds).
- Marks processes that hold an `/dev/accel/*` node open.
- `install.sh` (checksum-verified download or build from source) and a hardened systemd user unit.
- CI (fmt, clippy with and without the tray, tests, cargo-deny, shellcheck, unit verify), tagged releases with `SHA256SUMS`, Dependabot.

- Total RAM held by the OpenVINO apps next to system RAM and what is available, in the tray, console and `--json`.
- Per-process RAM is PSS (shared pages split between their users), so it adds up; resident size is the fallback.
- Memory limits from cgroup v2 (containers, units with `MemoryMax`) count: when a listed app is closer to its limit than the machine is to running out, "available" and the warning use the limit.
- Low-memory warning: the tray icon changes when available RAM falls under `--warn-below` percent (default 15, 0 disables). Informational only.
- Tray and app icons in the vino house style (three meters on the dark tile): idle, active and low-memory states, as themed `-symbolic` icons with drawn pixmaps as the fallback. `--install-icons DIR` writes them, and `install.sh` runs it.
- The tray refuses to start a second copy in the same session (a lock in `$XDG_RUNTIME_DIR`).
- `-V` for `--version`, as the other vino tools take.
- vinoPop started through its new `vinopop` launcher is recognised.

### Changed

- The tray hides itself (StatusNotifierItem `Passive`, as vinoWhisper's does) unless an OpenVINO process is listed or the NPU is busy.
- The tray is filed as an application status item, like the other vino trays, and the menu shows the version. Underscores in process names are no longer swallowed by the menu.

### Fixed

- `vinoauthface-auth` never showed: it is setgid, so its `/proc` maps can't be read. Such a process is now listed when its executable is a vino one.
- Each thread of a multithreaded app was listed as its own process.
- A file with `libopenvino` anywhere in its path counted as the library; only the file name does now.
- The NPU figure never appeared if the counter wasn't readable at startup, and a reset counter was reported as 0 busy.
- Output order changed on every poll; it is now apps first, busiest first.
