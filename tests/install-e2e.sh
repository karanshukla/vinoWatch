#!/bin/sh
# End-to-end tests for install.sh's release download: checksum checks,
# VINOWATCH_VERSION handling, the release-archive branch and --uninstall.
#
# Usage: tests/install-e2e.sh path/to/vinowatch
#
# Nothing is downloaded and nothing touches the real home: a fake curl on
# PATH serves files from a local directory (and logs the URLs asked for), a
# fake systemctl fails like a machine with no user session, and every run
# gets its own HOME.
set -eu

[ $# -eq 1 ] && [ -x "$1" ] || { echo "usage: $0 path/to/vinowatch" >&2; exit 2; }
binary=$(cd "$(dirname "$1")" && pwd)/$(basename "$1")
repo=$(cd "$(dirname "$0")/.." && pwd)
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

case "$(uname -m)" in
    x86_64) arch=x86_64 ;;
    aarch64 | arm64) arch=aarch64 ;;
    *) echo "no release for $(uname -m)" >&2; exit 2 ;;
esac
tarball="vinowatch-$arch-linux.tar.gz"

# A release laid out as release.yml packs it.
mkdir -p "$work/pack/vinowatch" "$work/release" "$work/fakebin"
cp "$binary" "$work/pack/vinowatch/vinowatch"
cp "$repo/vinowatch.service" "$repo/install.sh" "$repo/install-npu-reset.sh" \
    "$repo/vino-npu-reset.service" "$repo/vino-npu-reset.rules" "$work/pack/vinowatch/"
tar -czf "$work/good.tar.gz" -C "$work/pack" vinowatch
(cd "$work" && cp good.tar.gz "$tarball" && sha256sum "$tarball" > good.sums && rm "$tarball")

cat > "$work/fakebin/curl" <<'EOF'
#!/bin/sh
# curl -fsSL -o OUT URL, served from $FAKE_RELEASE.
out=$3 url=$4
echo "$url" >> "$FAKE_RELEASE/urls"
[ -f "$FAKE_RELEASE/$(basename "$url")" ] || exit 22
cp "$FAKE_RELEASE/$(basename "$url")" "$out"
EOF
printf '#!/bin/sh\nexit 1\n' > "$work/fakebin/systemctl"
chmod +x "$work/fakebin/curl" "$work/fakebin/systemctl"

failures=0
fail() { echo "FAIL: $*" >&2; failures=$((failures + 1)); }

# serve TARBALL SUMS: what the fake release holds. "-" leaves a file out.
serve() {
    rm -rf "$work/release" && mkdir -p "$work/release"
    [ "$1" = - ] || cp "$1" "$work/release/$tarball"
    [ "$2" = - ] || cp "$2" "$work/release/SHA256SUMS"
}

# run [ENV=VALUE...] -- ARGS: install.sh piped into sh, as the README says.
run() {
    home=$(mktemp -d "$work/home.XXXXXX")
    : > "$work/release/urls"
    set +e
    env -i PATH="$work/fakebin:/usr/bin:/bin" HOME="$home" FAKE_RELEASE="$work/release" \
        "$@" sh -s < "$repo/install.sh" > "$work/out" 2>&1
    status=$?
    set -e
}

expect_refused() {
    what=$1 message=$2
    [ "$status" -eq 1 ] || fail "$what: exit $status, wanted 1"
    grep -q "$message" "$work/out" || fail "$what: no '$message' in: $(cat "$work/out")"
    [ ! -e "$home/.local/bin/vinowatch" ] || fail "$what: installed anyway"
}

icons="scalable/apps/vinowatch.svg symbolic/apps/vinowatch-symbolic.svg
symbolic/apps/vinowatch-active-symbolic.svg symbolic/apps/vinowatch-low-memory-symbolic.svg"

# A good release installs the binary, the unit and the icons.
serve "$work/good.tar.gz" "$work/good.sums"
run
[ "$status" -eq 0 ] || fail "good release: exit $status: $(cat "$work/out")"
"$home/.local/bin/vinowatch" --version | grep -q '^vinowatch ' || fail "installed binary doesn't run"
[ -f "$home/.config/systemd/user/vinowatch.service" ] || fail "no unit installed"
for icon in $icons; do
    [ -f "$home/.local/share/icons/hicolor/$icon" ] || fail "no $icon"
done
grep -qx "https://github.com/karanshukla/vinoWatch/releases/latest/download/$tarball" \
    "$work/release/urls" || fail "latest URL: $(cat "$work/release/urls")"
grep -q "No systemd user session" "$work/out" || fail "no-session message missing"

# --uninstall removes every file it installed.
set +e
env -i PATH="$work/fakebin:/usr/bin:/bin" HOME="$home" sh "$repo/install.sh" --uninstall > "$work/out" 2>&1
status=$?
set -e
[ "$status" -eq 0 ] || fail "uninstall: exit $status"
left=$(find "$home" -type f)
[ -z "$left" ] || fail "uninstall left: $left"

# A version picks that release, with or without the v.
run VINOWATCH_VERSION=0.3.0
[ "$status" -eq 0 ] || fail "VINOWATCH_VERSION=0.3.0: exit $status"
grep -qx "https://github.com/karanshukla/vinoWatch/releases/download/v0.3.0/$tarball" \
    "$work/release/urls" || fail "version URL: $(cat "$work/release/urls")"

# Anything that isn't tag-shaped never reaches a URL.
run VINOWATCH_VERSION='v1;x'
expect_refused "VINOWATCH_VERSION='v1;x'" "isn't a release tag"
[ ! -s "$work/release/urls" ] || fail "fetched with a bad version"
run VINOWATCH_VERSION='../../x'
expect_refused "VINOWATCH_VERSION=../../x" "isn't a release tag"

# A tarball that doesn't match its checksum.
printf 'not the release' > "$work/bad.tar.gz"
serve "$work/bad.tar.gz" "$work/good.sums"
run
expect_refused "corrupt tarball" "doesn't match its SHA256SUMS entry"

# No SHA256SUMS at all.
serve "$work/good.tar.gz" -
run
expect_refused "no SHA256SUMS" "can't be verified"

# Two entries for the tarball, so the check can't tell which one counts.
cat "$work/good.sums" "$work/good.sums" > "$work/dup.sums"
serve "$work/good.tar.gz" "$work/dup.sums"
run
expect_refused "duplicate entry" "expected once"

# An entry for some other file only.
sed "s/$tarball/other.tar.gz/" "$work/good.sums" > "$work/other.sums"
serve "$work/good.tar.gz" "$work/other.sums"
run
expect_refused "no entry" "0 times"

# Run from an unpacked release archive: installs what's next to it.
home=$(mktemp -d "$work/home.XXXXXX")
mkdir -p "$work/unpacked" && tar -xzf "$work/good.tar.gz" -C "$work/unpacked"
set +e
env -i PATH="$work/fakebin:/usr/bin:/bin" HOME="$home" FAKE_RELEASE="$work/release" \
    sh "$work/unpacked/vinowatch/install.sh" > "$work/out" 2>&1
status=$?
set -e
[ "$status" -eq 0 ] || fail "from the archive: exit $status: $(cat "$work/out")"
cmp -s "$binary" "$home/.local/bin/vinowatch" || fail "from the archive: wrong binary"
! grep -q "Downloading" "$work/out" || fail "from the archive: downloaded anyway"

if [ "$failures" -ne 0 ]; then
    echo "$failures failure(s)" >&2
    exit 1
fi
echo "install.sh: all checks passed"
