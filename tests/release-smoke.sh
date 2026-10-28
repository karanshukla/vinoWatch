#!/bin/sh
# Smoke-tests the release tarballs release.yml builds, before they ship:
# checksums verify, each archive has what install.sh expects, and each
# binary is static and runs (aarch64 under qemu-user).
#
# Usage: tests/release-smoke.sh DIST VERSION
set -eu

[ $# -eq 2 ] || { echo "usage: $0 DIST VERSION" >&2; exit 2; }
dist=$1 version=$2
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

(cd "$dist" && sha256sum -c --strict SHA256SUMS)

for arch in x86_64 aarch64; do
    tarball="$dist/vinowatch-$arch-linux.tar.gz"
    mkdir -p "$work/$arch"
    tar -xzf "$tarball" -C "$work/$arch"
    dir="$work/$arch/vinowatch"
    for f in vinowatch vinowatch.service install.sh install-npu-reset.sh \
        vino-npu-reset.service vino-npu-reset.rules; do
        [ -f "$dir/$f" ] || { echo "$tarball has no $f" >&2; exit 1; }
    done
    info=$(file -b "$dir/vinowatch")
    case "$info" in
        *"static"*) ;;
        *) echo "$arch binary isn't static: $info" >&2; exit 1 ;;
    esac

    if [ "$arch" = "$(uname -m)" ]; then
        run="$dir/vinowatch"
    else
        # Ubuntu's qemu-user is static and drops the -static suffix.
        qemu=$(command -v "qemu-$arch-static" || command -v "qemu-$arch") ||
            { echo "no qemu-$arch" >&2; exit 1; }
        run="$qemu $dir/vinowatch"
    fi
    got=$($run --version)
    [ "$got" = "vinowatch $version" ] || { echo "$arch: '$got', wanted 'vinowatch $version'" >&2; exit 1; }
    $run --once --json 1 | jq -e '.processes | type == "array"' >/dev/null
    $run --install-icons "$work/$arch/icons" >/dev/null
    [ "$(find "$work/$arch/icons" -name '*.svg' | wc -l)" -eq 4 ]
    echo "$arch: ok"
done
