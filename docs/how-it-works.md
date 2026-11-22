# How it works

## Monitored processes

`vinoWatch` lists a process if any of the following conditions match:

- It has a `libopenvino*` shared library mapped in `/proc/<pid>/maps`.
- Its process name contains `openvino`.
- It is a known binary or script belonging to one of the `vino*` applications.

The third rule ensures helper processes that do not load OpenVINO directly (such as `vinoWhisper`'s tray, settings window, and idle dictation process, or `vinoAuthFace`'s tray) still count toward their app's RAM usage. These processes are labeled `helper` (`"openvino": false` in `--json`), contribute to the RAM total, and do not turn the tray icon green or count as OpenVINO processes in the tooltip.

Matching checks only the executable's filename (or, for Python processes, the script filename or `-m` module name). Directory paths, arguments, and working directories are ignored so tools like `pytest` or editors running inside a `vino*` repository are not misclassified.

Apps are distinguished by command line because Python-based apps all appear as `python3`. `vinoPop` is identified by its checkout path in either the command line or working directory.

## NPU load

NPU utilization is calculated from the `ivpu` driver's `npu_busy_time_us` counter by dividing busy time by elapsed wall-clock time between consecutive polls. Because the driver exposes only a device-wide counter, there is no per-process NPU percentage. Processes holding an open file descriptor to `/dev/accel/*` are marked `NPU open` to show which processes are likely using the NPU.

## RAM

- **Per-process RAM:** Read as PSS (Proportional Set Size) from `/proc/<pid>/smaps_rollup`, which divides shared pages (such as mapped OpenVINO libraries) across the processes using them so totals reflect actual memory footprint. If `smaps_rollup` cannot be read, `vinoWatch` falls back to resident set size (RSS), which counts shared pages per process. In `--json`, `ram_kind` indicates which measurement was used.
- **Available host RAM:** Read from `MemAvailable` in `/proc/meminfo`, the kernel's estimate of memory available without swapping.
- **Container and cgroup limits:** Because `/proc/meminfo` ignores container and systemd `MemoryMax` limits, `vinoWatch` also inspects cgroup v2 limits above each listed process (`memory.max` compared against `memory.current` minus inactive file cache, matching `docker stats`). It reports whichever limit leaves less headroom, labels the line `cgroup limit` instead of `RAM`, and triggers low-memory warnings against that tighter limit (`ram_limit` is `"cgroup"` or `"host"` in `--json`). Only cgroup v2 is supported.

## CPU

CPU percentages are provided by `sysinfo`. Because CPU usage is computed as a delta between samples, the first poll establishes a baseline and reports no CPU values.

## Polling overhead

Each process's `/proc/<pid>/maps` is read once and re-read only when its virtual memory size changes (which happens when a library is loaded or unloaded). Reading `/proc/<pid>/maps` for every process on every poll previously accounted for most polling CPU time: ~85 ms out of ~140 ms across ~450 processes (measured 2026-11-10).

## What the tray shows

Hovering over the tray icon displays the NPU load, OpenVINO process count, and total RAM held by OpenVINO apps relative to available memory. Clicking the icon opens a menu with one row per process.

The tray icon displays three vertical meters styled after the other `vino*` apps:

| Icon state | Condition |
|---|---|
| Green | A process loading OpenVINO is running, or the NPU is busy (`vino*` helpers alone, such as trays, do not activate green) |
| Amber | Available RAM is below 15% of total (`--warn-below PERCENT` adjusts this threshold; `0` disables it), or the NPU driver is detected as stuck |
| Hidden (passive) | The NPU has been idle for 5 minutes |

Passive mode matches `vinoWhisper`'s tray behavior: KDE Plasma moves the icon into the overflow expander and Waybar hides it. Renewed NPU activity brings the icon back automatically. Idle apps do not keep the icon active, whereas a stuck NPU driver does.

Each menu row displays its own meter icon:

| Row | Meter meaning |
|---|---|
| NPU | Overall NPU utilization |
| RAM | Overall RAM utilization |
| Process | Process CPU usage (full bar = one CPU core), process RAM (full bar = 20% of system RAM), and whether the process holds `/dev/accel/*` open |

Row icons are white while idle, green while active, and amber for any process using 20% or more of RAM (or the largest consumer when system RAM is low).

`vinoWatch` only updates its tray icon and tooltip. It never shows unsolicited popups (the single desktop notification is the direct response to a manual **Check NPU driver** action) and never terminates processes on your behalf.
