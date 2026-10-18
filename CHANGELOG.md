# Changelog

All notable changes to vinoWatch are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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
