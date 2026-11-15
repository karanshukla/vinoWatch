# Distros

The release binaries are static (musl), so the `curl | sh` install works the same on Fedora, Ubuntu/Debian, Arch and openSUSE.

| Needs | For |
|---|---|
| `curl` or `wget` (`wget -qO- <url> \| sh` works too), `tar`, `sha256sum` | the install |
| a systemd user session | the autostart |
| a Rust toolchain at `rust-version` (1.95) or newer: rustup, or the distro's `cargo` where it's that new | building from a checkout |

Everything `install.sh` writes is under your home, so it never touches `/usr`. `install-npu-reset.sh` writes only to `/etc/systemd/system` and `/etc/polkit-1/rules.d`, and runs `/usr/sbin/modprobe`, which is where Fedora, Debian/Ubuntu, Arch and openSUSE all have it.

## Immutable distros

On Fedora Atomic (Silverblue, Kinoite, Bazzite) and openSUSE Aeon, use the `curl | sh` install; nothing needs layering with `rpm-ostree` or `transactional-update`. `sudo ~/.local/share/vinowatch/install-npu-reset.sh` works too, since `/etc` is writable on all of them.

To build from source there, build in a toolbox (or distrobox) and install from the host, where `systemctl --user` reaches your session:

```sh
toolbox run cargo build --release --locked   # after: toolbox run sudo dnf install cargo
./install.sh                                  # no cargo on the host: installs target/release/vinowatch
```

On openSUSE Aeon and Kalpa there's no toolbox: use distrobox with a Tumbleweed image (`distrobox create -i registry.opensuse.org/opensuse/tumbleweed`), then `distrobox enter -- sudo zypper install cargo` and build with `distrobox enter -- cargo build --release --locked`.

GNOME needs the AppIndicator extension for the tray, and stock Silverblue and Aeon don't ship it. Installing it per user from [extensions.gnome.org](https://extensions.gnome.org/extension/615/appindicator-support/) works with no layering; Bluefin ships it.

## SteamOS and NixOS

`install-npu-reset.sh` can't install on SteamOS, since an update resets `/etc`. That doesn't matter there: Steam Deck hardware has no NPU.

On NixOS `/etc` is read-only and there's no `/usr/sbin/modprobe`, so the script stops and says so; declare the unit in `configuration.nix` instead.
