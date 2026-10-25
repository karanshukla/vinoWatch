#!/bin/sh
# Builds the static musl binaries and packs dist/ the way a release ships it:
# one tarball per arch plus SHA256SUMS, which install.sh checks downloads
# against. release.yml publishes the result; ci.yml builds and smoke-tests
# the same thing on every PR.
set -eu

# The dependencies are pure Rust, so rust-lld links the arm64 binary
# without a cross toolchain.
export CARGO_TARGET_AARCH64_UNKNOWN_LINUX_MUSL_LINKER=rust-lld
mkdir -p dist
for arch in x86_64 aarch64; do
    cargo build --release --locked --target "$arch-unknown-linux-musl"
    dir="dist/vinowatch"
    rm -rf "$dir" && mkdir -p "$dir"
    cp "target/$arch-unknown-linux-musl/release/vinowatch" vinowatch.service install.sh \
        install-npu-reset.sh vino-npu-reset.service vino-npu-reset.rules "$dir"
    tar -czf "dist/vinowatch-$arch-linux.tar.gz" -C dist vinowatch
done
rm -rf dist/vinowatch
(cd dist && sha256sum vinowatch-*-linux.tar.gz > SHA256SUMS)
