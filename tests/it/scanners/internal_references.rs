//! Published files cite published sources only.
//!
//! The repository keeps working material — research notes, specifications,
//! decision records — in folders that never ship. A published file that points
//! into them points a reader at nothing: the path does not exist in the crate
//! they downloaded, and an identifier like a decision number or a requirement
//! id means nothing outside the working tree. The source document itself is
//! what a reader can follow, so that is what gets cited.
//!
//! Published means: everything under `src/`, `tests/`, `examples/` and the
//! site, the README, and the unreleased section of the changelog. Released
//! changelog sections are history and stay as written.
//!
//! The needles are assembled at run time, so this file does not cite what it
//! forbids and needs no exemption.

use super::{files_under, read, relative, root};

/// The working folders, each followed by `/` when cited as a path.
fn working_folders() -> [String; 3] {
    ["concepts", "specs", ".specify"].map(|d| format!("{d}/"))
}

/// Whether `text[at..]` starts a word: nothing alphanumeric right before it.
fn at_word_start(text: &str, at: usize) -> bool {
    text[..at]
        .chars()
        .next_back()
        .is_none_or(|c| !(c.is_alphanumeric() || c == '_'))
}

/// The internal references on one line, as the matched text.
fn internal_references(line: &str) -> Vec<String> {
    let mut found = Vec::new();

    for folder in working_folders() {
        for (at, _) in line.match_indices(&folder) {
            // The dotted folder needs no word boundary; the others must start a
            // word, so a longer name merely ending in one is not matched.
            if folder.starts_with('.') || at_word_start(line, at) {
                found.push(folder.clone());
            }
        }
    }

    // Identifiers: a decision number (D or R, an optional hyphen, up to three
    // digits), a requirement or success criterion (FR or SC, a hyphen,
    // digits), a user story (US, digits) or a task (T, three digits). The
    // planted test below spells out each shape.
    for (at, c) in line.char_indices() {
        if !at_word_start(line, at) {
            continue;
        }
        let rest = &line[at..];
        let (prefix_len, max_digits, min_digits) =
            if rest.starts_with("FR-") || rest.starts_with("SC-") {
                (3, 4, 1)
            } else if rest.starts_with("US") {
                (2, 3, 1)
            } else if c == 'T' {
                (1, 3, 3)
            } else if c == 'D' || c == 'R' {
                (if rest[1..].starts_with('-') { 2 } else { 1 }, 3, 1)
            } else {
                continue;
            };
        let digits: String = rest[prefix_len..]
            .chars()
            .take_while(char::is_ascii_digit)
            .collect();
        let end = at + prefix_len + digits.len();
        let ends_word = line[end..]
            .chars()
            .next()
            .is_none_or(|c| !(c.is_alphanumeric() || c == '_'));
        if (min_digits..=max_digits).contains(&digits.len()) && ends_word {
            found.push(line[at..end].to_owned());
        }
    }
    found
}

/// The unreleased section of the changelog: from the first `## ` heading that
/// says *unreleased* to the next `## ` heading.
fn unreleased_section(changelog: &str) -> String {
    let mut out = String::new();
    let mut inside = false;
    for line in changelog.lines() {
        if line.starts_with("## ") {
            if inside {
                break;
            }
            inside = line.to_lowercase().contains("unreleased");
        }
        if inside {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

/// No published file cites the working tree.
#[test]
fn published_files_cite_no_internal_material() {
    let mut texts: Vec<(String, String)> = Vec::new();
    for path in files_under("src", &["rs"], &[])
        .into_iter()
        .chain(files_under("tests", &["rs"], &[]))
        .chain(files_under("examples", &["rs"], &[]))
        .chain(files_under(
            "site",
            &["md", "html", "toml", "scss", "txt", "svg"],
            &["public"],
        ))
    {
        texts.push((relative(&path), read(&path)));
    }
    texts.push(("README.md".to_owned(), read(&root().join("README.md"))));
    texts.push((
        "CHANGELOG.md (unreleased)".to_owned(),
        unreleased_section(&read(&root().join("CHANGELOG.md"))),
    ));

    assert!(
        texts.iter().any(|(p, _)| p.starts_with("examples/")),
        "the scan read no example — it has stopped working"
    );
    let rust_files = texts.iter().filter(|(p, _)| p.ends_with(".rs")).count();
    assert!(
        rust_files > 30 && texts.iter().any(|(p, _)| p.starts_with("site/")),
        "the scan found only {rust_files} Rust files and no site page — it has stopped working",
    );

    let mut found = Vec::new();
    for (path, text) in &texts {
        for (index, line) in text.lines().enumerate() {
            for hit in internal_references(line) {
                found.push(format!("{path}:{}: {hit} — {}", index + 1, line.trim()));
            }
        }
    }
    assert!(
        found.is_empty(),
        "published files citing material that does not ship. Cite the source \
         document (title, version, page or clause) and state the decision \
         itself: {found:#?}",
    );
}

/// Published citations of the regulation itself are not internal material.
#[test]
fn regulatory_citations_pass_the_guard() {
    for line in [
        "/// [EnWG § 14a] netzorientierte Steuerung",
        "/// BK6-22-300 Anlage 1 Ziff. 4.5.2, Tenorziffer 2",
        "/// MiSpeL Anlage 1 formula (19), Fall A5",
        "/// [MsbG § 60 Abs. 6], Kap. 3.1 der Allg. Festlegungen 6.1d",
        "/// UTILTS AHB 1.1: Z69 Addition, ZG6 Aufteilungsfaktor",
    ] {
        assert!(
            internal_references(line).is_empty(),
            "a regulatory citation was flagged: {line}"
        );
    }
}

/// The guard catches what it is for.
#[test]
fn the_reference_guard_catches_planted_violations() {
    let [concepts, specs, specify] = working_folders();
    for (line, hit) in [
        (format!("//! (`{specs}016-x/vectors.md`)"), specs.clone()),
        (
            format!("/// See {concepts}reference/af.pdf."),
            concepts.clone(),
        ),
        (format!("Recorded in `{specify}memory`."), specify.clone()),
        (
            format!("/// cut once (decision {}17)", 'D'),
            format!("{}17", 'D'),
        ),
        (format!("/// rejected in {}16.", 'R'), format!("{}16", 'R')),
        (
            format!("/// Q2 under {}-3: zero.", 'D'),
            format!("{}-3", 'D'),
        ),
        (
            format!("// ── B. semantics ({}{}-002)", 'S', 'C'),
            format!("{}{}-002", 'S', 'C'),
        ),
        (
            format!("// ── C. construction ({}{}-004)", 'F', 'R'),
            format!("{}{}-004", 'F', 'R'),
        ),
        (
            format!("// ── {}S2: the former shapes", 'U'),
            format!("{}S2", 'U'),
        ),
        (format!("- done in {}012", 'T'), format!("{}012", 'T')),
    ] {
        assert_eq!(internal_references(&line), [hit], "{line}");
    }

    for clean in [
        "/// `1-0:1.29.0` is the Lastgang (D = 29).",
        "/// DE2380 carries HHMM, DTM+163 the start.",
        "/// AWH cases B1, B2, L1 and Q2; the Hampel window; a DST day.",
        "/// the dependency specs are in Cargo.toml",
        "/// RFC 3339, ISO 8601, EN 50160, Z69 and ZG6.",
        "/// Tag 10D, a T-junction, T12 and T1000 are not task ids.",
    ] {
        let hits = internal_references(clean);
        assert!(hits.is_empty(), "{clean}: {hits:?}");
    }

    let changelog =
        "# Changelog\n\n## [0.26.0] — unreleased\n\nnew\n\n## [0.25.0] — 2026-10-01\n\nold\n";
    let section = unreleased_section(changelog);
    assert!(
        section.contains("new") && !section.contains("old"),
        "{section}"
    );
}
