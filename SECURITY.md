# Security

`vinoWatch` runs as your regular user for the duration of your session and never runs as root. During normal operation it is read-only and opens no network sockets, reading only:

- `/proc/<pid>/{maps,cmdline,cwd,fd,smaps_rollup}`
- `/proc/meminfo`
- `/proc/<pid>/cgroup` and the `memory.*` files of ancestor cgroups
- `/sys/class/accel/*/device/npu_busy_time_us`
- The session D-Bus (to register and update the tray icon)

Aside from CLI flags and tray menu clicks, it accepts no external input.

Two tray menu actions run commands only when clicked:

- **Check NPU driver** runs a fixed Python probe script.
- **Reset NPU driver** runs `systemctl start vino-npu-reset.service`.

The privileged half of driver resets is opt-in via `sudo ./install-npu-reset.sh`. It installs a oneshot systemd service that runs `modprobe -r intel_vpu` followed by `modprobe intel_vpu`, along with a polkit rule allowing the active local user to start that single unit without a password. The unit takes no arguments and cannot stop, edit, or start any other service.

## Reporting

Report vulnerabilities through [private vulnerability reporting](https://github.com/karanshukla/vinoWatch/security/advisories/new). Do not open a public issue for anything exploitable.

## Threat model

| Threat | Mitigation |
|---|---|
| Hostile process name or command line corrupts output | Process names are printed as plain text and escaped in `--json`. Nothing read from `/proc` is ever passed to a shell. |
| Driver probe runs an interpreter path read from `/proc` | Without `--probe-python`, the probe executes `argv[0]` from one of your own processes that maps `libopenvino`, and only if it is an absolute path to a `python*` binary, using fixed arguments and no shell. Because only your own processes are readable, this cannot run anything beyond your existing user privileges (see the final row). |
| Another local user reloads the NPU driver | The polkit rule allows any active local seat session to start the reset unit. At worst, this causes a failed NPU request in another app; `modprobe -r` refuses to unload the module while any process holds `/dev/accel` open, so active NPU consumers are protected. |
| Reading other users' processes | Not attempted. The kernel's `ptrace` access controls govern `/proc/<pid>/{maps,cwd,fd}`, so unreadable processes are skipped. |
| Information disclosure via tray metadata | The tray tooltip and menu expose process names, PIDs, and app names to clients on your session D-Bus that can read StatusNotifierItem properties. This matches the standard visibility of desktop tray icons, and there is no network exposure. |
| Tampered release tarball | `install.sh` verifies the downloaded tarball against `SHA256SUMS` from the same release and aborts on a mismatch or missing file. This catches corrupted or swapped assets, though it cannot protect against a compromised GitHub release itself. |
| Compromised dependency or GitHub Action | Crates are restricted to crates.io (`cargo deny`), `Cargo.lock` is enforced with `--locked`, every CI action is pinned to a commit SHA, Dependabot waits 7 days before opening PRs, and nothing auto-merges. |
| Malicious process already running with the user's privileges | Out of scope, since such a process could already read the same `/proc` and sysfs files directly. The systemd user unit applies hardening (`NoNewPrivileges`, Unix sockets only, no namespaces) to limit the impact of a bug in `vinoWatch` itself. Read-only home/filesystem sandboxing is intentionally omitted because it requires a user namespace, which prevents reading other apps' `/proc` entries. |
