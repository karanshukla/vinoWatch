# Comment style

A comment is the last resort. Work down this ladder, and write prose only
when all four rungs fail.

1. **Extract it.** Logic that a comment explains belongs in a function with
   a domain name. Arithmetic spelled out in a comment belongs in named
   constants. A coercion that is explained twice belongs in one named helper.

   ```rust
   // Before
   let share = |(l, a): (u64, u64)| a as f64 / l.max(1) as f64; // in two files
   if meta.is_dir() && mode & 0o1000 != 0 { // like /tmp
   let python = (path, uid); // `.0` is the path, `.1` who owns it

   // After
   if memory.tighter_than(other) {
   if meta.is_dir() && mode & STICKY != 0 {
   let sampler = Sampler::started(warn_below); // not `new` plus a baseline poll
   ```

2. **Name it.** A comment that labels a block (`// Phase 2: cache lookup`)
   means the block wants to be a function. Sentinels get names too: write
   `const NEVER_WARN: u8 = 0`, not `0 // turns it off`. A variant whose comment says
   it means more than its name (`LowMemory // also a stuck driver`) gets
   renamed (`Warning`).

3. **Pin it with a test pair.** A comment that states a business rule is a
   rule nothing enforces. Write one test inside the boundary and one
   outside, named after the rule (`warns_just_under_the_threshold` /
   `does_not_warn_at_the_threshold`), instead of writing `// exactly at the
   threshold is fine`. The pair is the point: it fails the day someone moves
   the limit. In a test, a comment that labels the next few asserts is the
   same smell. Split those asserts into their own test, named after what the
   comment said.

4. **Pin cross-boundary rules with an end-to-end test.** The `--json`
   shape, exit codes, and what `install.sh` writes belong in `tests/cli.rs`
   or `tests/install-e2e.sh`, not in a paragraph.

5. **Only then, prose.** Hidden constraints, upstream bugs, incident
   history that still constrains the code, protocol requirements. State the
   constraint, not its biography: "zbus stays on ksni's major" is enough
   without the story of how we found out. Measured claims carry a date.

## Linking the test from the code

When rung 3 or 4 replaced a comment, the code it governs says which test
pins the rule, in its doc comment:

```rust
/// Under the `--warn-below` share of the total is available (pinned by
/// `tests::warns_just_under_the_threshold`, `tests::does_not_warn_at_the_threshold`).
```

An integration test is cited by its file, `` `tests/cli.rs::a_second_tray_exits_cleanly` ``.

rustdoc can't link to `#[test]` functions (they are compiled out even
under `--cfg test`), so these are code spans, not intra-doc links.
`tests/doc_links.rs` (CI job "Doc Links") checks them instead: every
`` `tests::name` `` or `` `module::tests::name` `` after "pinned by" must be a
`#[test]` fn in that module's file (`src/module.rs` or `src/module/mod.rs`).

## Exempt

- `///` and `//!` docs on items, which say what an item is and how to call
  it. They follow the same "constraint, not biography" rule, but aren't
  replaced by code.
- `// SAFETY:` comments on `unsafe`, `#[allow]` reasons, and coverage
  pragmas.
