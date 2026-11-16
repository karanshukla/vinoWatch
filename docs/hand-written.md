# What is hand-written, and what a crate costs

Three pieces of vinoWatch could be a crate, and two now are. Each was tried on 2026-11-15, as a release build (`lto`, one codegen unit, stripped) for `x86_64-unknown-linux-gnu` in a cloud container. Sizes are `target/release/vinowatch` in bytes.

| Piece | Crate | Outcome | Tray build | Console-only build | Crates |
|---|---|---|---|---|---|
| `--json` (`sample.rs`) | `serde_json` | taken | 3,285,880 → 3,302,088 (+16,208) | 630,088 → 641,184 (+11,096) | +3 in the lock (`serde_json`, `itoa`, `zmij`) |
| PNG encoder (`png.rs`) | `png` | taken 2026-11-16 | 3,302,088 → 3,388,968 (+86,880) | no PNG there | +9 in the binary (73 → 82) |
| arguments (`cli.rs`) | `lexopt` | not taken | 3,302,088 → 3,308,448 (+6,360) | 641,184 → 651,888 (+10,704) | +1 |

## `--json`: serde_json

The bytes did not change. `WireFormat` in `sample.rs` is a `serde_json` formatter for the two places the output differs from the crate's defaults: floats have one decimal, and control characters are written as `\u00XX` (the crate would write `\n` and `\t`). `snapshot_json_shape`, `json_numbers_have_one_decimal_and_sizes_are_whole_mebibytes` and `json_names_keep_their_escapes` pin it.

## PNG: the png crate

The bytes did not change. `png.rs` asks the crate for `Compression::NoCompression` and `Filter::NoFilter`, which is what the hand-written encoder wrote: 8-bit RGBA in stored deflate blocks. Both were run side by side at 1x1, 2x1, 16x16, 48x48, 200x100 and 300x300 before the old one was deleted (2026-11-16), and the output matched byte for byte. A 48 px icon is still 9 KiB.

The cost is the row above: nine crates in a binary that runs for the whole session (two versions of `miniz_oxide` among them) and 87 KiB in place of sixty lines. The encoder was kept hand-written on 2026-11-15 for that reason, and swapped the next day at the owner's request.

`an_image_is_signature_header_data_end` and `rows_over_one_stored_block_split_into_several` pin the layout, and `a_standard_decoder_reads_back_the_same_pixels` decodes it again, which is all a dbusmenu host needs.

## Arguments: the parser stays

`lexopt` passes thirteen of the seventeen messages `every_error_says_what_was_wrong` pins. The other four are forms it accepts or words differently, and each is a change to the command line:

| Input | Today | With `lexopt` |
|---|---|---|
| `-hV` | `unknown option -hV` | shows the help |
| `--` | `unknown option --` | accepted, runs |
| `--warn-below=25` | `unknown option --warn-below=25` | accepted as 25 |
| `--json=1` | `unknown option --json=1` | `unexpected argument for option '--json': "1"` |

`clap` was not built: it writes its own help, and `the_usage_text_is_pinned` holds the text byte for byte.

Smaller ones, not measured: `driver::wait_timeout` (eight lines, tested by `a_hung_probe_times_out`) against the `wait-timeout` crate, and the tray's `markup_escape` (three replacements).
