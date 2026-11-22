# Contributing

`vinoWatch` was developed on a machine without an Intel NPU, so real-world NPU hardware reports are especially helpful. If you have an Intel NPU, please let us know whether the `NPU` percentage updates while running `vinoWhisper` or `vinoSpeak` on the NPU and whether the `NPU open` marker appears on the expected process, ideally with the output of `vinowatch --json --once` captured while the app is active.

## Development setup

`vinoWatch` uses a standard Cargo workflow. Running the test suite does not require OpenVINO or an Intel NPU; tests operate against simulated `/proc` and sysfs trees.

```bash
cargo test
cargo test --no-default-features   # console-only build
```

To test against live processes, run any `vino*` app that loads OpenVINO (or a small script that executes `import openvino; openvino.Core()` inside one of their virtual environments) alongside `cargo run -- --console 1`.

## Before pushing

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo clippy --no-default-features --all-targets -- -D warnings
cargo test
cargo deny check
```

If you modified `install.sh`, also run the end-to-end installer test (which runs offline without touching your real `$HOME`):

```bash
cargo build && tests/install-e2e.sh target/debug/vinowatch
```

CI additionally builds and tests the release tarballs on every pull request and enforces the `cargo llvm-cov` line coverage floor configured in `ci.yml`.

## Guidelines

- `tests/cli.rs` defines the CLI contract for scripts (exit codes and `--json` schema). Update it whenever those change and document the change in the changelog.
- Any change to process discovery or app naming requires a unit test with a simulated `/proc` entry (see `src/detect.rs` and `fake_openvino_process` in `src/sample.rs`).
- If a change can only be verified on real NPU hardware, note that clearly in the pull request rather than adding a mock test.
- Keep comments minimal: prefer extracting functions, using clear names, or pinning invariants with paired tests, and cite the governing test in comments. See [docs/comment-style.md](docs/comment-style.md); `tests/doc_links.rs` validates citations. Record performance or size measurements in `docs/` with their date.
- Before replacing hand-written code with an external crate, review [docs/hand-written.md](docs/hand-written.md) for the measured size and dependency trade-offs.
- Use Conventional Commits (`feat:`, `fix:`, `docs:`, etc.); `CHANGELOG.md` is generated with `git cliff`.
- Pin every GitHub Actions `uses:` reference to a full commit SHA with a version comment.
- Include a written rationale next to every advisory ignore in `deny.toml`.

## Adding another vino app

1. Add the app's matching marker to the `MARKERS` table in `src/detect.rs` and add a variant to `Project`. Match against what appears in the process command line (inspect with `ps -eo pid,args` while the app is running).
2. Add a row to the recognized-applications table in `README.md`.
3. Add a test case to `names_each_project_from_its_command_line`.

## Releases

Versions follow `Cargo.toml`. To cut a release:

1. Bump the version in `Cargo.toml`.
2. Regenerate the changelog: `git cliff -c cliff.toml --tag vX.Y.Z -o CHANGELOG.md`.
3. Commit the changes and push the tag `vX.Y.Z`.

`release.yml` validates that the tag matches `Cargo.toml`, points to a commit on `main`, and has a matching `## [X.Y.Z]` section in `CHANGELOG.md` (used as the release notes). It then builds static `x86_64` and `aarch64` binaries and publishes them alongside `SHA256SUMS`.
