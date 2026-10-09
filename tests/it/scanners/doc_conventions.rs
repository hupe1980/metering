//! Two documentation conventions this crate states, made mechanical.
//!
//! Neither test judges prose. Each holds one property a machine can decide,
//! and its failure message says what to do instead.

use super::{files_under, read, relative, root};

/// The longest run of `///` lines allowed on one item.
///
/// `//!` is exempt: a module doc is where a module explains itself once instead
/// of on each of its functions.
const MAX_ITEM_DOC_LINES: usize = 60;

/// Every `///` block of `text` longer than the limit, as `(first line, length)`,
/// and the number of blocks seen.
fn long_doc_blocks(text: &str) -> (Vec<(usize, usize)>, usize) {
    let mut long = Vec::new();
    let (mut blocks, mut run, mut start) = (0usize, 0usize, 0usize);
    for (index, line) in text.lines().chain(std::iter::once("")).enumerate() {
        if line.trim_start().starts_with("///") {
            if run == 0 {
                start = index + 1;
            }
            run += 1;
            continue;
        }
        if run > 0 {
            blocks += 1;
            if run > MAX_ITEM_DOC_LINES {
                long.push((start, run));
            }
        }
        run = 0;
    }
    (long, blocks)
}

/// No item's documentation grows past a screenful.
///
/// An API doc is read next to the item it documents; a block long enough to
/// bury the next one belongs in that module's guide, where one explanation
/// stays in one place. A second worked example is the usual cause.
#[test]
fn no_item_doc_grows_into_a_chapter() {
    let mut long: Vec<String> = Vec::new();
    let mut checked = 0usize;

    for path in files_under("src", &["rs"], &[]) {
        let (found, blocks) = long_doc_blocks(&read(&path));
        checked += blocks;
        for (start, run) in found {
            long.push(format!("{}:{start} — {run} lines", relative(&path)));
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

/// Whether `line` dates itself by a release of this crate: *since*, *as of*,
/// *added in* and the like, followed by a `0.x` version.
///
/// A release number is a fact about the history, and the history is
/// `CHANGELOG.md`'s. In a reference document it is stale the release after.
fn names_a_release(line: &str) -> bool {
    let lower = line.to_lowercase();
    [
        "as of ",
        "since ",
        "introduced in ",
        "changed in ",
        "added in ",
        "removed in ",
        "new in ",
    ]
    .iter()
    .any(|lead| {
        lower.match_indices(lead).any(|(i, _)| {
            let rest = &lower[i + lead.len()..];
            let rest = rest
                .strip_prefix('v')
                .or_else(|| rest.strip_prefix("release "))
                .or_else(|| rest.strip_prefix("version "))
                .unwrap_or(rest);
            let mut chars = rest.chars();
            chars.next() == Some('0')
                && chars.next() == Some('.')
                && chars.next().is_some_and(|c| c.is_ascii_digit())
        })
    })
}

/// Reference documentation describes the design as it is, not by the release
/// it arrived in.
///
/// Doc comments under `src/` and `tests/`, the site's pages and the README are
/// all reference. Tests are grouped by module, not by release, so no file
/// needs an exemption.
#[test]
fn reference_docs_name_no_release() {
    let mut found: Vec<String> = Vec::new();
    let mut checked = 0usize;

    let rust = files_under("src", &["rs"], &[])
        .into_iter()
        .chain(files_under("tests", &["rs"], &[]));
    for path in rust {
        for (index, line) in read(&path).lines().enumerate() {
            let trimmed = line.trim_start();
            if !trimmed.starts_with("///") && !trimmed.starts_with("//!") {
                continue;
            }
            checked += 1;
            if names_a_release(trimmed) {
                found.push(format!("{}:{}: {}", relative(&path), index + 1, trimmed));
            }
        }
    }

    let prose = files_under("site/content", &["md"], &[])
        .into_iter()
        .chain(std::iter::once(root().join("README.md")));
    for path in prose {
        for (index, line) in read(&path).lines().enumerate() {
            checked += 1;
            if names_a_release(line) {
                found.push(format!(
                    "{}:{}: {}",
                    relative(&path),
                    index + 1,
                    line.trim()
                ));
            }
        }
    }

    assert!(
        checked > 5_000,
        "the scan found only {checked} documentation lines — it has stopped working",
    );
    assert!(
        found.is_empty(),
        "documentation dated by a release number. Describe the current design; \
         which release it arrived in belongs in CHANGELOG.md: {found:#?}",
    );
}

/// Both scanners catch what they are for.
#[test]
fn the_doc_scanners_catch_planted_violations() {
    let chapter =
        "///\n".repeat(MAX_ITEM_DOC_LINES + 1) + "pub fn f() {}\n/// short\npub fn g() {}";
    assert_eq!(
        long_doc_blocks(&chapter),
        (vec![(1, MAX_ITEM_DOC_LINES + 1)], 2)
    );
    let at_the_limit = "///\n".repeat(MAX_ITEM_DOC_LINES);
    assert_eq!(
        long_doc_blocks(&at_the_limit),
        (vec![], 1),
        "a block at end of file"
    );
    let module_doc = "//!\n".repeat(MAX_ITEM_DOC_LINES + 1);
    assert_eq!(
        long_doc_blocks(&module_doc),
        (vec![], 0),
        "module docs are exempt"
    );

    for dated in [
        "/// Since 0.17 the tag is pinned.",
        "/// The tags added in 0.20.",
        "/// As of v0.21, a key is tagged.",
        "Changed in release 0.18: the gas codes.",
    ] {
        assert!(names_a_release(dated), "{dated}");
    }
    for clean in [
        "/// A factor of 0.5 since the meter is shared.",
        "/// Since 2025 the Modul 3 is mandatory.",
        "/// Values as of 01.10.2026.",
    ] {
        assert!(!names_a_release(clean), "{clean}");
    }
}
