# CLAUDE.md

Guidance for Claude Code when working in this repository.

## What this is

`vinowatch`: a Rust tray daemon (and console tool) that lists processes using OpenVINO, with their CPU, RAM and, from the `ivpu` driver, system-wide NPU load. It knows the other vino projects (vinoWhisper, vinoSpeak, vinoPop, vinoAuthFace, vinoSearch) by command line. User docs are the README; don't duplicate them here.

## Layout

- `src/detect.rs`: is this process an OpenVINO user, which vino app is it. Takes the `/proc` root as a parameter.
- `src/npu.rs`: the busy-time counter and the percentage between polls.
- `src/sample.rs`: one poll, plus the `--json` shape.
- `src/tray.rs`: ksni StatusNotifierItem, behind the default `tray` feature.
- `src/icon.rs`, `src/raster.rs`: the icons, drawn from one shape list as tray pixmaps and as the SVGs `--install-icons` writes. Same palette and grid as vinoAuthFace's tray. `data/` holds the generated SVGs; after changing a shape run `VINOWATCH_BLESS_ICONS=1 cargo test`.
- `src/driver.rs`: the stuck-`intel_vpu` probe (runs Python, as vinoPop's `npu_canary.py`) and the reload via `vino-npu-reset.service`. Root half: `install-npu-reset.sh`, `vino-npu-reset.service`, `vino-npu-reset.rules`.
- `src/cli.rs`: flags.

## Commands

```bash
cargo test
cargo clippy --all-targets -- -D warnings
cargo clippy --no-default-features --all-targets -- -D warnings
cargo fmt --check
cargo deny check
```

CI runs those, plus `shellcheck` on both install scripts, `systemd-analyze verify` on both units and a `cargo +1.95 check` against `rust-version` (sysinfo sets that floor; raise both together).

## Gotchas

- **sysinfo lists every thread as a process.** `Sampler::poll` skips them with `thread_kind()`; without that, each multithreaded app shows up once per thread. `finds_a_process_by_its_mapped_library` guards it.
- **`comm` is truncated to 15 characters and Python apps are `python3`.** Name apps from the command line, never `Process::name()`.
- **The tray runtime needs `enable_all()`.** ksni's zbus needs tokio IO; with only `enable_time()` it panics at startup. No unit test reaches this, so check the tray by running it.
- **CPU and NPU are deltas.** The first poll is a baseline, so both the console and tray call `poll()` once and sleep before the first real one.
- **Add RAM up as PSS, not RSS.** The apps share the mapped OpenVINO libraries, so summed resident sizes overstate the total. `Proc::ram_is_pss` records which one a figure is.
- **`/proc/meminfo` is the whole machine, not the container.** `Sampler::poll` also takes the tightest cgroup v2 limit above each listed process (`detect::cgroup_headroom`) and uses whichever has the smaller share left. Checked live: a 900 MiB `systemd-run --user --scope -p MemoryMax=900M` scope around vinoSpeak flagged low memory before the OOM kill; without this it never did. cgroup v1 is ignored.
- **Low memory is informational.** It changes the tray icon and tooltip only; nothing stops or restarts an app, and nothing pops up. The only notification is the result of a driver check or reset the user clicked.
- **The probe's Python is a venv's, from `argv[0]`.** `/proc/<pid>/exe` resolves to the base interpreter, which can't import openvino. The tray keeps every one it has seen, because a reset needs the apps closed first.
- **polkit's `subject.local` needs a seat.** A seatless session (ssh, `systemd-run -p PAMName=login`) is never local, so the reset rule refuses it. `subject.active` falls back to the user's display session, which is what lets the tray's user service (in no session) through. To test on the VM: `sudo systemd-run -p PAMName=login -p User=dev -p TTYPath=/dev/tty5 -E XDG_SEAT=seat0 -E XDG_VTNR=5 -E XDG_SESSION_TYPE=wayland sleep 120`, then `sudo chvt 5`.
- **No `ProtectKernelModules=`/`ProtectKernelLogs=` in the user unit.** On systemd 255 they fail every start with 218/CAPABILITIES.
- No per-process NPU numbers exist: the `ivpu` driver only has a device counter. `NPU open` (an `/dev/accel/*` fd) is a hint, not a measurement.

## Testing on the linuxlab VM

No NPU there, so the NPU path is only covered by the fake-sysfs tests; never claim it works on hardware. Real OpenVINO is in the `.venv`s of vinoWhisper and vinoSpeak, so a process that does `import openvino; openvino.Core()` exercises the maps detection for real. The tray can be checked in the headless sway/waybar session (`busctl --user` shows the registered item).

## CI and repo rules

- Every Actions `uses:` is pinned to a commit SHA with a version comment.
- A `deny.toml` advisory ignore needs a written reason next to it.
- `release.yml` publishes on `v*` tags and refuses one that doesn't match `Cargo.toml`.
