//! Two documentation conventions this crate states, made mechanical.
//!
//! Neither test judges prose. Each holds one property a machine can decide, and
//! its failure message says what to do instead. Sources are normalised because
//! a Windows checkout hands out `\r\n`, which every pattern here would miss.

use std::path::{Path, PathBuf};

/// Every `.rs` file under `src/`, plus the `.rs` files under `tests/`.
fn sources(dir: &str) -> Vec<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join(dir);
    let mut paths: Vec<PathBuf> = std::fs::read_dir(&root)
        .unwrap_or_else(|e| panic!("{} is readable: {e}", root.display()))
        .map(|entry| entry.expect("readable entry").path())
        .filter(|path| path.extension().is_some_and(|e| e == "rs"))
        .collect();
    paths.sort();
    paths
}

/// The site's own pages, and the README.
fn prose_sources() -> Vec<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut paths: Vec<PathBuf> = std::fs::read_dir(root.join("site/content/docs"))
        .expect("the site's docs are readable")
        .map(|entry| entry.expect("readable entry").path())
        .filter(|path| path.extension().is_some_and(|e| e == "md"))
        .collect();
    paths.push(root.join("README.md"));
    paths.sort();
    paths
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("{} is readable: {e}", path.display()))
        .replace("\r\n", "\n")
}

fn name(path: &Path) -> String {
    path.file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned()
}

/// The longest run of `///` lines allowed on one item.
///
/// `//!` is exempt: a module doc is where a module explains itself once instead
/// of on each of its functions.
const MAX_ITEM_DOC_LINES: usize = 60;

/// No item's documentation grows past a screenful.
///
/// An API doc is read next to the item it documents; a block long enough to
/// bury the next one belongs in that module's guide, where one explanation
/// stays in one place. A second worked example is the usual cause.
#[test]
fn no_item_doc_grows_into_a_chapter() {
    let mut long: Vec<String> = Vec::new();
    let mut checked = 0usize;

    for path in sources("src") {
        let text = read(&path);
        let mut run = 0usize;
        let mut start = 0usize;

        for (index, line) in text.lines().enumerate() {
            if line.trim_start().starts_with("///") {
                if run == 0 {
                    start = index + 1;
                }
                run += 1;
                continue;
            }
            if run > 0 {
                checked += 1;
                if run > MAX_ITEM_DOC_LINES {
                    long.push(format!("{}:{start} — {run} lines", name(&path)));
                }
            }
            run = 0;
        }
    }

    assert!(
        checked > 500,
        "the scan found only {checked} doc blocks — it has stopped working, not the crate",
    );
    assert!(
        long.is_empty(),
        "item docs longer than {MAX_ITEM_DOC_LINES} lines. Move the long-form \
         explanation to the guide for that module and link to it; a second \
         worked example is the usual cause: {long:#?}",
    );
}

/// Phrases that describe a *past* state of this crate rather than its design.
///
/// Each is specific enough that an ordinary sentence does not contain it. Bare
/// `used to` is absent on purpose — *"only used to derive a ceiling"* is
/// ordinary English.
const HISTORICAL: &[&str] = &[
    "it used to",
    "which used to",
    "that used to",
    "this used to",
    "used to be",
    "an earlier version",
    "a previous version",
    "the previous implementation",
    "in an earlier release",
    "before this change",
    "prior to this",
    "the bug that",
    "the bug this",
    "was a bug",
    "this was false",
    "was not true",
    "has been renamed",
    "was renamed",
    "was removed in",
    "was added in",
    "hid behind",
    "no test caught",
    "had claimed",
    "used to say",
    "used to return",
    "went unnoticed",
];

/// Version numbers, which date a document that is meant to describe the present.
const VERSIONED: &[&str] = &["as of 0.", "since 0.", "introduced in 0.", "changed in 0."];

/// Reference documentation describes the current design and its rationale.
///
/// *"X is done this way because Y would be wrong"* earns its place; *"X used to
/// be Y"* is a changelog entry that escaped into a reference. History goes in
/// `CHANGELOG.md`.
///
/// A test's own doc comment follows the same rule: state the property it
/// guards, in the present tense, not the defect it was written for.
#[test]
fn reference_docs_are_not_a_changelog() {
    let mut found: Vec<String> = Vec::new();
    let mut checked = 0usize;

    let rust: Vec<PathBuf> = sources("src")
        .into_iter()
        .chain(sources("tests"))
        // This file quotes the phrases it bans, and has to.
        .filter(|path| name(path) != "doc_conventions.rs")
        .collect();

    for path in rust {
        // A release number earns its place in exactly one file: the one that
        // locks the `serde` wire format, where *which release a tag arrived in*
        // is the fact being recorded rather than a stale annotation on a
        // description of the present.
        let versioned = name(&path) != "serde_representation.rs";
        for (index, line) in read(&path).lines().enumerate() {
            let trimmed = line.trim_start();
            if !trimmed.starts_with("///") && !trimmed.starts_with("//!") {
                continue;
            }
            checked += 1;
            let lower = trimmed.to_lowercase();
            let extra: &[&str] = if versioned { VERSIONED } else { &[] };
            let phrases = HISTORICAL.iter().chain(extra);
            for phrase in phrases {
                if lower.contains(phrase) {
                    found.push(format!("{}:{}: {}", name(&path), index + 1, trimmed.trim()));
                }
            }
        }
    }

    for path in prose_sources() {
        for (index, line) in read(&path).lines().enumerate() {
            checked += 1;
            let lower = line.to_lowercase();
            for phrase in HISTORICAL.iter().chain(VERSIONED) {
                if lower.contains(phrase) {
                    found.push(format!("{}:{}: {}", name(&path), index + 1, line.trim()));
                }
            }
        }
    }

    assert!(
        checked > 5_000,
        "the scan found only {checked} documentation lines — it has stopped working",
    );
    assert!(
        found.is_empty(),
        "documentation describing a past state of the crate. Reference docs \
         describe the current design and why the alternative would be wrong; \
         the history goes in CHANGELOG.md: {found:#?}",
    );
}
