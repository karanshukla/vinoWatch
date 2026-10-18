# Contributing

vinoWatch was written on a machine with no Intel NPU, so the most useful thing you can bring is a report from one: does the `NPU` figure move when you run vinoWhisper or vinoSpeak on the NPU, and does the `NPU open` marker land on the right process? Attach `vinowatch --json --once` taken while the app runs.

## Setup

Plain cargo. Nothing needs OpenVINO or an NPU; the tests build a fake `/proc` and a fake sysfs.

```bash
cargo test
cargo test --no-default-features   # the console-only build
```

To see it against the real thing, run any vino app that loads OpenVINO (for example a script that does `import openvino; openvino.Core()` from one of their venvs) and `cargo run -- --console 1`.

## Before you push

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo clippy --no-default-features --all-targets -- -D warnings
cargo test
cargo deny check
```

- A change to how processes are found or named needs a test with a fake `/proc` entry (see `src/detect.rs`).
- A change that can only be verified on an NPU: say so in the PR instead of faking a test.
- Keep code comments to the *why*, one or two lines. Measured claims carry a date.
- Commits are conventional (`feat:`, `fix:`, `docs:`, ...); the changelog is generated from them with `git cliff`.
- Every Actions `uses:` is pinned to a commit SHA with a version comment.
- A `deny.toml` advisory ignore needs a written reason next to it.

## Adding another vino app

Add its marker to the `MARKERS` table in `src/detect.rs`, a variant to `Project`, a row to the README table, and a case to `names_each_project_from_its_command_line`. Match on whatever appears in its command line; check with `ps -eo pid,args` while it runs.

## Releases

Versions follow Cargo.toml. Bump it, run `git cliff -c cliff.toml --tag vX.Y.Z -o CHANGELOG.md`, commit, then push the tag `vX.Y.Z`. `release.yml` refuses a tag that doesn't match Cargo.toml, builds static x86_64 and aarch64 binaries, and publishes them with `SHA256SUMS`.
