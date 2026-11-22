# Distros

Release binaries are statically linked with musl, so the `curl | sh` installer works identically across Fedora, Ubuntu/Debian, Arch, and openSUSE.

## Requirements

| Requirement | Used for |
|---|---|
| `curl` or `wget` (`wget -qO- <url> \| sh` also works), `tar`, `sha256sum` | Downloading and verifying releases |
| A systemd user session | Autostarting the tray service |
| Rust toolchain at `rust-version` (`1.95`) or newer (via `rustup` or distro `cargo`) | Building from a Git checkout |

All files created by `install.sh` are placed inside your home directory, without modifying `/usr`. The optional `install-npu-reset.sh` script writes only to `/etc/systemd/system` and `/etc/polkit-1/rules.d`, and invokes `/usr/sbin/modprobe` (present at that path on Fedora, Debian/Ubuntu, Arch, and openSUSE).

## Immutable distros

On Fedora Atomic (Silverblue, Kinoite, Bazzite) and openSUSE Aeon, use the standard `curl | sh` install; no `rpm-ostree` or `transactional-update` package layering is needed. `sudo ~/.local/share/vinowatch/install-npu-reset.sh` also works because `/etc` is writable on all of them.

To build from source on an immutable system, compile inside a `toolbox` (or `distrobox`) container and run `./install.sh` on the host so `systemctl --user` can reach your session:

```sh
toolbox run cargo build --release --locked   # after: toolbox run sudo dnf install cargo
./install.sh                                  # installs target/release/vinowatch without host cargo
```

On openSUSE Aeon and Kalpa (which do not ship `toolbox`), use `distrobox` with a Tumbleweed image:

```sh
distrobox create -i registry.opensuse.org/opensuse/tumbleweed
distrobox enter -- sudo zypper install cargo
distrobox enter -- cargo build --release --locked
```

On GNOME-based immutable distros, the tray requires the AppIndicator extension, which stock Silverblue and Aeon do not preinstall (Bluefin includes it). Installing it per user from [extensions.gnome.org](https://extensions.gnome.org/extension/615/appindicator-support/) works without layering packages.

## SteamOS and NixOS

- **SteamOS:** `install-npu-reset.sh` cannot persist across system updates because updates reset `/etc`. This has no practical impact on Steam Deck hardware, which does not include an Intel NPU.
- **NixOS:** `/etc` is read-only and `/usr/sbin/modprobe` does not exist, so `install-npu-reset.sh` exits with an explanation. Declare the reset unit directly in `configuration.nix` instead.
