# Security

vinoWatch runs as you, for the whole session, and only reads: `/proc/<pid>/{maps,cmdline,cwd,fd,smaps_rollup}`, `/proc/meminfo`, `/sys/class/accel/*/device/npu_busy_time_us`, and the session D-Bus to show the tray icon. It writes nothing, opens no sockets, and takes no input except its command-line flags. It never runs as root and never needs to.

## Reporting

Use [private vulnerability reporting](https://github.com/karanshukla/vinoWatch/security/advisories/new). Do not open a public issue for anything exploitable.

## Threat model

| Threat | Defence |
|---|---|
| A hostile process name or command line breaks the output | Names are printed as text, and escaped in `--json`. Nothing from `/proc` is ever run, or passed to a shell. |
| Reading other users' processes | Not attempted. The kernel's `ptrace` access rules decide what `maps`, `cwd` and `fd` return; an unreadable process is just not listed. |
| It leaks what you run | The tray tooltip and menu list process names, pids and app names to anything on your session bus that can read the StatusNotifierItem. That is the same exposure as any tray icon; there is no network path. |
| Tampered release binary | `install.sh` verifies the tarball against `SHA256SUMS` from the same release and refuses to install on a mismatch or a missing file. This protects against a corrupted or swapped asset, not against a compromised release. |
| Compromised dependency or action | Crates from crates.io only (`cargo deny`), `Cargo.lock` enforced with `--locked`, every CI action pinned to a commit, Dependabot waits 7 days and nothing auto-merges. |
| A malicious process with the user's rights | Not defended against: it could already read the same files. The unit adds systemd hardening (read-only home, no new privileges, Unix sockets only), which limits what a bug in vinoWatch itself could do. |
