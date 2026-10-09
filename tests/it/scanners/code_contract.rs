//! One code per value, for **every** coded enum — the law, not the intention.
//!
//! The crate states the rule in its own docs: *"every type here that has a
//! string form has exactly one of them"*. This file is the enforcement — a
//! `Debug` rendering is not a contract, and a `serde` tag with no `as_str`,
//! `CODES` or `FromStr` beside it is a wire form consumers cannot reach.
//!
//! For every coded enum it asserts:
//!
//! 1. **`ALL` and `CODES` agree** — length and order. (Structural since
//!    `CODES` is computed from `ALL`, but pinned so a refactor cannot undo it.)
//! 2. **`as_str` is `Display`** — one string, not two.
//! 3. **`FromStr` inverts `as_str`**, and is lenient about case and
//!    surrounding whitespace, like every other parser here.
//! 4. **The codes are distinct**, so two variants cannot collapse into one row.
//! 5. **The `serde` tag *is* the code** — the property that lets a consumer
//!    pin a `CHECK` constraint to `CODES` and know the two cannot drift.
//! 6. **An unknown code is an error**, never a silent default.
//!
//! The list below is itself part of the contract, and
//! [`no_coded_enum_escapes_this_file`] holds it there: it reads the crate
//! source, collects every type the `string_codes!` macro is applied to, and
//! fails if one of them is missing from the list. A comment saying "remember to
//! add it here" is not a mechanism.

use std::collections::BTreeSet;

use metering::allocation::formula::{FactorKind, FlowDirection, Operator};
use metering::allocation::sharing::{
    Bilanzierungsmethode, Delivery, EligibilityBasis, Finding, SharingReadiness, Zaehlertyp,
};
use metering::grid::para14a::{SteuVeFallgruppe, Verursachungsregel};
use metering::grid::power_quality::Phase;
use metering::grid::rollout::RolloutObligation;
use metering::slp::strom::LoadProfile;
use metering::time::holiday::Bundesland;
use metering::vee::classification::Messtyp;
use metering::vee::substitute::{Method, SubstitutionReason};
use metering::vee::validation::{Grade, Rule, Severity};
use metering::{Direction, QualityFlag, Sparte, Unit};

/// Assert the full contract for one type.
macro_rules! assert_contract {
    ($ty:ty) => {{
        let name = stringify!($ty);
        let all = <$ty>::ALL;
        let codes = <$ty>::CODES;

        assert_eq!(
            all.len(),
            codes.len(),
            "{name}: ALL and CODES differ in length"
        );
        assert!(!all.is_empty(), "{name}: ALL is empty");

        let mut seen = std::collections::BTreeSet::new();
        for (value, code) in all.iter().zip(codes) {
            // 2 — as_str is Display.
            assert_eq!(
                value.as_str(),
                *code,
                "{name}: CODES is out of step with as_str"
            );
            assert_eq!(&value.to_string(), *code, "{name}: Display is not as_str");

            // 3 — FromStr inverts it, leniently.
            assert_eq!(
                &code
                    .parse::<$ty>()
                    .unwrap_or_else(|e| panic!("{name}/{code}: {e}")),
                value,
                "{name}: FromStr does not invert as_str"
            );
            let messy = format!("  {}\t", code.to_lowercase());
            assert_eq!(
                &messy
                    .parse::<$ty>()
                    .unwrap_or_else(|e| panic!("{name}/{messy:?}: {e}")),
                value,
                "{name}: FromStr is not lenient about case and whitespace"
            );

            // 4 — distinct.
            assert!(seen.insert(*code), "{name}: duplicate code {code}");

            // 5 — the serde tag is the code.
            #[cfg(feature = "serde")]
            {
                let json = serde_json::to_string(value).expect("serialises");
                assert_eq!(
                    json,
                    format!("\"{code}\""),
                    "{name}: the serde tag is not the code — a CHECK constraint \
                     generated from CODES would reject rows this crate writes"
                );
                let back: $ty = serde_json::from_str(&json).expect("deserialises");
                assert_eq!(&back, value, "{name}: serde does not round-trip");
            }

            // Width and alignment reach the output, so a code lines up in a
            // table the way every other one here does.
            let width = code.len() + 3;
            let padded = format!("{value:>width$}");
            assert!(
                padded.ends_with(code) && padded.len() == width,
                "{name}: Display ignores width and alignment, so a code cannot \
                 be lined up in a table"
            );
        }

        // 6 — an unknown code is an error.
        assert!(
            "__NOT_A_CODE__".parse::<$ty>().is_err(),
            "{name}: an unrecognised code must be an error, never a default"
        );
        let err = "__NOT_A_CODE__".parse::<$ty>().unwrap_err();
        assert_eq!(
            err.type_name(),
            name,
            "{name}: the error names the wrong type"
        );
        assert_eq!(
            err.expected_values(),
            Some(codes),
            "{name}: the error does not carry the accepted set"
        );
    }};
}

/// Every coded enum in the crate.
#[test]
fn every_coded_enum_holds_the_contract() {
    use metering::billing::zaehlzeit::{DayGroup, Modul3Conformance, Modul3Finding, Quarter};
    use metering::gas::conversion::G685FinalRounding;
    use metering::grid::rollout::QuotaScope;
    use metering::ids::{EicType, Issuer, Regelzone};
    use metering::series::reading::AnomalyKind;
    use metering::slp::gas::GasProfile;
    use metering::slp::strom::SlpDayType;
    use metering::time::calendar::{DayBoundary, DayKind};
    use metering::time::holiday::Holiday;

    // interval / quantities
    assert_contract!(Sparte);
    assert_contract!(QualityFlag);
    assert_contract!(Direction);

    // Redispatch — BilAReM Kap. 3
    use metering::grid::ausfallarbeit::{Abrechnungsvariante, Redispatchfall, Redispatchrichtung};
    assert_contract!(Redispatchrichtung);
    assert_contract!(Redispatchfall);
    assert_contract!(Abrechnungsvariante);

    // identifiers and channels
    assert_contract!(Issuer);
    assert_contract!(EicType);
    assert_contract!(Regelzone);
    assert_contract!(Phase);

    // calendar and profiles
    assert_contract!(DayKind);
    assert_contract!(DayBoundary);
    assert_contract!(Bundesland);
    assert_contract!(Holiday);
    assert_contract!(LoadProfile);
    assert_contract!(GasProfile);
    assert_contract!(SlpDayType);

    // pipeline
    assert_contract!(AnomalyKind);
    assert_contract!(Rule);
    assert_contract!(Severity);
    assert_contract!(Grade);
    assert_contract!(Method);
    assert_contract!(SubstitutionReason);
    assert_contract!(Messtyp);
    assert_contract!(DayGroup);
    assert_contract!(Quarter);
    assert_contract!(Modul3Finding);
    assert_contract!(Modul3Conformance);
    assert_contract!(G685FinalRounding);

    // allocation
    assert_contract!(Operator);
    assert_contract!(FlowDirection);
    assert_contract!(FactorKind);

    // regulatory classification
    assert_contract!(SteuVeFallgruppe);
    assert_contract!(Verursachungsregel);
    assert_contract!(RolloutObligation);
    assert_contract!(QuotaScope);
    assert_contract!(EligibilityBasis);
    assert_contract!(Finding);
    assert_contract!(Zaehlertyp);
    assert_contract!(Bilanzierungsmethode);
    assert_contract!(Delivery);
    assert_contract!(SharingReadiness);

    // EEG / MiSpeL and HeizkostenV
    use metering::eeg::mispel::{Abgrenzungsfall, Pauschalfall};
    use metering::heat::heizkosten::{Brennstoff, Schaetzgrundlage};
    assert_contract!(Abgrenzungsfall);
    assert_contract!(Pauschalfall);
    assert_contract!(Brennstoff);
    assert_contract!(Schaetzgrundlage);
}

/// `Unit` is the one type whose `FromStr` is deliberately wider than its
/// `CODES`: it reads `m³`, `kWh_th` and the UN/ECE Rec 20 codes as well. The
/// write half of the contract still holds.
#[test]
fn unit_writes_its_codes_and_reads_symbols_too() {
    assert_eq!(Unit::ALL.len(), Unit::CODES.len());
    for (v, code) in Unit::ALL.iter().zip(Unit::CODES) {
        assert_eq!(v.as_str(), *code);
        assert_eq!(&v.to_string(), *code);
        assert_eq!(&code.parse::<Unit>().unwrap(), v);
        #[cfg(feature = "serde")]
        assert_eq!(serde_json::to_string(v).unwrap(), format!("\"{code}\""));
    }
    for wide in ["m³", "kWh_th", "MTQ", "cbm", "kvarh"] {
        assert!(wide.parse::<Unit>().is_ok(), "{wide}");
    }
}

/// The lenient input aliases: accepted on the way in, never written out.
#[test]
fn input_aliases_normalise_onto_the_canonical_code() {
    // German callers type the umlaut.
    assert_eq!("WÄRME".parse::<Sparte>().unwrap(), Sparte::Waerme);
    assert_eq!("wärme".parse::<Sparte>().unwrap(), Sparte::Waerme);
    assert_eq!(
        Sparte::Waerme.to_string(),
        "WAERME",
        "one spelling comes out"
    );
    assert!(!Sparte::CODES.contains(&"WÄRME"), "an alias is not a code");

    // ISO 3166-2 writes the `DE-` prefix; the market does not.
    assert_eq!("DE-BY".parse::<Bundesland>().unwrap(), Bundesland::By);
    assert_eq!("de-by".parse::<Bundesland>().unwrap(), Bundesland::By);
    assert_eq!(Bundesland::By.to_string(), "BY");

    // Codes in circulation for HEF and HMF.
    use metering::slp::gas::GasProfile;
    assert_eq!("EF".parse::<GasProfile>().unwrap(), GasProfile::Hef);
    assert_eq!("MF".parse::<GasProfile>().unwrap(), GasProfile::Hmf);
    assert_eq!(GasProfile::Hef.to_string(), "HEF");

    // The market says Bezug and Einspeisung; the stored code matches the OBIS
    // helpers that answer the same question, `is_import` / `is_export`.
    assert_eq!("BEZUG".parse::<Direction>().unwrap(), Direction::Import);
    assert_eq!(
        "einspeisung".parse::<Direction>().unwrap(),
        Direction::Export
    );
    assert_eq!(Direction::Export.to_string(), "EXPORT");
    assert_eq!(Direction::Export.bezeichnung(), "Einspeisung");
    assert!(
        !Direction::CODES.contains(&"BEZUG"),
        "an alias is not a code"
    );
}

/// A code and a human-facing description are different things, and the types
/// that have both keep them apart.
#[test]
fn display_forms_are_codes_and_descriptions_are_prose() {
    use metering::time::holiday::Holiday;

    assert_eq!(Holiday::BussUndBettag.as_str(), "BUSS_UND_BETTAG");
    assert_eq!(Holiday::BussUndBettag.name(), "Buß- und Bettag");

    assert_eq!(Unit::KiloWattHour.as_str(), "KWH");
    assert_eq!(Unit::KiloWattHour.symbol(), "kWh");

    // The wire code and the market code are two surfaces.
    assert_eq!(Method::Vergleichswert.as_str(), "VERGLEICHSWERT");
    assert_eq!(
        Method::Vergleichswert.market_code(Sparte::Strom),
        Some("ZJ2")
    );

    // Every description is non-empty, and none of them is the code.
    for r in SubstitutionReason::ALL {
        assert!(!r.description().is_empty());
        assert_ne!(r.description(), r.as_str());
    }
}

/// The rule code is the only spelling — `Display`, `as_str` and the `serde`
/// tag all say `GAP` — so a stored finding reads back through the vocabulary
/// it was written with.
#[test]
fn validation_rules_are_their_codes_everywhere() {
    assert_eq!(Rule::Gap.as_str(), "GAP");
    assert_eq!(Rule::Laengsvergleich.to_string(), "LAENGSVERGLEICH");
    assert_eq!("zero_run".parse::<Rule>().unwrap(), Rule::ZeroRun);
    #[cfg(feature = "serde")]
    assert_eq!(serde_json::to_string(&Rule::Spike).unwrap(), "\"SPIKE\"");
    assert!("V01".parse::<Rule>().is_err());
}

/// Every `string_codes!` type appears in [`every_coded_enum_holds_the_contract`].
///
/// The macro is the crate's single definition of "this is a coded enum", so the
/// set of types it is applied to is the set the contract must cover. Reading it
/// out of the source is crude, and it is the only way to make the list above
/// self-maintaining: nothing in Rust lets a test enumerate the types a macro was
/// invoked on, so without this a new enum joins the crate with no contract at
/// all and every existing assertion still passes.
#[test]
fn no_coded_enum_escapes_this_file() {
    let mut coded: BTreeSet<String> = BTreeSet::new();
    for (_, text) in super::src_files() {
        coded.extend(string_codes_types(&text));
    }
    assert!(
        coded.len() > 30,
        "the scan found only {} types — it has stopped working, not the crate",
        coded.len()
    );

    // This file, read at compile time: no path to get wrong when it moves.
    let asserted = asserted_types(include_str!("code_contract.rs"));

    let missing: Vec<&String> = coded.difference(&asserted).collect();
    assert!(
        missing.is_empty(),
        "coded enums with no contract assertion: {missing:?} — add \
         `assert_contract!(…)` to every_coded_enum_holds_the_contract"
    );

    let stale: Vec<&String> = asserted.difference(&coded).collect();
    assert!(
        stale.is_empty(),
        "asserted types that are no longer coded enums: {stale:?}"
    );
}

/// The types named in every `string_codes! { … }` block of `text`, whatever
/// path the macro is invoked through.
fn string_codes_types(text: &str) -> BTreeSet<String> {
    let mut coded = BTreeSet::new();
    let mut inside = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if !inside {
            // An invocation, not the macro's own documentation of one.
            inside = !trimmed.starts_with("//") && trimmed.ends_with("string_codes! {");
            continue;
        }
        if trimmed == "}" {
            inside = false;
            continue;
        }
        if trimmed.is_empty() || trimmed.starts_with("//") || trimmed.starts_with('#') {
            continue;
        }
        // `Ty;` or `Ty, aliases = [...];`
        let name = trimmed
            .split([',', ';'])
            .next()
            .unwrap_or_default()
            .trim()
            .to_owned();
        if name.starts_with(char::is_uppercase)
            && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        {
            coded.insert(name);
        }
    }
    coded
}

/// The type arguments of every `assert_contract!(…)` in `text`.
fn asserted_types(text: &str) -> BTreeSet<String> {
    text.match_indices("assert_contract!(")
        .filter_map(|(i, m)| {
            let tail = &text[i + m.len()..];
            tail.find(')').map(|end| tail[..end].trim().to_owned())
        })
        // The macro name also occurs inside this file's own prose; only a
        // type-shaped argument is a real invocation.
        .filter(|name| {
            name.starts_with(char::is_uppercase)
                && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        })
        .collect()
}

// ── serde-derived enums ──────────────────────────────────────────────────────

/// A `serde`-derived enum in the source, and whether its tags are codes.
#[derive(Debug)]
struct SerdeEnum {
    name: String,
    /// `SCREAMING_SNAKE_CASE` by `rename_all`, every variant renamed, or no
    /// tag at all (`untagged`, or serialised through another type).
    tags_are_codes: bool,
}

/// Every enum in `text` whose attributes derive `Serialize`.
///
/// An enum that derives its `serde` impls instead of going through
/// `string_codes!` writes its Rust variant names unless told otherwise —
/// `"Unchanged"`, `{"Fixed": …}` — next to a crate whose every other tag is
/// `SCREAMING_SNAKE_CASE`. So it needs a `rename_all`, a `rename` on every
/// variant, or no tag at all.
fn serde_derived_enums(text: &str) -> Vec<SerdeEnum> {
    let mut found = Vec::new();
    let mut attrs = String::new();
    let mut current: Option<(String, String, bool, bool)> = None; // name, enum attrs, all renamed, any variant
    let mut variant_attrs = String::new();
    let mut depth = 0usize;

    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("//") || trimmed.is_empty() {
            continue;
        }

        let in_body = current.is_some() && depth == 1;
        let pending = if in_body { &variant_attrs } else { &attrs };
        // An attribute may span several lines; it is finished when its
        // parentheses balance.
        let open = pending.matches('(').count() != pending.matches(')').count();
        if trimmed.starts_with("#[") || open {
            if in_body {
                variant_attrs.push_str(trimmed);
            } else {
                attrs.push_str(trimmed);
            }
            continue;
        }

        if let Some((name, enum_attrs, all_renamed, any)) = &mut current {
            if depth == 1 && trimmed.starts_with(char::is_uppercase) {
                *any = true;
                if !variant_attrs.contains("rename =") && !variant_attrs.contains("rename=") {
                    *all_renamed = false;
                }
            }
            variant_attrs.clear();
            depth = depth + trimmed.matches('{').count() - trimmed.matches('}').count();
            if depth == 0 {
                let screaming = enum_attrs.contains(r#"rename_all = "SCREAMING_SNAKE_CASE""#);
                let tagless = enum_attrs.contains("untagged")
                    || enum_attrs.contains("into =")
                    || enum_attrs.contains("try_from =");
                found.push(SerdeEnum {
                    name: std::mem::take(name),
                    tags_are_codes: screaming || tagless || (*all_renamed && *any),
                });
                current = None;
            }
            continue;
        }

        let decl = trimmed
            .strip_prefix("pub(crate) ")
            .or_else(|| trimmed.strip_prefix("pub "))
            .unwrap_or(trimmed);
        if let Some(rest) = decl.strip_prefix("enum ")
            && attrs.contains("Serialize")
        {
            let name: String = rest
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            depth = trimmed
                .matches('{')
                .count()
                .saturating_sub(trimmed.matches('}').count());
            let enum_attrs = std::mem::take(&mut attrs);
            if depth == 0 {
                // A one-line body: only an enum-level attribute can make its
                // tags codes.
                found.push(SerdeEnum {
                    name,
                    tags_are_codes: enum_attrs.contains(r#"rename_all = "SCREAMING_SNAKE_CASE""#)
                        || enum_attrs.contains("untagged"),
                });
                continue;
            }
            current = Some((name, enum_attrs, true, false));
            continue;
        }
        attrs.clear();
    }
    found
}

/// Derived enums whose tags are still Rust variant names. Empty: an entry is
/// a known wire-format defect, and the staleness check below removes it once
/// fixed.
const PASCAL_CASE_TAGS: &[&str] = &[];

/// Every `serde`-derived enum writes codes, like every `string_codes!` one.
#[test]
fn every_serde_derived_enum_writes_codes() {
    let mut enums = Vec::new();
    for (file, text) in super::src_files() {
        for e in serde_derived_enums(&text) {
            enums.push((file.clone(), e));
        }
    }
    assert!(
        enums.len() >= 5,
        "the scan found only {} serde-derived enums — it has stopped working",
        enums.len()
    );

    let offenders: Vec<String> = enums
        .iter()
        .filter(|(_, e)| !e.tags_are_codes && !PASCAL_CASE_TAGS.contains(&e.name.as_str()))
        .map(|(file, e)| format!("{file}: {}", e.name))
        .collect();
    assert!(
        offenders.is_empty(),
        "serde-derived enums whose tags are Rust variant names — add \
         `rename_all = \"SCREAMING_SNAKE_CASE\"` or route them through \
         `string_codes!`: {offenders:#?}"
    );

    for listed in PASCAL_CASE_TAGS {
        assert!(
            enums
                .iter()
                .any(|(_, e)| e.name == *listed && !e.tags_are_codes),
            "{listed} no longer writes Rust variant names — remove it from PASCAL_CASE_TAGS"
        );
    }
}

/// Both scanners in this file catch what they are for.
#[test]
fn the_contract_scanners_catch_planted_violations() {
    let planted = "crate::ids::codes::string_codes! {\n    Planted;\n    // a comment\n    Aliased, aliases = [\"X\"];\n}\n\
                   string_codes! {\n    Imported;\n}\n\
                   /// string_codes! {\n///     Self::ALL;\n/// }\n";
    assert_eq!(
        string_codes_types(planted),
        BTreeSet::from([
            "Aliased".to_owned(),
            "Imported".to_owned(),
            "Planted".to_owned()
        ])
    );
    assert_eq!(
        // Assembled, so this file's own scan does not read it as an assertion.
        asserted_types(&format!(
            "{0}!(Planted);\n/// `{0}!(…)` in prose\n",
            "assert_contract"
        )),
        BTreeSet::from(["Planted".to_owned()])
    );

    let pascal = "#[derive(Debug)]\n#[cfg_attr(feature = \"serde\", derive(Serialize, Deserialize))]\n\
                  pub enum Pascal {\n    /// Doc.\n    Alpha,\n    Beta(u8),\n    Gamma { x: u8 },\n}\n";
    let found = serde_derived_enums(pascal);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].name, "Pascal");
    assert!(!found[0].tags_are_codes, "a derived enum without renames");

    let multi_line = "#[cfg_attr(\n    feature = \"serde\",\n    derive(Serialize, Deserialize)\n)]\nenum Hidden {\n    Alpha,\n}\n";
    let found = serde_derived_enums(multi_line);
    assert_eq!(found.len(), 1, "a multi-line cfg_attr derive");
    assert!(!found[0].tags_are_codes);

    let partly = "#[derive(Serialize)]\npub enum Partly {\n    #[serde(rename = \"Z69\")]\n    Plus,\n    Minus,\n}\n";
    assert!(
        !serde_derived_enums(partly)[0].tags_are_codes,
        "one variant unrenamed"
    );

    let renamed = "#[derive(Serialize)]\npub enum Renamed {\n    #[cfg_attr(feature = \"serde\", serde(rename = \"Z69\"))]\n    Plus,\n    #[serde(rename = \"Z70\")]\n    Minus,\n}\n";
    assert!(serde_derived_enums(renamed)[0].tags_are_codes);

    let screaming = "#[cfg_attr(feature = \"serde\", derive(Serialize, Deserialize))]\n\
                     #[cfg_attr(feature = \"serde\", serde(rename_all = \"SCREAMING_SNAKE_CASE\"))]\n\
                     pub enum Screaming {\n    Alpha,\n}\n";
    assert!(serde_derived_enums(screaming)[0].tags_are_codes);

    let plain = "#[derive(Debug)]\npub enum Plain {\n    Alpha,\n}\n";
    assert!(
        serde_derived_enums(plain).is_empty(),
        "no serde, no finding"
    );
}
