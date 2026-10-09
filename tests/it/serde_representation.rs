//! The `serde` representation is part of the public API — this file is the lock.
//!
//! Anyone who enables the `serde` feature and writes these values to a database,
//! a Kafka topic or a Parquet file has turned the enum tags into a **wire
//! format**. A variant rename would then be a silent breaking change: nothing
//! fails to compile, and old rows simply stop deserialising.
//!
//! The crate therefore commits to the representation under semver — see the
//! crate-level docs — and this file makes that commitment mechanical. Every
//! assertion below pins a literal tag string. Renaming a variant, changing a
//! `rename_all`, or reordering a struct's fields breaks a test here, in a file
//! whose failure message says what the change costs.
//!
//! Adding a **new** variant is not a breaking change for writers and is allowed
//! within a minor release; it is breaking for *readers* on older versions, which
//! is the usual open-enum trade-off.
//!
//! One rule governs the shapes below: **a type with a canonical `Display` string
//! travels as that string**, never as a second, parallel encoding. `ObisCode`
//! serialises as `"1-0:1.8.0"` and `Resolution` as `"PT15M"` — the same
//! bytes their `Display` writes and their `FromStr` reads. A value with two
//! spellings is a value whose stored keys can disagree with each other;
//! the `string_canonicalisation` module holds that property under proptest.

use metering::{
    MeterInterval, ObisCode, QualityFlag, Resolution, Sparte, Unit, vee::validation::Grade,
};
use rust_decimal::dec;
use time::macros::datetime;

/// Serialise to a compact JSON string.
fn json<T: serde::Serialize>(value: &T) -> String {
    serde_json::to_string(value).expect("serialises")
}

/// The SCREAMING_SNAKE_CASE tags every persisting consumer sees.
#[test]
fn unit_enum_tags_are_stable() {
    let cases: Vec<(String, &str)> = vec![
        (json(&Sparte::Strom), "\"STROM\""),
        (json(&Sparte::Gas), "\"GAS\""),
        (json(&Sparte::Waerme), "\"WAERME\""),
        (json(&Sparte::Wasser), "\"WASSER\""),
        (json(&Unit::KiloWattHour), "\"KWH\""),
        (json(&Unit::CubicMetre), "\"M3\""),
        (json(&QualityFlag::Measured), "\"MEASURED\""),
        (json(&QualityFlag::Estimated), "\"ESTIMATED\""),
        (json(&QualityFlag::Substituted), "\"SUBSTITUTED\""),
        (json(&QualityFlag::Calculated), "\"CALCULATED\""),
        (json(&QualityFlag::Corrected), "\"CORRECTED\""),
        (json(&QualityFlag::Preliminary), "\"PRELIMINARY\""),
        (json(&QualityFlag::Faulty), "\"FAULTY\""),
        (json(&QualityFlag::Unknown), "\"UNKNOWN\""),
    ];
    for (actual, expected) in cases {
        assert_eq!(
            actual, expected,
            "serde tag changed — this is a breaking change for stored data"
        );
    }
}

/// **One string per value.** The serde tag, `as_str`, `Display` and `FromStr`
/// all agree, so a value written through any of them reads back through any
/// other. Two spellings for one value is exactly how a hand-written fixture and
/// a stored row end up disagreeing.
#[test]
fn serde_tags_match_the_as_str_codes() {
    for v in Sparte::ALL {
        assert_eq!(json(&v), format!("\"{}\"", v.as_str()), "{v:?}");
        assert_eq!(v.as_str().parse::<Sparte>().unwrap(), v);
    }
    for v in QualityFlag::ALL {
        assert_eq!(json(&v), format!("\"{}\"", v.as_str()), "{v:?}");
        assert_eq!(v.as_str().parse::<QualityFlag>().unwrap(), v);
    }
    for v in Unit::ALL {
        assert_eq!(json(&v), format!("\"{}\"", v.as_str()), "{v:?}");
        assert_eq!(v.as_str().parse::<Unit>().unwrap(), v);
    }
}

/// An OBIS code travels as its canonical string, not as six separate numbers —
/// and the canonical string omits `*F` when F is 255 ("not applicable").
///
/// See the `string_canonicalisation` module for the invariants behind it.
#[test]
fn obis_code_is_a_string_on_the_wire() {
    assert_eq!(json(&ObisCode::STROM_BEZUG_TOTAL), "\"1-0:1.8.0\"");

    // Both spellings still read, so archives written under either form decode.
    for encoded in ["\"1-0:1.8.0\"", "\"1-0:1.8.0*255\""] {
        let parsed: ObisCode = serde_json::from_str(encoded).unwrap();
        assert_eq!(parsed, ObisCode::STROM_BEZUG_TOTAL, "{encoded}");
    }

    // Medium 6 is heat — the constant and the wire form agree.
    assert_eq!(json(&ObisCode::WAERME_ENERGY), "\"6-0:1.0.0\"");

    // A storage group that carries information is never elided.
    assert_eq!(
        json(&"1-0:1.8.0*1".parse::<ObisCode>().unwrap()),
        "\"1-0:1.8.0*1\""
    );
}

/// `Resolution` travels as its ISO 8601 duration — the same string
/// `Display` writes and `FromStr` reads.
///
/// ISO 8601 is an external standard that no refactor here can rename, so the
/// type has one spelling per value rather than a renameable Rust variant name.
#[test]
fn interval_resolution_shape_is_stable() {
    assert_eq!(json(&Resolution::QUARTER_HOUR), "\"PT15M\"");
    assert_eq!(json(&Resolution::Day), "\"P1D\"");
    assert_eq!(json(&Resolution::Month), "\"P1M\"");
    assert_eq!(json(&Resolution::Year), "\"P1Y\"");
    assert_eq!(json(&Resolution::minutes(5).unwrap()), "\"PT5M\"");

    // ...which is exactly the string form, rather than a parallel convention.
    assert_eq!(Resolution::QUARTER_HOUR.to_string(), "PT15M");
    assert_eq!(
        "PT15M".parse::<Resolution>().unwrap(),
        Resolution::QUARTER_HOUR
    );
}

#[test]
fn quality_grade_is_a_single_letter() {
    for (grade, tag) in [
        (Grade::A, "\"A\""),
        (Grade::B, "\"B\""),
        (Grade::C, "\"C\""),
        (Grade::F, "\"F\""),
    ] {
        assert_eq!(json(&grade), tag);
    }
}

/// The struct every consumer stores most of. Field names are as load-bearing as
/// the enum tags.
#[test]
fn meter_interval_field_names_are_stable() {
    let iv = MeterInterval::new(
        datetime!(2026-01-01 0:00 UTC),
        datetime!(2026-01-01 0:15 UTC),
        dec!(2.5),
        QualityFlag::Measured,
    )
    .unwrap()
    .with_obis(ObisCode::STROM_BEZUG_TOTAL);
    let encoded = json(&iv);
    for field in ["from", "to", "value", "quality", "obis"] {
        assert!(encoded.contains(&format!("\"{field}\"")), "{field} missing");
    }
    assert!(encoded.contains("\"1-0:1.8.0\""));

    let decoded: MeterInterval = serde_json::from_str(&encoded).unwrap();
    assert_eq!(decoded, iv, "round trip");
}

/// Encode, decode, and assert the value survived unchanged.
fn round_trip<T>(v: T)
where
    T: serde::Serialize + serde::de::DeserializeOwned + PartialEq + std::fmt::Debug,
{
    let encoded = json(&v);
    let decoded: T = serde_json::from_str(&encoded)
        .unwrap_or_else(|e| panic!("{v:?} failed to decode from {encoded}: {e}"));
    assert_eq!(decoded, v);
}

/// Everything that goes out must come back in as itself.
#[test]
fn every_enum_round_trips_through_json() {
    for v in Sparte::ALL {
        round_trip(v);
    }
    for v in QualityFlag::ALL {
        round_trip(v);
    }
    for v in Unit::ALL {
        round_trip(v);
    }
    for v in [
        Resolution::QUARTER_HOUR,
        Resolution::HALF_HOUR,
        Resolution::Hour,
        Resolution::Day,
        Resolution::Month,
        Resolution::Year,
        Resolution::minutes(5).unwrap(),
    ] {
        round_trip(v);
    }
    for v in [Grade::A, Grade::B, Grade::C, Grade::F] {
        round_trip(v);
    }
}

// ── the tags, module by module ───────────────────────────────────────────────

/// Assert that every value of a coded enum writes its `as_str` code and reads
/// back as itself.
macro_rules! tags_are_codes {
    ($($ty:ty),+ $(,)?) => {$(
        for v in <$ty>::ALL {
            assert_eq!(json(&v), format!("\"{}\"", v.as_str()), "{v:?}");
            round_trip(v);
        }
    )+};
}

/// `metering::time`: the Bundesland as its ISO 3166-2:DE code without the
/// prefix, the holidays, and the day boundary.
#[test]
fn time_tags_are_pinned() {
    use metering::time::calendar::DayBoundary;
    use metering::time::holiday::{Bundesland, Holiday};

    tags_are_codes!(Bundesland, Holiday, DayBoundary);
    assert_eq!(json(&Bundesland::By), r#""BY""#);
    assert_eq!(json(&Bundesland::Nw), r#""NW""#);
    assert_eq!(json(&Holiday::BussUndBettag), r#""BUSS_UND_BETTAG""#);
    assert_eq!(
        json(&Holiday::ChristiHimmelfahrt),
        r#""CHRISTI_HIMMELFAHRT""#
    );
    assert_eq!(json(&DayBoundary::Strom), r#""STROM""#);
    assert_eq!(json(&DayBoundary::Gas), r#""GAS""#);
}

/// `metering::vee`: the substitution method and reason. The market code is a
/// separate surface from the wire tag, and both are pinned: a consumer
/// generates a database CHECK from one and a MSCONS STS+Z40 from the other.
#[test]
fn vee_tags_are_pinned() {
    use metering::vee::substitute::{Method, SubstitutionReason};

    tags_are_codes!(Method, SubstitutionReason);
    assert_eq!(json(&Method::Vergleichswert), r#""VERGLEICHSWERT""#);
    assert_eq!(
        json(&SubstitutionReason::CommunicationFailure),
        r#""COMMUNICATION_FAILURE""#
    );
    assert_eq!(SubstitutionReason::CommunicationFailure.code(), "Z75");
}

/// `metering::series`: the reading anomaly and the flow direction.
#[test]
fn series_tags_are_pinned() {
    use metering::Direction;
    use metering::series::reading::AnomalyKind;

    tags_are_codes!(AnomalyKind, Direction);
    assert_eq!(
        json(&AnomalyKind::BackwardsWithoutRegisterWidth),
        r#""BACKWARDS_WITHOUT_REGISTER_WIDTH""#
    );
    assert_eq!(json(&Direction::Import), r#""IMPORT""#);
}

/// `metering::slp`: the electricity and gas profile codes.
///
/// The gas codes are the BDEW set — HEF/HMF/HKO, the eleven Gewerbe types and
/// GHD. `"EF"` and `"MF"` are read leniently, for callers migrating stored
/// strings, but never written.
#[test]
fn slp_tags_are_pinned() {
    use metering::slp::gas::GasProfile;
    use metering::slp::strom::LoadProfile;

    tags_are_codes!(LoadProfile, GasProfile);
    assert_eq!(json(&GasProfile::Hef), r#""HEF""#);
    let migrated: GasProfile = serde_json::from_str(r#""EF""#).unwrap();
    assert_eq!(json(&migrated), r#""HEF""#);
    assert!(
        serde_json::from_str::<LoadProfile>(r#""HEF""#).is_err(),
        "a gas code is not an electricity profile"
    );
    assert!(
        serde_json::from_str::<LoadProfile>(r#""GARBAGE""#).is_err(),
        "an unknown profile code is an error, never a silent default"
    );
}

/// `metering::ids`: MaLo and MeLo travel as their canonical strings, with the
/// check digit enforced on the way back in; the issuer and the EIC object
/// type are their codes.
#[test]
fn ids_tags_are_pinned() {
    use metering::ids::{EicType, Issuer};
    use metering::{MaloId, MeloId};

    let malo: MaloId = "41373559241".parse().unwrap();
    assert_eq!(json(&malo), r#""41373559241""#);
    round_trip(malo);
    assert!(serde_json::from_str::<MaloId>(r#""41373559240""#).is_err());

    let melo: MeloId = "DE00056266802AO6G56M11SN51G21M24S".parse().unwrap();
    assert_eq!(json(&melo), r#""DE00056266802AO6G56M11SN51G21M24S""#);
    round_trip(melo);

    tags_are_codes!(Issuer, EicType);
    assert_eq!(json(&Issuer::Gs1), r#""GS1""#);
    // The EIC object type's code *is* its letter — the market writes `10Y…`,
    // never `10AREA…`.
    assert_eq!(json(&EicType::Area), r#""Y""#);
}

/// `metering::grid`: the § 14a vocabulary and the phase. The ones whose Rust
/// name and market spelling disagree are pinned literally, because
/// `SCREAMING_SNAKE_CASE` would split them.
#[test]
fn grid_tags_are_pinned() {
    use metering::grid::para14a::{SteuVeFallgruppe, Verursachungsregel};
    use metering::grid::power_quality::Phase;

    tags_are_codes!(SteuVeFallgruppe, Verursachungsregel, Phase);
    assert_eq!(
        json(&Verursachungsregel::SteuVeZuletzt),
        r#""STEUVE_ZULETZT""#
    );
    assert_eq!(json(&Phase::L1), r#""L1""#);
}

/// `metering::billing`: the Modul 3 conformance vocabulary.
#[test]
fn billing_tags_are_pinned() {
    use metering::billing::zaehlzeit::{Modul3Conformance, Modul3Finding, Quarter};

    tags_are_codes!(Quarter, Modul3Finding, Modul3Conformance);
    assert_eq!(
        json(&Modul3Finding::Modul1NotSelected),
        r#""MODUL_1_NOT_SELECTED""#
    );
    assert_eq!(json(&Quarter::Q1), r#""Q1""#);
}

/// `metering::allocation`: the § 42c sharing vocabulary and the register
/// sample a session is split on.
#[test]
fn allocation_tags_are_pinned() {
    use metering::allocation::session::MeterSample;
    use metering::allocation::sharing::{Bilanzierungsmethode, Zaehlertyp};

    tags_are_codes!(Zaehlertyp, Bilanzierungsmethode);
    assert_eq!(
        json(&Zaehlertyp::IntelligentesMesssystem),
        r#""INTELLIGENTES_MESSSYSTEM""#
    );
    assert_eq!(json(&Bilanzierungsmethode::Rlm), r#""RLM""#);

    let sample = MeterSample::new(datetime!(2026-06-01 12:15 UTC), dec!(1006.5));
    assert_eq!(
        json(&sample),
        r#"{"at":"2026-06-01T12:15:00Z","reading":"1006.5"}"#
    );
}

/// `MeterReading`'s field names, like `MeterInterval`'s above.
#[test]
fn meter_reading_field_names_are_stable() {
    use metering::series::reading::MeterReading;
    use rust_decimal::dec;
    use time::macros::datetime;

    let reading = MeterReading::measured(datetime!(2026-06-01 0:00 UTC), dec!(14230.5));
    let encoded = serde_json::to_string(&reading).unwrap();
    for field in ["at", "value", "quality", "obis_code"] {
        assert!(
            encoded.contains(&format!(r#""{field}":"#)),
            "field {field} missing from {encoded}"
        );
    }
    let decoded: MeterReading = serde_json::from_str(&encoded).unwrap();
    assert_eq!(decoded, reading);
}

/// A tag that no longer exists must fail loudly, not silently pick a default.
#[test]
fn unknown_tags_are_rejected() {
    assert!(serde_json::from_str::<Sparte>("\"KOHLE\"").is_err());
    assert!(serde_json::from_str::<QualityFlag>("\"Mscons\"").is_err());
    // A reversed interval is refused on the way in, like at construction.
    assert!(
        serde_json::from_str::<MeterInterval>(
            r#"{"from":"2026-01-01T00:15:00Z","to":"2026-01-01T00:00:00Z","value":"1","quality":"MEASURED"}"#
        )
        .is_err()
    );
}

/// A 2025 profile table travels as a **list** of day tables, because a JSON
/// object key must be a string and a `(month, day_type)` tuple is not one. The
/// derived map representation compiled and then failed at run time with "key
/// must be a string" — an API that existed only in the type system.
#[test]
fn a_dynamic_slp_profile_travels_as_a_list_of_day_tables() {
    use metering::slp::strom::{DynamicSlpProfile, SlpDayType};

    let mut profile = DynamicSlpProfile::new(metering::slp::strom::LoadProfile::G25);
    profile
        .insert(1, SlpDayType::Samstag, vec![dec!(0.25); 96])
        .unwrap();

    let encoded = json(&profile);
    assert!(
        encoded.contains(r#""days":[{"month":1,"day_type":"SAMSTAG","values":["0.25","#),
        "{encoded}"
    );
    let back: DynamicSlpProfile = serde_json::from_str(&encoded).expect("reads back");
    assert_eq!(back, profile);
}

/// The GHD Summenlastprofil — the fifteenth gas profile type, pinned so its tag
/// cannot drift.
#[test]
fn the_ghd_summenlastprofil_is_on_the_wire() {
    use metering::slp::gas::GasProfile;

    assert_eq!(json(&GasProfile::Ghd), r#""GHD""#);
    assert_eq!(
        serde_json::from_str::<GasProfile>(r#""GHD""#).unwrap(),
        GasProfile::Ghd
    );
    assert_eq!(
        GasProfile::ALL.len(),
        15,
        "the Leitfaden publishes fifteen gas profiles"
    );
}

/// A Berechnungsformel travels with the AHB's own codes — Z69/Z70 signs, Z71/Z72
/// directions — and goes back through `Formula::new`, so a stored formula that
/// no longer validates is refused on the way in.
#[test]
fn a_formula_writes_the_ahb_codes_and_validates_on_the_way_in() {
    use metering::allocation::formula::{FlowDirection, Formula, MeloOperand, SplitFactor};

    let plant: metering::MeloId = "DE0001234567890000000000000000001".parse().unwrap();
    let tenant: metering::MeloId = "DE0001234567890000000000000000002".parse().unwrap();
    let formula =
        Formula::constant_share(tenant, plant, SplitFactor::new(dec!(0.10)).unwrap()).unwrap();
    let encoded = json(&formula);
    let value: serde_json::Value = serde_json::from_str(&encoded).unwrap();
    assert_eq!(value["result"], 3);
    assert_eq!(
        value["steps"]["1"]["PRODUCT"][0]["MELO"]["direction"],
        "Z72"
    );
    assert_eq!(value["steps"]["1"]["PRODUCT"][0]["MELO"]["split"], "0.1");
    assert_eq!(value["steps"]["2"]["SUM"][0][0], "Z70");
    assert_eq!(value["steps"]["3"]["POSITIVE"], 2);
    assert_eq!(serde_json::from_str::<Formula>(&encoded).unwrap(), formula);

    // An orphan step is refused on the way in, as `Formula::new` refuses it.
    let orphan = encoded.replace(r#""result":3"#, r#""result":2"#);
    assert!(serde_json::from_str::<Formula>(&orphan).is_err());
    // So is a factor outside its rules.
    assert!(serde_json::from_str::<SplitFactor>(r#""1.5""#).is_err());
    assert_eq!(
        json(&MeloOperand::new(plant, FlowDirection::Verbrauch).direction),
        r#""Z71""#
    );
}

/// The allocation keys, tagged. `EqualShares` is the statutory doubt case of
/// § 42b Abs. 5 Satz 3 and `Cascading` a contractual shape; both are stored by
/// a consumer alongside the two that were already here, so both are a wire
/// format from the day they exist.
#[test]
fn allocation_key_tags_are_stable() {
    use metering::allocation::community::AllocationKey;
    use std::collections::BTreeMap;

    let t1: metering::MeloId = "DE0001234567890000000000000000001".parse().unwrap();
    assert_eq!(
        json(&AllocationKey::EqualShares),
        r#"{"kind":"EQUAL_SHARES"}"#
    );
    assert_eq!(
        json(&AllocationKey::Proportional),
        r#"{"kind":"PROPORTIONAL"}"#
    );
    assert_eq!(
        json(&AllocationKey::Cascading {
            weights: BTreeMap::from([(t1, dec!(1))]),
        }),
        r#"{"kind":"CASCADING","weights":{"DE0001234567890000000000000000001":"1"}}"#
    );
}

/// The four Regelzonen, and the control-area codes they carry.
#[test]
fn regelzone_tags_are_pinned() {
    use metering::ids::Regelzone;

    for v in Regelzone::ALL {
        assert_eq!(json(&v), format!("\"{}\"", v.as_str()), "{v:?}");
    }
    // `SCREAMING_SNAKE_CASE` would write TENNE_T; the operator's own spelling
    // has an inner capital and the code must not inherit it.
    assert_eq!(json(&Regelzone::TenneT), r#""TENNET""#);
    assert_eq!(json(&Regelzone::FiftyHertz), r#""FIFTY_HERTZ""#);
    assert_eq!(
        json(&Regelzone::TransnetBw.control_area_eic()),
        r#""10YDE-ENBW-----N""#
    );
}

/// An EIC travels as its sixteen characters, like every other identifier here
/// — and the check character is enforced on the way back in.
#[test]
fn an_eic_is_a_string_on_the_wire() {
    use metering::ids::Eic;

    let eic: Eic = "10YDE-VE-------2".parse().unwrap();
    assert_eq!(json(&eic), r#""10YDE-VE-------2""#);
    assert_eq!(json(&eic), format!("\"{eic}\""), "serde is Display");

    let back: Eic = serde_json::from_str(r#""10YDE-VE-------2""#).expect("reads back");
    assert_eq!(back, eic);

    assert!(serde_json::from_str::<Eic>(r#""10YDE-VE-------3""#).is_err());
}

/// A Marktpartner-ID travels as its digits, like every other identifier here.
#[test]
fn a_bdew_code_is_a_string_on_the_wire() {
    use metering::ids::BdewCode;

    let code: BdewCode = "9900987654329".parse().unwrap();
    assert_eq!(json(&code), r#""9900987654329""#);
    assert_eq!(json(&code), format!("\"{code}\""), "serde is Display");

    let back: BdewCode = serde_json::from_str(r#""9900987654329""#).expect("reads back");
    assert_eq!(back, code);

    // Thirteen digits or nothing — the structure is enforced on the way in.
    assert!(serde_json::from_str::<BdewCode>(r#""99009876543""#).is_err());
}

/// The § 14a inputs and parameters, field by field.
#[test]
fn para14a_field_names_are_stable() {
    use metering::grid::para14a::{Anlage, Para14aConfig, SteuVeFallgruppe};

    let device = Anlage::new(SteuVeFallgruppe::Waermepumpe, dec!(20));
    assert_eq!(
        json(&device),
        r#"{"fallgruppe":"WAERMEPUMPE","netzanschlussleistung_kw":"20"}"#,
    );

    // Only the presumptive parameters travel; 4,2 kW and 11 kW are constants.
    let cfg = Para14aConfig::VERMUTUNG;
    let value: serde_json::Value = serde_json::from_str(&json(&cfg)).unwrap();
    assert_eq!(value["skalierungsfaktor"], "0.4");
    assert_eq!(value["gleichzeitigkeitsfaktoren"][0], "0.80");
    assert!(value.get("mindestleistung_kw").is_none());
}

/// Per-phase apparent power, field by field.
#[test]
fn phase_apparent_power_field_names_are_stable() {
    use metering::grid::power_quality::{Phase, PhaseApparentPower};

    let p = PhaseApparentPower::single_phase(Phase::L2, dec!(4.6));
    assert_eq!(json(&p), r#"{"l1_kva":"0","l2_kva":"4.6","l3_kva":"0"}"#);
}

/// An allocation key carries one discriminator at a fixed path — a
/// settlement stores it and queries on it.
#[test]
fn an_allocation_key_carries_one_discriminator() {
    use metering::allocation::community::AllocationKey;
    use metering::allocation::formula::SplitFactor;
    use std::collections::BTreeMap;

    let t1: metering::MeloId = "DE0001234567890000000000000000001".parse().unwrap();
    let constant = AllocationKey::Constant {
        fractions: BTreeMap::from([(t1, SplitFactor::new(dec!(0.25)).unwrap())]),
    };
    let value: serde_json::Value = serde_json::from_str(&json(&constant)).unwrap();
    assert_eq!(value["kind"], "CONSTANT");
    assert_eq!(value["fractions"][t1.as_str()], "0.25");

    for key in [
        constant,
        AllocationKey::Proportional,
        AllocationKey::EqualShares,
    ] {
        let back: AllocationKey = serde_json::from_str(&json(&key)).expect("round trips");
        assert_eq!(back, key);
    }
}

/// A community allocation is a settlement record, so every field name it
/// carries is part of the wire format.
#[test]
fn community_allocation_field_names_are_stable() {
    use metering::allocation::community::{AllocationKey, allocate};
    use metering::{DayBoundary, MeloId, Resolution, Series};

    let one = |kwh| {
        Series::new(
            Resolution::QUARTER_HOUR,
            DayBoundary::Strom,
            vec![
                MeterInterval::quarter_hour(
                    datetime!(2026-06-01 12:00 UTC),
                    kwh,
                    QualityFlag::Measured,
                )
                .unwrap(),
            ],
        )
        .unwrap()
    };
    let t1: MeloId = "DE0001234567890000000000000000001".parse().unwrap();
    let (plant, tenant) = (one(dec!(10)), one(dec!(1)));
    let out = allocate(&plant, &[(t1, &tenant)], &AllocationKey::Proportional).unwrap();
    let value: serde_json::Value = serde_json::from_str(&json(&out[0])).unwrap();

    assert_eq!(value["from"], "2026-06-01T12:00:00Z");
    assert_eq!(value["to"], "2026-06-01T12:15:00Z");
    assert_eq!(value["generation"], "10");
    assert_eq!(value["residual"], "9");
    assert_eq!(value["quality"], "MEASURED");
    assert_eq!(value["shares"][0]["participant"], t1.as_str());
    assert_eq!(value["shares"][0]["consumption"], "1");
    assert_eq!(value["shares"][0]["share"], "10");
    assert_eq!(value["shares"][0]["allocated"], "1");
}

/// Instants travel as **RFC 3339** and dates as **ISO 8601** — the spellings a
/// `TIMESTAMPTZ` cast, a JSON Schema `format: date-time` and every log viewer
/// already understand.
///
/// `time`'s own representation — `[2026, 152, 12, 0, 0, 0, 0, 0, 0]`, the
/// year, the **ordinal day**, the clock and the offset — is stable and
/// deliberately compact, and unusable as a stored one: `WHERE from >
/// '2026-06-01'` has no meaning against an ordinal tuple, and no schema
/// language recognises it.
#[test]
fn instants_are_rfc3339_and_dates_are_iso8601() {
    use metering::grid::power_quality::PowerQualityInterval;
    use metering::series::reading::MeterReading;

    let iv = MeterInterval::new(
        datetime!(2026-06-01 12:00 UTC),
        datetime!(2026-06-01 12:15 UTC),
        dec!(2.5),
        QualityFlag::Measured,
    )
    .unwrap()
    .with_obis(ObisCode::STROM_BEZUG_TOTAL);
    assert_eq!(
        json(&iv),
        r#"{"from":"2026-06-01T12:00:00Z","to":"2026-06-01T12:15:00Z","value":"2.5","quality":"MEASURED","obis":"1-0:1.8.0"}"#,
    );

    // A sub-second instant keeps its precision.
    let precise = MeterReading::measured(datetime!(2026-10-25 00:30:15.25 UTC), dec!(1000));
    let value: serde_json::Value = serde_json::from_str(&json(&precise)).unwrap();
    assert_eq!(value["at"], "2026-10-25T00:30:15.25Z");

    // Every instant-bearing type, not just the hot one.
    let pq = PowerQualityInterval::empty(
        datetime!(2026-06-01 0:00 UTC),
        datetime!(2026-06-01 0:10 UTC),
    );
    let value: serde_json::Value = serde_json::from_str(&json(&pq)).unwrap();
    assert_eq!(value["from"], "2026-06-01T00:00:00Z");

    // Dates are dates, not midnight instants: a validity bound is a German
    // calendar day and carries no time and no offset.
    let milestone = metering::grid::rollout::ROLLOUT_MILESTONES
        .iter()
        .find(|m| m.window_from().is_some())
        .expect("a flow milestone");
    let value: serde_json::Value = serde_json::from_str(&json(milestone)).unwrap();
    assert_eq!(value["deadline"], "2026-12-31");
    assert_eq!(value["window_from"], "2025-02-25");

    // `None` is `null`, not a missing key or an empty string.
    let stock = metering::grid::rollout::ROLLOUT_MILESTONES
        .iter()
        .find(|m| m.window_from().is_none())
        .expect("a stock milestone");
    let value: serde_json::Value = serde_json::from_str(&json(stock)).unwrap();
    assert!(value["window_from"].is_null());
}

/// Every instant- and date-bearing type reads back what it wrote, through both
/// a human-readable format and a binary one.
#[test]
fn timestamps_round_trip_through_json_and_postcard() {
    use metering::series::reading::MeterReading;

    let iv = MeterInterval::new(
        datetime!(2026-06-01 12:00 UTC),
        datetime!(2026-06-01 12:15 UTC),
        dec!(2.5),
        QualityFlag::Measured,
    )
    .unwrap();
    let reading = MeterReading::measured(datetime!(2026-03-29 01:00 UTC), dec!(42.125));

    macro_rules! both_ways {
        ($value:expr) => {{
            let v = $value;
            let text = serde_json::to_string(&v).expect("json");
            assert_eq!(
                serde_json::from_str::<_>(&text).ok(),
                Some(v.clone()),
                "json"
            );
            let bytes = postcard::to_allocvec(&v).expect("postcard");
            assert_eq!(postcard::from_bytes::<_>(&bytes).ok(), Some(v), "postcard");
            bytes
        }};
    }

    let interval_bytes = both_ways!(iv);
    both_ways!(reading);

    // The binary form is the compact tuple, not the string: an RFC 3339
    // timestamp is twenty bytes, and `MeterInterval` carries two of them. The
    // split on `is_human_readable` is what keeps the hot type cheap in the
    // formats a binary encoding is chosen for.
    assert!(
        !interval_bytes.windows(4).any(|w| w == b"2026"),
        "postcard must not carry the textual year: {interval_bytes:?}",
    );
    assert!(
        interval_bytes.len() < 40,
        "two instants, a Decimal and two enums in {} bytes",
        interval_bytes.len(),
    );
}

/// The hot types round-trip through a **non-self-describing** binary format,
/// and the internally-tagged configuration types deliberately do not.
///
/// `deserialize_any` is the one question postcard and bincode cannot answer, so
/// every field of a hot type names its own representation through
/// `crate::wire`. An internal tag needs `deserialize_any` by construction — the
/// documented price of a discriminator at a fixed, queryable path.
#[test]
fn the_hot_types_survive_a_binary_format_and_the_tagged_ones_do_not() {
    use metering::allocation::community::AllocationKey;

    // Hot path: intervals, channels, readings.
    let iv = MeterInterval::new(
        datetime!(2026-06-01 12:00 UTC),
        datetime!(2026-06-01 12:15 UTC),
        dec!(2.5),
        QualityFlag::Measured,
    )
    .unwrap()
    .with_obis(ObisCode::STROM_BEZUG_TOTAL);
    let bytes = postcard::to_allocvec(&iv).expect("serialises");
    assert_eq!(
        postcard::from_bytes::<MeterInterval>(&bytes).expect("reads back"),
        iv,
    );

    let code = ObisCode::GAS_BRENNWERT_MONATSMITTEL;
    let bytes = postcard::to_allocvec(&code).expect("serialises");
    assert_eq!(postcard::from_bytes::<ObisCode>(&bytes).unwrap(), code);

    // A quantity travels as a string in a binary format too, and the sign and
    // the scale survive it. The field says so itself — a *bare*
    // `rust_decimal::Decimal` still deserialises however the consumer's own
    // feature selection says, which is the point: this crate does not reach
    // across the build graph to decide that.
    let negative = metering::series::reading::MeterReading::measured(
        datetime!(2026-06-01 0:00 UTC),
        dec!(-12345.6789),
    );
    let bytes = postcard::to_allocvec(&negative).expect("serialises");
    assert_eq!(
        postcard::from_bytes::<metering::series::reading::MeterReading>(&bytes)
            .expect("reads back"),
        negative,
    );

    // Configuration: internal tagging needs a self-describing format, which is
    // the documented cost of putting the discriminator at a fixed, queryable
    // path. Pinned so the trade-off cannot silently move in either direction.
    let key = AllocationKey::Proportional;
    let bytes = postcard::to_allocvec(&key).expect("serialises");
    assert!(
        postcard::from_bytes::<AllocationKey>(&bytes).is_err(),
        "internally tagged types are JSON-shaped on purpose",
    );
}

/// A quantity is its exact decimal string, and a JSON **number** is refused.
///
/// `0.1` is not representable in binary floating point, so accepting the number
/// would mean rounding it and carrying that through every conservation identity
/// the crate advertises. The scale survives too: `"2.50"` is a quantity
/// reported to two decimal places and stays one.
#[test]
fn a_quantity_is_a_string_and_a_json_number_is_refused() {
    let interval = MeterInterval::new(
        datetime!(2026-06-01 12:00 UTC),
        datetime!(2026-06-01 12:15 UTC),
        dec!(2.50),
        QualityFlag::Measured,
    )
    .unwrap();
    let encoded = json(&interval);
    assert!(encoded.contains(r#""value":"2.50""#), "{encoded}");
    let back: MeterInterval = serde_json::from_str(&encoded).expect("reads back");
    assert_eq!(back.value().scale(), 2, "the reported precision survives");
    assert_eq!(back, interval);

    let as_number = encoded.replace(r#""value":"2.50""#, r#""value":2.50"#);
    let refused = serde_json::from_str::<MeterInterval>(&as_number)
        .expect_err("a float cannot hold an exact quantity");
    assert!(
        refused.to_string().contains("string"),
        "the message should say what was expected: {refused}",
    );

    // More digits than a `Decimal` holds are refused rather than rounded away.
    let too_precise = encoded.replace(r#""2.50""#, r#""2.5000000000000000000000000000001""#);
    assert!(
        serde_json::from_str::<MeterInterval>(&too_precise).is_err(),
        "silently dropping digits is how a conservation identity stops holding",
    );
}

/// The same representation inside a sequence, an array and a map.
///
/// `serde(with)` names functions over the field's own type, so a container
/// needs its own module or its elements fall back to the inherited impl. These
/// three are the crate's only container-shaped quantities.
#[test]
fn quantities_in_containers_are_strings_too() {
    use metering::allocation::community::AllocationKey;
    use metering::slp::gas::WeekdayFactors;
    use metering::slp::strom::{DynamicSlpProfile, SlpDayType};

    let mut profile = DynamicSlpProfile::new(metering::slp::strom::LoadProfile::G25);
    let mut day = vec![dec!(0.125); 96];
    day[0] = dec!(0.25);
    profile.insert(1, SlpDayType::Werktag, day).unwrap();
    assert!(
        json(&profile).contains(r#""values":["0.25","0.125","#),
        "{}",
        json(&profile)
    );

    let factors = WeekdayFactors::new([
        dec!(1.0253),
        dec!(1.0253),
        dec!(1.0253),
        dec!(1.0253),
        dec!(1.0253),
        dec!(0.9235),
        dec!(0.9500),
    ]);
    let factors = factors.expect("the seven factors sum to seven");
    let encoded = json(&factors);
    assert!(encoded.contains(r#""1.0253""#), "{encoded}");
    assert_eq!(
        serde_json::from_str::<WeekdayFactors>(&encoded).expect("reads back"),
        factors,
    );

    // An array is a tuple to `serde` and a sequence is not, so the two halves
    // of `decimal_array` have to agree about the length prefix. Only a format
    // that omits it can tell the difference.
    let bytes = postcard::to_allocvec(&factors).expect("serialises");
    assert_eq!(
        postcard::from_bytes::<WeekdayFactors>(&bytes).expect("reads back"),
        factors,
    );

    let t1: metering::MeloId = "DE0001234567890000000000000000001".parse().unwrap();
    let key = AllocationKey::Cascading {
        weights: [(t1, dec!(0.10))].into_iter().collect(),
    };
    let encoded = json(&key);
    assert!(
        encoded.contains(r#""DE0001234567890000000000000000001":"0.10""#),
        "{encoded}"
    );
    assert_eq!(
        serde_json::from_str::<AllocationKey>(&encoded).expect("reads back"),
        key,
    );
}

/// Every `Decimal` this crate can hold survives both wire formats, exactly.
///
/// The deserialiser is strict, and a strict reader that cannot read its own
/// writer is worse than a lenient one. Asserted over the *content* — sign and
/// scale included — because a hand-picked value does not reach the mantissa and
/// scale extremes where an exact parse would fail.
#[test]
fn every_quantity_round_trips_through_json_and_postcard() {
    use proptest::prelude::*;
    use rust_decimal::Decimal;

    let quantity = (
        proptest::num::i128::ANY.prop_map(|m| m % (1i128 << 96)),
        0u32..=28,
    )
        .prop_map(|(mantissa, scale)| Decimal::from_i128_with_scale(mantissa, scale));

    proptest!(|(value in quantity)| {
        let reading = metering::series::reading::MeterReading::measured(
            datetime!(2026-06-01 0:00 UTC),
            value,
        );

        let encoded = json(&reading);
        let back: metering::series::reading::MeterReading =
            serde_json::from_str(&encoded).expect("JSON reads back");
        prop_assert_eq!(back.value, value);
        prop_assert_eq!(back.value.scale(), value.scale(), "the reported precision survives");

        let bytes = postcard::to_allocvec(&reading).expect("serialises");
        let back: metering::series::reading::MeterReading =
            postcard::from_bytes(&bytes).expect("postcard reads back");
        prop_assert_eq!(back.value, value);
        prop_assert_eq!(back.value.scale(), value.scale());
    });
}

/// A validation report is data: it round-trips through both formats, with
/// instants as RFC 3339 and quantities as strings.
#[test]
fn a_validation_report_round_trips() {
    use metering::vee::validation::{Report, Rules, validate};
    use metering::{DayBoundary, Series};
    use time::macros::{date, datetime};

    let day = DayBoundary::Strom.day(date!(2026 - 06 - 01)).unwrap();
    let ivs = (0..95)
        .map(|i| {
            MeterInterval::quarter_hour(
                day.start() + time::Duration::minutes(15 * i),
                dec!(1.5),
                QualityFlag::Measured,
            )
            .unwrap()
        })
        .collect();
    let series = Series::new(Resolution::QUARTER_HOUR, DayBoundary::Strom, ivs).unwrap();
    let report = validate(
        &series,
        &Rules::strom(day, datetime!(2026-07-01 0:00 UTC), None),
    );
    let json = serde_json::to_string(&report).unwrap();
    assert!(json.contains(r#""rule":"GAP""#), "{json}");
    assert!(json.contains(r#""from":"2026-06-01T21:45:00Z""#), "{json}");
    assert_eq!(serde_json::from_str::<Report>(&json).unwrap(), report);
    let bytes = postcard::to_allocvec(&report).unwrap();
    assert_eq!(postcard::from_bytes::<Report>(&bytes).unwrap(), report);
}
