# Hand-written code vs. external crates

Three components in `vinoWatch` were evaluated against external crates, and two were ultimately replaced. Each replacement was benchmarked on 2026-11-15 using release builds (`lto`, single codegen unit, stripped) targeting `x86_64-unknown-linux-gnu` inside a cloud container. Binary sizes measure `target/release/vinowatch` in bytes.

| Component | Crate | Decision | Tray build | Console-only build | Dependency impact |
|---|---|---|---|---|---|
| `--json` (`sample.rs`) | `serde_json` | Adopted | 3,285,880 → 3,302,088 (+16,208) | 630,088 → 641,184 (+11,096) | +3 crates in `Cargo.lock` (`serde_json`, `itoa`, `zmij`) |
| PNG encoder (`png.rs`) | `png` | Adopted (2026-11-16) | 3,302,088 → 3,388,968 (+86,880) | N/A (not compiled) | +9 crates in binary (73 → 82) |
| CLI arguments (`cli.rs`) | `lexopt` | Kept hand-written | 3,302,088 → 3,308,448 (+6,360) | 641,184 → 651,888 (+10,704) | +1 crate |

## `--json`: `serde_json`

The output format remained byte-for-byte identical. `WireFormat` in `sample.rs` implements a custom `serde_json` formatter for the two places where output differs from `serde_json`'s defaults: floating-point numbers always have one decimal place, and control characters are escaped as `\u00XX` (whereas `serde_json` defaults to `\n` and `\t`). This behavior is pinned by `snapshot_json_shape`, `json_numbers_have_one_decimal_and_sizes_are_whole_mebibytes`, and `json_names_keep_their_escapes`.

## PNG encoder: `png`

The output remained byte-for-byte identical. `png.rs` configures the crate with `Compression::NoCompression` and `Filter::NoFilter`, matching the previous hand-written encoder's 8-bit RGBA stored deflate blocks. Both encoders were compared across `1x1`, `2x1`, `16x16`, `48x48`, `200x100`, and `300x300` images before removing the hand-written implementation on 2026-11-16, with identical output bytes. A 48 px icon remains 9 KiB.

The trade-off was 9 additional crates in a long-running session binary (including two versions of `miniz_oxide`) and 87 KiB of binary size to replace ~60 lines of code. For that reason it was initially kept hand-written on 2026-11-15, then switched to `png` the next day at the maintainer's request.

`an_image_is_signature_header_data_end` and `rows_over_one_stored_block_split_into_several` pin the chunk layout, and `a_standard_decoder_reads_back_the_same_pixels` verifies round-trip decoding for `dbusmenu` hosts.

## CLI arguments: kept hand-written

`lexopt` matched 13 of the 17 error messages pinned by `every_error_says_what_was_wrong`. The remaining four cases change CLI parsing or error wording:

| Input | Current behavior | Behavior with `lexopt` |
|---|---|---|
| `-hV` | `unknown option -hV` | Displays help |
| `--` | `unknown option --` | Accepted, runs normally |
| `--warn-below=25` | `unknown option --warn-below=25` | Accepted as `25` |
| `--json=1` | `unknown option --json=1` | `unexpected argument for option '--json': "1"` |

`clap` was not benchmarked because it generates its own help text, whereas `the_usage_text_is_pinned` pins the help text byte-for-byte.

## Smaller hand-written utilities

Two smaller helpers were not benchmarked against crates:

- `driver::wait_timeout`: 8 lines (tested by `a_hung_probe_times_out`), instead of pulling in `wait-timeout`.
- Tray `markup_escape`: three string replacements.
