#!/bin/sh
# Install vinoWatch for the current user: the binary and a systemd user unit
# that starts the tray with the graphical session. Run with --uninstall to
# remove them again.
#
# From a checkout it builds from source. Anywhere else, e.g. piped from curl,
# it downloads the latest release, or the one VINOWATCH_VERSION names
# (e.g. VINOWATCH_VERSION=v0.1.0):
#   curl -fsSL https://raw.githubusercontent.com/karanshukla/vinoWatch/main/install.sh | sh
set -eu

repo="karanshukla/vinoWatch"
bin="$HOME/.local/bin/vinowatch"
unit_dir="${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user"
data_dir="${XDG_DATA_HOME:-$HOME/.local/share}"
hicolor="$data_dir/icons/hicolor"

if [ "${1:-}" = "--uninstall" ]; then
    systemctl --user disable --now vinowatch.service 2>/dev/null || true
    rm -f "$bin" "$unit_dir/vinowatch.service" \
        "$hicolor/scalable/apps/vinowatch.svg" \
        "$hicolor/symbolic/apps/vinowatch-symbolic.svg" \
        "$hicolor/symbolic/apps/vinowatch-active-symbolic.svg" \
        "$hicolor/symbolic/apps/vinowatch-low-memory-symbolic.svg"
    systemctl --user daemon-reload 2>/dev/null || true
    echo "Removed vinoWatch."
    exit 0
fi

# Piped into sh, $0 is the shell rather than this file, and dirname "$0" is
# whatever directory curl ran in. Only trust the files next to this script
# when it really is one: a checkout, or an unpacked release archive.
here=
if [ -f "$0" ] && [ "$(basename "$0")" = install.sh ]; then
    dir=$(cd "$(dirname "$0")" && pwd)
    if grep -qx 'name = "vinowatch"' "$dir/Cargo.toml" 2>/dev/null ||
        { [ -f "$dir/vinowatch" ] && [ -f "$dir/vinowatch.service" ]; }; then
        here=$dir
    fi
fi

if [ -z "$here" ]; then
    case "$(uname -m)" in
        x86_64) arch=x86_64 ;;
        aarch64 | arm64) arch=aarch64 ;;
        *) echo "No prebuilt binary for $(uname -m). Build from a checkout instead." >&2; exit 1 ;;
    esac
    version=${VINOWATCH_VERSION:-latest}
    case "$version" in
        *[!A-Za-z0-9.+-]*) echo "VINOWATCH_VERSION=$version isn't a release tag like v0.1.0." >&2; exit 1 ;;
        [0-9]*) version="v$version" ;;
    esac
    if [ "$version" = latest ]; then
        url="https://github.com/$repo/releases/latest/download"
    else
        url="https://github.com/$repo/releases/download/$version"
    fi
    tarball="vinowatch-$arch-linux.tar.gz"
    tmp=$(mktemp -d)
    trap 'rm -rf "$tmp"' EXIT
    echo "Downloading vinowatch $version for $arch..."
    curl -fsSL -o "$tmp/$tarball" "$url/$tarball"
    if ! curl -fsSL -o "$tmp/SHA256SUMS" "$url/SHA256SUMS"; then
        echo "Couldn't download SHA256SUMS for vinowatch $version, so the download can't be verified." >&2
        echo "Pick another release with VINOWATCH_VERSION, or build from a checkout with ./install.sh." >&2
        exit 1
    fi
    count=$(awk -v f="$tarball" '$2 == f { n++ } END { print n + 0 }' "$tmp/SHA256SUMS")
    if [ "$count" -ne 1 ]; then
        echo "SHA256SUMS lists $tarball $count times, expected once." >&2
        exit 1
    fi
    if ! (cd "$tmp" && awk -v f="$tarball" '$2 == f' SHA256SUMS | sha256sum -c --strict --quiet -); then
        echo "$tarball doesn't match its SHA256SUMS entry. Not installing it." >&2
        exit 1
    fi
    tar -xzf "$tmp/$tarball" -C "$tmp"
    here="$tmp/vinowatch"
fi

# Release archives ship the binary. A checkout has Cargo.toml instead.
if [ -f "$here/Cargo.toml" ]; then
    cargo build --release --locked --manifest-path "$here/Cargo.toml"
    built="$here/target/release/vinowatch"
else
    built="$here/vinowatch"
fi

# install(1) replaces the file in place, including an old symlink, and a
# running tray keeps its old inode until the restart below.
install -Dm755 "$built" "$bin"
install -Dm644 "$here/vinowatch.service" "$unit_dir/vinowatch.service"
# The tray draws its own icons too, so a build without this flag (an older
# release, or --no-default-features) still shows one.
if ! "$bin" --install-icons "$data_dir" >/dev/null 2>&1; then
    echo "This vinowatch can't install its icons; the tray will draw its own."
fi
# Plasma keeps icons it has looked up in memory, so an updated icon wouldn't
# show until the next login. This is the signal an icon theme change sends.
dbus-send --session --type=signal /KIconLoader org.kde.KIconLoader.iconChanged int32:0 \
    2>/dev/null || true

if systemctl --user daemon-reload 2>/dev/null; then
    systemctl --user enable vinowatch.service
    # Only start it when a graphical session is up; otherwise it starts at the next login.
    if systemctl --user is-active --quiet graphical-session.target; then
        systemctl --user restart vinowatch.service
        echo "Installed. The tray starts with your graphical session."
    elif [ -n "${WAYLAND_DISPLAY:-}" ]; then
        # A Wayland session that never starts graphical-session.target: sway or
        # Hyprland launched without uwsm or sway-systemd. The unit's WantedBy
        # never fires there, so start it now and say how to make it stick.
        systemctl --user restart vinowatch.service
        echo "Installed and started. Your session doesn't start graphical-session.target,"
        echo "so add this to your compositor's config to start the tray at each login:"
        case "${XDG_CURRENT_DESKTOP:-}" in
            *Hyprland*) echo "  exec-once = systemctl --user start vinowatch.service" ;;
            *sway*) echo "  exec systemctl --user start vinowatch.service" ;;
            *) echo "  systemctl --user start vinowatch.service" ;;
        esac
    else
        echo "Installed. The tray starts with your graphical session."
    fi
else
    echo "Installed $bin. No systemd user session here, so run vinowatch yourself."
fi
