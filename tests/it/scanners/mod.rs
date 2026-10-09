//! Tests that read the repository's own files as data.
//!
//! A convention that lives only in a document holds until the next person
//! writes the natural thing instead. Each scanner here exposes the function
//! that decides a single line or snippet, and each has a test that feeds it a
//! planted violation: a scanner whose pattern stops matching fails there,
//! instead of passing for ever without looking at anything.
//!
//! Paths are resolved from `CARGO_MANIFEST_DIR`, directories are walked
//! recursively, and every text is normalised from `\r\n`, because a Windows
//! checkout hands those out and a pattern split on `\n` would then match
//! nothing.

use std::path::{Path, PathBuf};

mod arithmetic_conventions;
mod code_contract;
mod doc_conventions;
mod internal_references;
mod serde_wire;

/// The crate root.
pub(crate) fn root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

/// Every file under `dir` (relative to the crate root) whose extension is one
/// of `extensions`, recursively, sorted. Directories named in `skip` are not
/// entered.
pub(crate) fn files_under(dir: &str, extensions: &[&str], skip: &[&str]) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    let mut dirs = vec![root().join(dir)];
    while let Some(dir) = dirs.pop() {
        let entries = std::fs::read_dir(&dir)
            .unwrap_or_else(|e| panic!("{} is readable: {e}", dir.display()));
        for entry in entries {
            let path = entry.expect("readable entry").path();
            let name = path.file_name().unwrap_or_default().to_string_lossy();
            if path.is_dir() {
                if !skip.iter().any(|s| *s == name) {
                    dirs.push(path);
                }
            } else if path
                .extension()
                .is_some_and(|e| extensions.iter().any(|x| e == *x))
            {
                paths.push(path);
            }
        }
    }
    paths.sort();
    paths
}

/// A file's text, line endings normalised.
pub(crate) fn read(path: &Path) -> String {
    std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("{} is readable: {e}", path.display()))
        .replace("\r\n", "\n")
}

/// A path relative to the crate root, with `/` separators.
pub(crate) fn relative(path: &Path) -> String {
    path.strip_prefix(root())
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

/// Every `.rs` file under `src/`, as `(path relative to src/, text)`.
///
/// Panics when the walk finds nothing: an empty scan passes every check.
pub(crate) fn src_files() -> Vec<(String, String)> {
    let out: Vec<(String, String)> = files_under("src", &["rs"], &[])
        .into_iter()
        .map(|path| {
            let name = relative(&path)
                .strip_prefix("src/")
                .map(str::to_owned)
                .unwrap_or_default();
            (name, read(&path))
        })
        .collect();
    assert!(
        out.len() > 20,
        "the scan found only {} files under src/ — it has stopped working, not the crate",
        out.len()
    );
    out
}

/// The scans themselves see the crate.
#[test]
fn the_walk_finds_the_nested_modules() {
    let files = src_files();
    // Every area of the crate lives in a subdirectory, so a walk that does not
    // recurse sees only `lib.rs` and its siblings.
    assert!(
        files.iter().any(|(name, _)| name.contains('/')),
        "the walk does not descend into src/'s subdirectories"
    );
    assert!(files.iter().any(|(name, _)| name == "lib.rs"));
}

/// Every scanner proves it bites: a scanner file without a planted-violation
/// test is a check nobody has seen fail.
#[test]
fn every_scanner_has_a_planted_violation_test() {
    let scanners = files_under("tests/it/scanners", &["rs"], &[]);
    let mut checked = 0;
    for path in scanners {
        let name = relative(&path);
        if name.ends_with("mod.rs") {
            continue;
        }
        let text = read(&path);
        let planted = text
            .lines()
            .any(|l| l.trim_start().starts_with("fn ") && l.contains("planted"));
        assert!(planted, "{name} has no `…planted…` test fn");
        checked += 1;
    }
    assert!(checked >= 5, "found only {checked} scanner files");
}
