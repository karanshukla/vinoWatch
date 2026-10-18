# Changelog

All notable changes to vinoWatch are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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

### Fixed

- Each thread of a multithreaded app was listed as its own process.
- A file with `libopenvino` anywhere in its path counted as the library; only the file name does now.
- The NPU figure never appeared if the counter wasn't readable at startup, and a reset counter was reported as 0 busy.
- Output order changed on every poll; it is now apps first, busiest first.
