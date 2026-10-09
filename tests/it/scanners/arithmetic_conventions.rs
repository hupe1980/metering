//! Source scans for the two arithmetic conventions the crate states and a
//! reviewer cannot reliably enforce.
//!
//! Both read `src/` as data, for the reason the other scanners do: a rule that
//! lives only in a document holds until the next person writes the natural
//! thing instead. A third, the float crossing, is a scan of the same kind; the
//! tie table of `metering::precision` is pinned last.

/// The non-test part of every source file under `src/`.
///
/// Everything from a module's `#[cfg(test)]` marker on is test code, which may
/// round and divide however its fixture needs.
fn sources() -> Vec<(String, String)> {
    super::src_files()
        .into_iter()
        .map(|(name, text)| {
            let body = text
                .split_once("#[cfg(test)]")
                .map_or(text.clone(), |(before, _)| before.to_owned());
            (name, body)
        })
        .collect()
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
    // A trailing comment is prose, whatever operators it mentions.
    let line = line.split_once("//").map_or(line, |(code, _)| code);
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
/// Every rounding in `src/` calls `round_dp_with_strategy` with a named `*_DP`
/// constant and a named strategy from `metering::precision`. A bare
/// `round_dp(` — `rust_decimal`'s half-to-even default, chosen by nobody — is
/// refused outright, and so is a literal width or an inline
/// `RoundingStrategy::` at the call site.
///
/// Exceptions are listed here with their reason, not annotated at the call
/// site.
#[test]
fn every_rounding_names_its_constant_and_strategy() {
    let mut offenders = Vec::new();
    let mut calls = 0usize;
    for (name, text) in sources() {
        for (n, line) in text.lines().enumerate() {
            if is_comment(line) {
                continue;
            }
            if line.contains("round_dp_with_strategy(") {
                calls += 1;
            }
            for offence in rounding_offences(&name, line) {
                offenders.push(format!("{offence} — {name}:{}: {}", n + 1, line.trim()));
            }
        }
    }
    assert!(calls > 5, "the scan found only {calls} roundings");
    assert!(
        offenders.is_empty(),
        "every rounding names a precision constant and its strategy:\n  {}",
        offenders.join("\n  ")
    );
}

/// (file, literal, reason) — roundings that belong to a *caller's* setting.
const ROUNDING_ALLOWED: &[(&str, &str, &str)] = &[
    (
        "gas/conversion.rs",
        "0",
        "G685FinalRounding::WholeKwh — the Netzbetreiber's own final rounding",
    ),
    (
        "gas/conversion.rs",
        "2",
        "G685FinalRounding::TwoDecimals — likewise",
    ),
];

/// What is wrong with the roundings on one line of `file`, if anything.
fn rounding_offences(file: &str, line: &str) -> Vec<&'static str> {
    let mut found = Vec::new();
    if bare_round_dp(line) {
        found.push("bare round_dp");
    }
    let call = "round_dp_with_strategy(";
    for (pos, _) in line.match_indices(call) {
        let args = &line[pos + call.len()..];
        let mut parts = args.split([',', ')']);
        let dp = parts.next().unwrap_or_default().trim();
        let strategy = parts.next().unwrap_or_default().trim();
        let literal = !dp.is_empty() && dp.chars().all(|c| c.is_ascii_digit());
        if literal
            && !ROUNDING_ALLOWED
                .iter()
                .any(|(f, lit, _)| *f == file && *lit == dp)
        {
            found.push("literal width");
        }
        if strategy.contains("RoundingStrategy::") {
            found.push("inline strategy");
        }
    }
    found
}

/// `round_dp(` that is not `round_dp_with_strategy(`.
fn bare_round_dp(line: &str) -> bool {
    line.match_indices("round_dp(").any(|(i, _)| {
        // `round_dp_with_strategy(` does not contain `round_dp(`, so any hit is
        // the bare call — unless it is part of a longer identifier.
        i == 0
            || !(line.as_bytes()[i - 1].is_ascii_alphanumeric() || line.as_bytes()[i - 1] == b'_')
    })
}

/// **One float crossing.** `Decimal` is the quantity type; a float may be used
/// where `Decimal` cannot compute (the SigLinDe sigmoid's fractional power)
/// and for pure statistics that never write into a quantity. Every other
/// `f64 ↔ Decimal` conversion is refused, and a failed one must be `None`,
/// never zero.
#[test]
fn floats_cross_into_quantities_only_at_the_listed_sites() {
    let mut offenders = Vec::new();
    let mut seen = Vec::new();
    for (name, text) in sources() {
        for (n, line) in text.lines().enumerate() {
            if is_comment(line) || !float_crossing(line) {
                continue;
            }
            if float_allowed(&name) {
                seen.push(name.clone());
            } else {
                offenders.push(format!("{name}:{}: {}", n + 1, line.trim()));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "an f64 crossing outside the allow-list:\n  {}",
        offenders.join("\n  ")
    );
    // A listed site that no longer crosses is a stale exemption.
    for (file, _) in FLOAT_ALLOWED {
        assert!(
            seen.iter().any(|s| s == file),
            "{file} is on the float allow-list and no longer crosses — remove it"
        );
    }
}

/// The only files allowed to convert between `f64` and `Decimal`, with why.
const FLOAT_ALLOWED: &[(&str, &str)] = &[(
    "slp/gas.rs",
    "the SigLinDe sigmoid raises to a fractional power (Leitfaden SLP Gas)",
)];

fn float_allowed(file: &str) -> bool {
    FLOAT_ALLOWED.iter().any(|(f, _)| *f == file)
}

fn float_crossing(line: &str) -> bool {
    ["from_f64", "to_f64", "Decimal::try_from("]
        .iter()
        .any(|p| line.contains(p))
}

/// The scanners catch what they are for — planted violations.
#[test]
fn the_scanners_catch_planted_violations() {
    assert!(bare_round_dp("let x = y.round_dp(2);"));
    assert!(!bare_round_dp(
        "let x = y.round_dp_with_strategy(PERCENT_DP, PERCENT_STRATEGY);"
    ));
    assert!(float_crossing("let d = Decimal::try_from(f).ok()?;"));
    assert!(float_crossing("let f = d.to_f64()?;"));
    assert!(!float_crossing("let d = Decimal::from(3u32);"));
    assert!(scaled_quotient("let x = a / b * c;"));
    assert!(scaled_quotient("let x = total / count() * share;"));
    assert!(scaled_quotient("let x = v[0] / n * k;"));
    assert!(!scaled_quotient("let x = a * c / b;"));
    assert!(!scaled_quotient("let x = t * p / (e * pn * k);"));
    assert!(!scaled_quotient("let x = a / b; // * c"));
    assert!(bare_round_dp("    .round_dp(PERCENT_DP)"));
    assert!(!bare_round_dp("my_round_dp(x)"));
    assert!(float_crossing("let d = Decimal::from_f64(f)?;"));
    // The float allow-list is per file and exact: a crossing anywhere else is
    // an offence, and the list cannot silently widen to a directory.
    assert!(float_allowed("slp/gas.rs"));
    assert!(!float_allowed("billing/forecast.rs"));
    assert!(!float_allowed("slp/strom.rs"));
    assert!(!float_allowed("slp/"));
    assert_eq!(
        FLOAT_ALLOWED.len(),
        1,
        "a new float site is a decision — record it"
    );

    assert_eq!(
        rounding_offences(
            "billing/aggregation.rs",
            "let x = y.round_dp_with_strategy(3, RoundingStrategy::MidpointAwayFromZero);"
        ),
        ["literal width", "inline strategy"]
    );
    assert_eq!(
        rounding_offences("billing/aggregation.rs", "let x = y.round_dp(2);"),
        ["bare round_dp"]
    );
    assert!(
        rounding_offences(
            "billing/aggregation.rs",
            "let x = y.round_dp_with_strategy(PERCENT_DP, PERCENT_STRATEGY);"
        )
        .is_empty()
    );
    // The allow-list is per file: the same literal elsewhere is reported.
    let g685 = "x.round_dp_with_strategy(2, G685_STRATEGY)";
    assert!(rounding_offences("gas/conversion.rs", g685).is_empty());
    assert_eq!(
        rounding_offences("heat/heizkosten.rs", g685),
        ["literal width"]
    );
}

/// Every precision constant at a tie, positive and negative: the table in
/// `metering::precision` is the behaviour, pinned here.
#[test]
fn every_precision_constant_resolves_its_ties_as_documented() {
    use metering::precision::*;
    use rust_decimal::{Decimal, RoundingStrategy, dec};

    fn r(x: Decimal, dp: u32, s: RoundingStrategy) -> Decimal {
        x.round_dp_with_strategy(dp, s)
    }
    // Toward zero: the formula quotient (a tie and a near-tie both cut down,
    // both signs), so a proportional share never exceeds its pool.
    assert_eq!(
        r(
            dec!(0.6666665),
            FORMULA_QUOTIENT_DP,
            FORMULA_QUOTIENT_STRATEGY
        ),
        dec!(0.666666)
    );
    assert_eq!(
        r(
            dec!(0.6666669),
            FORMULA_QUOTIENT_DP,
            FORMULA_QUOTIENT_STRATEGY
        ),
        dec!(0.666666)
    );
    assert_eq!(
        r(
            dec!(-0.6666665),
            FORMULA_QUOTIENT_DP,
            FORMULA_QUOTIENT_STRATEGY
        ),
        dec!(-0.666666)
    );
    // Kaufmännisch: half away from zero, both signs.
    assert_eq!(r(dec!(0.125), PERCENT_DP, PERCENT_STRATEGY), dec!(0.13));
    assert_eq!(r(dec!(-0.125), PERCENT_DP, PERCENT_STRATEGY), dec!(-0.13));
    assert_eq!(
        r(dec!(2499.995), BENUTZUNGSDAUER_DP, BENUTZUNGSDAUER_STRATEGY),
        dec!(2500.00)
    );
    assert_eq!(
        r(dec!(-1.005), BENUTZUNGSDAUER_DP, BENUTZUNGSDAUER_STRATEGY),
        dec!(-1.01)
    );
    assert_eq!(r(dec!(0.0025), FORECAST_DP, FORECAST_STRATEGY), dec!(0.003));
    assert_eq!(
        r(dec!(-0.0025), FORECAST_DP, FORECAST_STRATEGY),
        dec!(-0.003)
    );
    assert_eq!(
        r(dec!(1.00005), SEASONAL_FACTOR_DP, SEASONAL_FACTOR_STRATEGY),
        dec!(1.0001)
    );
    assert_eq!(
        r(dec!(-1.00005), SEASONAL_FACTOR_DP, SEASONAL_FACTOR_STRATEGY),
        dec!(-1.0001)
    );
    assert_eq!(
        r(
            dec!(0.00005),
            FORECAST_ACCURACY_DP,
            FORECAST_ACCURACY_STRATEGY
        ),
        dec!(0.0001)
    );
    assert_eq!(
        r(
            dec!(-0.00005),
            FORECAST_ACCURACY_DP,
            FORECAST_ACCURACY_STRATEGY
        ),
        dec!(-0.0001)
    );
    assert_eq!(
        r(dec!(166.6665), AUSFALLARBEIT_DP, AUSFALLARBEIT_STRATEGY),
        dec!(166.667)
    );
    assert_eq!(
        r(dec!(-166.6665), AUSFALLARBEIT_DP, AUSFALLARBEIT_STRATEGY),
        dec!(-166.667)
    );
    assert_eq!(
        r(dec!(0.0000025), SUBSTITUTE_DP, SUBSTITUTE_STRATEGY),
        dec!(0.000003)
    );
    assert_eq!(
        r(dec!(-0.0000025), SUBSTITUTE_DP, SUBSTITUTE_STRATEGY),
        dec!(-0.000003)
    );
    assert_eq!(
        r(dec!(1.24205), DYNAMIZATION_DP, DYNAMIZATION_STRATEGY),
        dec!(1.2421)
    );
    assert_eq!(
        r(dec!(-1.24205), DYNAMIZATION_DP, DYNAMIZATION_STRATEGY),
        dec!(-1.2421)
    );
    assert_eq!(
        r(dec!(108.6745), DYNAMIZED_VALUE_DP, DYNAMIZED_VALUE_STRATEGY),
        dec!(108.675)
    );
    assert_eq!(
        r(
            dec!(-108.6745),
            DYNAMIZED_VALUE_DP,
            DYNAMIZED_VALUE_STRATEGY
        ),
        dec!(-108.675)
    );
    assert_eq!(
        r(dec!(0.95435), G685_ZUSTANDSZAHL_DP, G685_STRATEGY),
        dec!(0.9544)
    );
    assert_eq!(
        r(dec!(10.0005), G685_BRENNWERT_DP, G685_STRATEGY),
        dec!(10.001)
    );
    assert_eq!(
        r(dec!(-10.0005), G685_BRENNWERT_DP, G685_STRATEGY),
        dec!(-10.001)
    );
    // Mathematisch (Leitfaden SLP Gas footnote 22): half to even, both signs.
    assert_eq!(r(dec!(2.25), 1, KUNDENWERT_STRATEGY), dec!(2.2));
    assert_eq!(r(dec!(2.35), 1, KUNDENWERT_STRATEGY), dec!(2.4));
    assert_eq!(
        r(dec!(1.00005), KUNDENWERT_DP, KUNDENWERT_STRATEGY),
        dec!(1.0000)
    );
    assert_eq!(
        r(dec!(-1.00015), KUNDENWERT_DP, KUNDENWERT_STRATEGY),
        dec!(-1.0002)
    );
    assert_eq!(
        r(
            dec!(0.26665),
            ALLOCATION_TEMPERATURE_WEIGHT_DP,
            ALLOCATION_TEMPERATURE_WEIGHT_STRATEGY
        ),
        dec!(0.2666)
    );
    assert_eq!(
        r(
            dec!(-0.13335),
            ALLOCATION_TEMPERATURE_WEIGHT_DP,
            ALLOCATION_TEMPERATURE_WEIGHT_STRATEGY
        ),
        dec!(-0.1334)
    );
    // Allocation: toward zero, so a tie — and everything else — only shrinks.
    assert_eq!(
        r(dec!(0.0000015), ALLOCATION_DP, ALLOCATION_STRATEGY),
        dec!(0.000001)
    );
    assert_eq!(
        r(dec!(-0.0000015), ALLOCATION_DP, ALLOCATION_STRATEGY),
        dec!(-0.000001)
    );
    assert_eq!(
        r(dec!(0.0000019), ALLOCATION_DP, ALLOCATION_STRATEGY),
        dec!(0.000001)
    );
}
