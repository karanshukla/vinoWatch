# How it works

## Which processes

A process is listed if any of these holds:

- it has a `libopenvino*` file mapped (`/proc/<pid>/maps`),
- it has `openvino` in its name,
- it is one of the vino apps' own programs.

The last rule is there so helpers that never load OpenVINO count toward their app: vinoWhisper's tray, settings window and idle dictation process, vinoAuthFace's tray. Those are marked `helper` (`"openvino": false` in `--json`), add to the RAM total, and don't turn the icon green or count as OpenVINO processes in the tooltip.

A program is its executable's file name, or for Python the script's file name or `-m` module. Directories, arguments and the working directory don't count, so `pytest` in a vino checkout or an editor opening a vino file isn't listed.

Apps are told apart by command line, because the Python ones all show up as `python3`. vinoPop is recognised by its checkout path, in the command line or the working directory.

## NPU load

The `ivpu` driver's `npu_busy_time_us` counter, as busy time over wall-clock time between two polls. It is a device-wide number: the driver has no per-process counter. A process holding `/dev/accel/*` open is marked `NPU open`, which tells you who is probably using it, not how much.

## RAM

Per process this is PSS from `/proc/<pid>/smaps_rollup`: shared pages, like the mapped OpenVINO libraries, are split between the processes using them, so the figures add up to what the apps really hold. Where that file can't be read it falls back to resident size, which counts shared pages in every process (`--json` says which with `ram_kind`).

"Available" is `MemAvailable` from `/proc/meminfo`, the kernel's own estimate of what can be used without swapping.

`/proc/meminfo` ignores container and `MemoryMax` limits, so vinoWatch also reads the cgroup v2 limit above each listed process (`memory.max` against `memory.current`, minus inactive file cache as `docker stats` does) and reports whichever has the smaller share left. Then the line says `cgroup limit` instead of `RAM`, and the low-memory warning is judged against that limit (`--json` has `ram_limit`: `cgroup` or `host`). Only cgroup v2 is read.

## CPU

From `sysinfo`. It is a delta, so the first poll only sets a baseline and nothing is shown for it.

## The poll's own cost

Each process's maps are read once and again only when its virtual size changes, which loading a library does. Reading `/proc/<pid>/maps` for every process on every poll was most of a poll's CPU: ~85 of ~140 ms with ~450 processes (measured 2026-11-10).

## What the tray shows

Hover the icon for the NPU load, a process count and how much RAM the OpenVINO apps hold against what the machine has free. Click it for one row per process.

The icon is three meters in the vino apps' style:

| Icon | When |
|---|---|
| green | a process that loads OpenVINO runs, or the NPU is busy (a vino app's helper alone, like its tray, doesn't count) |
| amber | available RAM is under 15% of the total (`--warn-below PERCENT` changes that, `0` turns it off), or the driver is stuck |
| hidden (passive) | the NPU has been idle for 5 minutes |

Passive is what vinoWhisper's tray does too: Plasma tucks the icon under the expander and Waybar hides it. NPU load brings it back. Apps left running with the NPU idle don't keep it in view, a stuck driver does.

Each menu row has its own meter icon:

| Row | Meter |
|---|---|
| NPU | the NPU load |
| RAM | how full RAM is |
| a process | its CPU (full is one core), its RAM (full is a fifth of the machine's) and whether it holds the NPU |

A row is white while that thing idles, green while it works, and amber for the process holding a fifth of RAM or more, or the biggest one while RAM is low.

It only changes the icon and tooltip. There are no popups (the one notification is the answer to a driver check you asked for), and nothing is ever stopped for you.
