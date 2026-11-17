//! Every `` `tests::name` `` a comment cites must be a `#[test]` fn, so a rule
//! that names its test fails CI when the test is renamed or deleted. rustdoc
//! can't check these: `#[test]` fns are compiled out of the docs. See
//! docs/comment-style.md.

use std::fs;
use std::path::{Path, PathBuf};

#[test]
fn every_cited_test_exists() {
    let mut broken = Vec::new();
    for file in rust_files(Path::new("src")) {
        let text = fs::read_to_string(&file).unwrap();
        for (line_no, line) in text.lines().enumerate() {
            if !line.trim_start().starts_with("//") {
                continue;
            }
            for cited in cited_tests(line) {
                if !resolves(&text, cited) {
                    broken.push(format!("{}:{}: `{cited}`", file.display(), line_no + 1));
                }
            }
        }
    }
    assert!(broken.is_empty(), "no such test:\n{}", broken.join("\n"));
}

#[test]
fn the_check_finds_a_cited_test_and_rejects_a_missing_one() {
    let text = "/// (pinned by `tests::real`)\n#[cfg(test)]\nmod tests {\n    #[test]\n    fn real() {}\n    fn helper() {}\n}\n";
    assert_eq!(cited_tests(text.lines().next().unwrap()), ["tests::real"]);
    assert!(resolves(text, "tests::real"));
    assert!(!resolves(text, "tests::helper"), "not a #[test]");
    assert!(!resolves(text, "tests::gone"));
}

#[test]
fn an_integration_test_is_cited_by_its_file() {
    assert!(resolves("", "tests/cli.rs::a_second_tray_exits_cleanly"));
    assert!(!resolves("", "tests/cli.rs::gone"));
    assert!(!resolves("", "tests/missing.rs::anything"));
}

fn rust_files(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for entry in fs::read_dir(dir).unwrap().flatten() {
        let path = entry.path();
        if path.is_dir() {
            files.extend(rust_files(&path));
        } else if path.extension().is_some_and(|e| e == "rs") {
            files.push(path);
        }
    }
    files
}

/// Code spans like `` `tests::name` `` or `` `module::tests::name` ``.
fn cited_tests(line: &str) -> Vec<&str> {
    line.split('`')
        .skip(1)
        .step_by(2)
        .filter(|span| {
            (span.contains("tests::") || span.starts_with("tests/"))
                && span.contains("::")
                && span
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | ':' | '/' | '.'))
        })
        .collect()
}

/// `tests::name` is in `text`; `module::tests::name` in `src/module.rs` or `src/module/mod.rs`;
/// `tests/file.rs::name` in that integration test.
fn resolves(text: &str, cited: &str) -> bool {
    if let Some((file, name)) = cited
        .strip_prefix("tests/")
        .and_then(|c| c.split_once("::"))
    {
        return fs::read_to_string(Path::new("tests").join(file))
            .is_ok_and(|t| is_test_fn(&t, name));
    }
    let (module, name) = cited.rsplit_once("tests::").unwrap();
    let module = module.trim_end_matches("::");
    if module.is_empty() {
        return is_test_fn(text, name);
    }
    let base = Path::new("src").join(module.replace("::", "/"));
    [base.with_extension("rs"), base.join("mod.rs")]
        .iter()
        .filter_map(|file| fs::read_to_string(file).ok())
        .any(|text| is_test_fn(&text, name))
}

/// A `fn name(` whose attributes, above it, include `#[test]`.
fn is_test_fn(text: &str, name: &str) -> bool {
    let lines: Vec<&str> = text.lines().collect();
    let signature = format!("fn {name}(");
    lines.iter().enumerate().any(|(i, line)| {
        line.trim_start().starts_with(&signature)
            && lines[..i]
                .iter()
                .rev()
                .map(|l| l.trim())
                .take_while(|l| l.starts_with("#[") || l.starts_with("///"))
                .any(|l| l == "#[test]")
    })
}
