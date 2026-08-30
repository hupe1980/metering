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

use metering::*;

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
    use metering::calendar::{DayBoundary, DayKind};
    use metering::classification::SeriesOrigin;
    use metering::conversion::G685FinalRounding;
    use metering::holiday::Holiday;
    use metering::ids::{CodeVergabestelle, MaloIssuer};
    use metering::lifecycle::{MeterLifecycleEventType, MeterStatus};
    use metering::load_profile::SlpDayType;
    use metering::measurement_series::ProvenanceEventType;
    use metering::obis::RegisterUnit;
    use metering::reading::AnomalyKind;
    use metering::rollout::QuotaScope;
    use metering::zaehlzeit::{DayGroup, Modul3Conformance, Modul3Finding, Quarter};

    // interval / quantities
    assert_contract!(Sparte);
    assert_contract!(QualityFlag);

    // identifiers and channels
    assert_contract!(MaloIssuer);
    assert_contract!(CodeVergabestelle);
    assert_contract!(RegisterUnit);
    assert_contract!(Phase);

    // calendar and profiles
    assert_contract!(DayKind);
    assert_contract!(DayBoundary);
    assert_contract!(Bundesland);
    assert_contract!(Holiday);
    assert_contract!(LoadProfile);
    assert_contract!(SlpDayType);

    // pipeline
    assert_contract!(AnomalyKind);
    assert_contract!(ValidationRuleId);
    assert_contract!(ValidationSeverity);
    assert_contract!(QualityGrade);
    assert_contract!(SubstituteMethod);
    assert_contract!(SubstitutionReason);
    assert_contract!(Messtyp);
    assert_contract!(SeriesOrigin);
    assert_contract!(DayGroup);
    assert_contract!(Quarter);
    assert_contract!(Modul3Finding);
    assert_contract!(Modul3Conformance);
    assert_contract!(G685FinalRounding);

    // master data and lifecycle
    assert_contract!(MarktRolle);
    assert_contract!(EnergyFlow);
    assert_contract!(ProvenanceEventType);
    assert_contract!(MeterStatus);
    assert_contract!(MeterLifecycleEventType);
    assert_contract!(VirtualMeterKind);

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
}

/// `MeasurementUnit` is the one type whose `FromStr` is deliberately wider than
/// its `CODES`: it reads `m³`, `kWh_th` and the UN/ECE Rec 20 codes, and writes
/// exactly two. The write half of the contract still holds.
#[test]
fn measurement_unit_writes_two_codes_and_reads_many() {
    assert_eq!(MeasurementUnit::ALL.len(), MeasurementUnit::CODES.len());
    for (v, code) in MeasurementUnit::ALL.iter().zip(MeasurementUnit::CODES) {
        assert_eq!(v.as_str(), *code);
        assert_eq!(&v.to_string(), *code);
        assert_eq!(&code.parse::<MeasurementUnit>().unwrap(), v);
        #[cfg(feature = "serde")]
        assert_eq!(serde_json::to_string(v).unwrap(), format!("\"{code}\""));
    }
    // ...and the wider read set, which is the point of the asymmetry.
    for wide in ["m³", "kWh_th", "MTQ", "cbm"] {
        assert!(wide.parse::<MeasurementUnit>().is_ok(), "{wide}");
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
    assert_eq!("EF".parse::<LoadProfile>().unwrap(), LoadProfile::GasHEF);
    assert_eq!("MF".parse::<LoadProfile>().unwrap(), LoadProfile::GasHMF);
    assert_eq!(LoadProfile::GasHEF.to_string(), "HEF");

    // The BDEW Rollenmodell spells it ÜNB; a stored code should not need an
    // umlaut, so `abbreviation()` keeps it and `as_str()` does not.
    assert_eq!("ÜNB".parse::<MarktRolle>().unwrap(), MarktRolle::Uenb);
    assert_eq!(MarktRolle::Uenb.abbreviation(), "ÜNB");
    assert_eq!(MarktRolle::Uenb.as_str(), "UENB");
}

/// A code and a human-facing description are different things, and the types
/// that have both keep them apart.
#[test]
fn display_forms_are_codes_and_descriptions_are_prose() {
    use metering::holiday::Holiday;
    use metering::obis::RegisterUnit;

    assert_eq!(Holiday::BussUndBettag.as_str(), "BUSS_UND_BETTAG");
    assert_eq!(Holiday::BussUndBettag.name(), "Buß- und Bettag");

    assert_eq!(RegisterUnit::KiloWattHour.as_str(), "KILO_WATT_HOUR");
    assert_eq!(RegisterUnit::KiloWattHour.symbol(), "kWh");

    assert_eq!(SubstituteMethod::ZeroFill.as_str(), "ZERO_FILL");
    assert_eq!(
        SubstituteMethod::ZeroFill.description(),
        "Nullwert (dokumentierter Lieferstopp)"
    );

    // Every description is non-empty, and none of them is the code.
    for m in SubstituteMethod::ALL {
        assert!(!m.description().is_empty());
        assert_ne!(m.description(), m.as_str());
    }
    for r in SubstitutionReason::ALL {
        assert!(!r.description().is_empty());
        assert_ne!(r.description(), r.as_str());
    }
}

/// The `Vnn` rule code is the only spelling — `Display`, `as_str` and the
/// `serde` tag all say `V01` — so a stored finding reads back through the
/// vocabulary it was written with.
#[test]
fn validation_rule_ids_are_the_vnn_codes_everywhere() {
    assert_eq!(ValidationRuleId::GapDetected.as_str(), "V01");
    assert_eq!(ValidationRuleId::ImplausiblePower.to_string(), "V12");
    assert_eq!(
        "v01".parse::<ValidationRuleId>().unwrap(),
        ValidationRuleId::GapDetected
    );
    #[cfg(feature = "serde")]
    assert_eq!(
        serde_json::to_string(&ValidationRuleId::DstAmbiguity).unwrap(),
        "\"V07\""
    );

    // V10 is retired and must not parse — the number is left unused so a stored
    // `V10` row cannot be silently reinterpreted as something else.
    assert!("V10".parse::<ValidationRuleId>().is_err());
    assert!(!ValidationRuleId::CODES.contains(&"V10"));
    assert_eq!(ValidationRuleId::CODES.len(), 11);
}

/// Every `string_codes!` type appears in [`every_coded_enum_holds_the_contract`].
///
/// The macro is the crate's single definition of "this is a coded enum", so the
/// set of types it is applied to is the set the contract must cover. Reading it
/// out of the source is crude, and it is the only way to make the list above
/// self-maintaining: nothing in Rust lets a test enumerate the types a macro was
/// invoked on, so without this a new enum joins the crate with no contract at
/// all and every existing assertion still passes.
///
/// This is a test, so it may read files; the no-I/O guarantee is about `src/`.
#[test]
fn no_coded_enum_escapes_this_file() {
    use std::collections::BTreeSet;

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));

    // Types the macro is applied to, from every `string_codes! { … }` block.
    let mut coded: BTreeSet<String> = BTreeSet::new();
    let src = std::fs::read_dir(root.join("src")).expect("src/ is readable");
    for entry in src {
        let path = entry.expect("readable entry").path();
        if path.extension().is_none_or(|e| e != "rs") {
            continue;
        }
        let text = std::fs::read_to_string(&path).expect("readable source");
        let mut rest = text.as_str();
        while let Some(start) = rest.find("crate::codes::string_codes! {") {
            let body = &rest[start..];
            let open = body.find('{').expect("the macro body opens");
            let close = body.find("\n}").expect("the macro body closes");
            for line in body[open + 1..close].lines() {
                let line = line.trim();
                if line.is_empty() || line.starts_with("//") {
                    continue;
                }
                // `Ty;` or `Ty, aliases = [...];`
                let name = line
                    .split([',', ';'])
                    .next()
                    .unwrap_or_default()
                    .trim()
                    .to_owned();
                if name.starts_with(char::is_uppercase) {
                    coded.insert(name);
                }
            }
            rest = &body[close..];
        }
    }
    assert!(
        coded.len() > 30,
        "the scan found only {} types — it has stopped working, not the crate",
        coded.len()
    );

    // Types this file asserts the contract for.
    let this_file = std::fs::read_to_string(root.join("tests/code_contract.rs"))
        .expect("this test file is readable");
    let asserted: BTreeSet<String> = this_file
        .match_indices("assert_contract!(")
        .filter_map(|(i, m)| {
            let tail = &this_file[i + m.len()..];
            tail.find(')').map(|end| tail[..end].trim().to_owned())
        })
        // The macro name also occurs inside this file's own prose; only a
        // type-shaped argument is a real invocation.
        .filter(|name| {
            name.starts_with(char::is_uppercase)
                && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        })
        .collect();

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
