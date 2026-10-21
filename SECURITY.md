# Security

vinoWatch runs as you, for the whole session, and mostly reads: `/proc/<pid>/{maps,cmdline,cwd,fd,smaps_rollup}`, `/proc/meminfo`, `/proc/<pid>/cgroup` and the `memory.*` files of the cgroups above each process, `/sys/class/accel/*/device/npu_busy_time_us`, and the session D-Bus to show the tray icon. It writes nothing, opens no network sockets, and takes no input except its command-line flags and its tray menu. It never runs as root.

Two menu items run something, only when clicked: **Check NPU driver** runs a fixed Python script, and **Reset NPU driver** runs `systemctl start vino-npu-reset.service`. The reset's root half is opt-in (`sudo ./install-npu-reset.sh`): a oneshot unit that runs `modprobe -r intel_vpu` then `modprobe intel_vpu`, and a polkit rule that lets the active local user start that one unit without a password. It can't stop, edit or start anything else, and takes no arguments.

## Reporting

Use [private vulnerability reporting](https://github.com/karanshukla/vinoWatch/security/advisories/new). Do not open a public issue for anything exploitable.

## Threat model

| Threat | Defence |
|---|---|
| A hostile process name or command line breaks the output | Names are printed as text, and escaped in `--json`. Nothing from `/proc` is passed to a shell. |
| The probe runs an interpreter path read from `/proc` | Without `--probe-python`, the probe runs `argv[0]` of one of your own processes that maps libopenvino, if it is an absolute path to a `python*`, with fixed arguments and no shell. Only your own processes are readable, so this runs nothing a process with your rights couldn't already run (see the last row). |
| Another local user reloads the NPU driver | The polkit rule admits any active local session, so whoever is at the seat can reload it. The worst case is a failed NPU request in someone else's app, and `modprobe -r` refuses while `/dev/accel` is open, so an app using the NPU keeps it. |
| Reading other users' processes | Not attempted. The kernel's `ptrace` access rules decide what `maps`, `cwd` and `fd` return; an unreadable process is just not listed. |
| It leaks what you run | The tray tooltip and menu list process names, pids and app names to anything on your session bus that can read the StatusNotifierItem. That is the same exposure as any tray icon; there is no network path. |
| Tampered release binary | `install.sh` verifies the tarball against `SHA256SUMS` from the same release and refuses to install on a mismatch or a missing file. This protects against a corrupted or swapped asset, not against a compromised release. |
| Compromised dependency or action | Crates from crates.io only (`cargo deny`), `Cargo.lock` enforced with `--locked`, every CI action pinned to a commit, Dependabot waits 7 days and nothing auto-merges. |
| A malicious process with the user's rights | Not defended against: it could already read the same files. The unit adds systemd hardening (no new privileges, Unix sockets only, no namespaces), which limits what a bug in vinoWatch itself could do. It has no read-only home or filesystem: those need a user namespace, and from inside one the other apps' /proc entries are unreadable. |
