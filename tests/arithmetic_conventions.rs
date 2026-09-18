//! Source scans for the two arithmetic conventions the crate states and a
//! reviewer cannot reliably enforce.
//!
//! Both read `src/` as data, for the reason the other scanners do: a rule that
//! lives only in a document holds until the next person writes the natural
//! thing instead.

use std::fs;
use std::path::Path;

/// Source files to scan, with their contents, line endings normalised.
///
/// The normalisation is not cosmetic: a scan that splits on `"\n"` under CRLF
/// sees one enormous line and matches nothing, which is the shape of a scanner
/// that passes for ever without looking at anything.
fn sources() -> Vec<(String, String)> {
    let mut out = Vec::new();
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut entries: Vec<_> = fs::read_dir(&dir)
        .expect("src/ is readable")
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "rs"))
        .collect();
    entries.sort();
    for path in entries {
        let text = fs::read_to_string(&path).expect("source is UTF-8");
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .to_owned();
        // Everything from the module's `#[cfg(test)]` marker is test code.
        // A test asserting `round_dp(4)` is **pinning** a documented width, not
        // introducing an undocumented one, and policing it would push the
        // constant into the assertion where it stops being a check.
        let text = text.replace("\r\n", "\n");
        let body = text
            .split_once("#[cfg(test)]")
            .map_or(text.clone(), |(before, _)| before.to_owned());
        out.push((name, body));
    }
    out
}

fn is_comment(line: &str) -> bool {
    let t = line.trim_start();
    t.starts_with("//")
}

/// **Multiply before you divide.**
///
/// `a ÷ b × c` rounds the quotient to 28 significant digits and then scales
/// that error up by `c`; `a × c ÷ b` rounds once, at the end. The difference is
/// invisible in an example and decisive at a tie: three tenants of 4 kWh
/// against 9 kWh of generation exhaust it exactly one way and come three
/// millionths short the other.
///
/// The pattern this rejects is a division by a **simple** term followed by a
/// multiplication — `x / y * z`. A division by a parenthesised product,
/// `x / (a * b)`, is a single quotient with a composed denominator and is
/// exactly what the rule asks for, so it is not matched.
///
/// There is no exception list. If a future expression genuinely needs the other
/// order, it needs a reason in this test, next to the rule it is an exception
/// to — not a comment at the call site where nobody rechecks it.
#[test]
fn no_quotient_is_scaled_after_the_fact() {
    let mut offenders = Vec::new();
    for (name, text) in sources() {
        for (n, line) in text.lines().enumerate() {
            if is_comment(line) {
                continue;
            }
            if scaled_quotient(line) {
                offenders.push(format!("{name}:{}: {}", n + 1, line.trim()));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "divide-then-multiply rounds a quotient and then scales the error up. \
         Reorder to multiply first:\n  {}",
        offenders.join("\n  ")
    );
}

/// `… / <simple term> * …` — a quotient used as a factor.
fn scaled_quotient(line: &str) -> bool {
    let bytes = line.as_bytes();
    let mut i = 0;
    while let Some(pos) = line[i..].find('/') {
        let at = i + pos;
        i = at + 1;
        // `//` is a comment, `/=` is an assignment, `*/` closes a block comment.
        if bytes.get(at + 1).is_some_and(|c| *c == b'/' || *c == b'=') {
            continue;
        }
        if at > 0 && bytes[at - 1] == b'*' {
            continue;
        }
        let rest = line[at + 1..].trim_start();
        // A parenthesised divisor is a composed denominator, not a scaled
        // quotient: `T_n × p ÷ (T_eff × p_n × K)` is one rounding.
        if rest.starts_with('(') {
            continue;
        }
        let term_len = rest
            .find(|c: char| !(c.is_alphanumeric() || c == '_' || c == '.' || c == '[' || c == ']'))
            .unwrap_or(rest.len());
        let after = rest[term_len..].trim_start();
        // `a / b()` — a call, whose parentheses are part of the term.
        let after = after.strip_prefix("()").map_or(after, str::trim_start);
        if after.starts_with('*') && !after.starts_with("*/") {
            return true;
        }
    }
    false
}

/// **A rounding that is not stated is a rounding nobody can reproduce.**
///
/// Every `round_dp` in `src/` cuts a quantity somebody stores, prints or
/// settles on, so the number of places is a documented constant rather than a
/// literal in an expression. `round_dp(4)` in the middle of a function is a
/// rule with no name and no place to cite.
///
/// Exceptions are listed here with their reason, not annotated at the call
/// site.
#[test]
fn every_rounding_names_the_constant_it_rounds_to() {
    // (file, literal, reason) — each is a rounding that belongs to a *caller's*
    // setting rather than to a crate-wide constant.
    const ALLOWED: &[(&str, &str, &str)] = &[
        (
            "conversion.rs",
            "0",
            "G685FinalRounding::WholeKwh — the Netzbetreiber's own final rounding,              named by the variant rather than by a crate constant",
        ),
        (
            "conversion.rs",
            "2",
            "G685FinalRounding::TwoDecimals — likewise",
        ),
        (
            "forecast.rs",
            "4",
            "the seasonal factor is a dimensionless ratio of two rates that is              multiplied into a projection already cut to FORECAST_DP",
        ),
    ];

    let mut offenders = Vec::new();
    for (name, text) in sources() {
        for (n, line) in text.lines().enumerate() {
            if is_comment(line) {
                continue;
            }
            for call in ["round_dp(", "round_dp_with_strategy("] {
                let Some(pos) = line.find(call) else { continue };
                let arg = line[pos + call.len()..]
                    .split([',', ')'])
                    .next()
                    .unwrap_or_default()
                    .trim();
                if arg.is_empty() || !arg.chars().all(|c| c.is_ascii_digit()) {
                    continue; // a named constant — which is the rule
                }
                if ALLOWED.iter().any(|(f, lit, _)| *f == name && *lit == arg) {
                    continue;
                }
                offenders.push(format!("{name}:{}: {}", n + 1, line.trim()));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "a rounding width is a documented constant, not a literal:\n  {}",
        offenders.join("\n  ")
    );
}
