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
//! serialises as `"1-0:1.8.0"` and `IntervalResolution` as `"PT15M"` — the same
//! bytes their `Display` writes and their `FromStr` reads. A value with two
//! spellings is a value whose stored keys can disagree with each other;
//! `tests/string_canonicalisation.rs` holds that property under proptest.

#![cfg(feature = "serde")]

use metering::{
    IntervalResolution, MeasurementSource, MeasurementUnit, MeterInterval, ObisCode,
    ProvenanceEventType, QualityFlag, QualityGrade, Sparte,
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
        (json(&MeasurementUnit::KiloWattHour), "\"KWH\""),
        (json(&MeasurementUnit::CubicMetre), "\"M3\""),
        (json(&QualityFlag::Measured), "\"MEASURED\""),
        (json(&QualityFlag::Estimated), "\"ESTIMATED\""),
        (json(&QualityFlag::Substituted), "\"SUBSTITUTED\""),
        (json(&QualityFlag::Calculated), "\"CALCULATED\""),
        (json(&QualityFlag::Corrected), "\"CORRECTED\""),
        (json(&QualityFlag::Preliminary), "\"PRELIMINARY\""),
        (json(&QualityFlag::Faulty), "\"FAULTY\""),
        (json(&QualityFlag::Unknown), "\"UNKNOWN\""),
        (json(&ProvenanceEventType::Ingested), "\"INGESTED\""),
        (
            json(&ProvenanceEventType::QualityAssessed),
            "\"QUALITY_ASSESSED\"",
        ),
        (
            json(&ProvenanceEventType::SubstituteGenerated),
            "\"SUBSTITUTE_GENERATED\"",
        ),
        (json(&ProvenanceEventType::Corrected), "\"CORRECTED\""),
        (json(&ProvenanceEventType::Archived), "\"ARCHIVED\""),
        (json(&ProvenanceEventType::Anonymised), "\"ANONYMISED\""),
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
    for v in MeasurementUnit::ALL {
        assert_eq!(json(&v), format!("\"{}\"", v.as_str()), "{v:?}");
        assert_eq!(v.as_str().parse::<MeasurementUnit>().unwrap(), v);
    }
}

/// Externally-tagged data-carrying variants: the tag *and* the field names are
/// load-bearing.
#[test]
fn measurement_source_shape_is_stable() {
    let mscons = MeasurementSource::Mscons {
        pid: 13005,
        message_ref: Some("REF-1".to_owned()),
        sender_mp_id: "9900357000004".parse().unwrap(),
    };
    assert_eq!(
        json(&mscons),
        r#"{"MSCONS":{"pid":13005,"message_ref":"REF-1","sender_mp_id":"9900357000004"}}"#
    );

    let push = MeasurementSource::SmgwDirectPush {
        device_id: "SMGW-001".to_owned(),
        session_id: "sess-1".to_owned(),
    };
    assert_eq!(
        json(&push),
        r#"{"SMGW_DIRECT_PUSH":{"device_id":"SMGW-001","session_id":"sess-1"}}"#
    );

    let redispatch = MeasurementSource::RedispatchImport {
        pid: 13022,
        activation_ref: None,
    };
    assert_eq!(
        json(&redispatch),
        r#"{"REDISPATCH_IMPORT":{"pid":13022,"activation_ref":null}}"#
    );
}

/// An OBIS code travels as its canonical string, not as six separate numbers —
/// and the canonical string omits `*F` when F is 255 ("not applicable").
///
/// See `tests/string_canonicalisation.rs` for the invariants behind it.
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

/// `IntervalResolution` travels as its ISO 8601 duration — the same string
/// `Display` writes and `FromStr` reads.
///
/// ISO 8601 is an external standard that no refactor here can rename, so the
/// type has one spelling per value rather than a renameable Rust variant name.
#[test]
fn interval_resolution_shape_is_stable() {
    assert_eq!(json(&IntervalResolution::QuarterHour), "\"PT15M\"");
    assert_eq!(json(&IntervalResolution::Day), "\"P1D\"");
    assert_eq!(json(&IntervalResolution::Month), "\"P1M\"");
    assert_eq!(json(&IntervalResolution::Year), "\"P1Y\"");
    assert_eq!(
        json(&IntervalResolution::from_seconds(300).unwrap()),
        "\"PT300S\""
    );

    // ...which is exactly the string form, rather than a parallel convention.
    assert_eq!(IntervalResolution::QuarterHour.to_string(), "PT15M");
    assert_eq!(
        "PT15M".parse::<IntervalResolution>().unwrap(),
        IntervalResolution::QuarterHour
    );
}

#[test]
fn quality_grade_is_a_single_letter() {
    for (grade, tag) in [
        (QualityGrade::A, "\"A\""),
        (QualityGrade::B, "\"B\""),
        (QualityGrade::C, "\"C\""),
        (QualityGrade::F, "\"F\""),
    ] {
        assert_eq!(json(&grade), tag);
    }
}

/// The struct every consumer stores most of. Field names are as load-bearing as
/// the enum tags.
#[test]
fn meter_interval_field_names_are_stable() {
    let iv = MeterInterval {
        from: datetime!(2026-01-01 0:00 UTC),
        to: datetime!(2026-01-01 0:15 UTC),
        value: dec!(2.5),
        quality: QualityFlag::Measured,
        obis_code: Some(ObisCode::STROM_BEZUG_TOTAL),
    };
    let encoded = json(&iv);
    for field in ["from", "to", "value", "quality", "obis_code"] {
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
    for v in MeasurementUnit::ALL {
        round_trip(v);
    }
    for v in [
        IntervalResolution::QuarterHour,
        IntervalResolution::HalfHour,
        IntervalResolution::Hour,
        IntervalResolution::Day,
        IntervalResolution::Month,
        IntervalResolution::Year,
        IntervalResolution::from_seconds(300).unwrap(),
    ] {
        round_trip(v);
    }
    for v in [
        QualityGrade::A,
        QualityGrade::B,
        QualityGrade::C,
        QualityGrade::F,
    ] {
        round_trip(v);
    }
    for v in [
        ProvenanceEventType::Ingested,
        ProvenanceEventType::QualityAssessed,
        ProvenanceEventType::SubstituteGenerated,
        ProvenanceEventType::Corrected,
        ProvenanceEventType::Archived,
        ProvenanceEventType::Anonymised,
    ] {
        round_trip(v);
    }
}

/// The tags introduced in 0.17. Same commitment as everything above: these are
/// a wire format, so a rename is a breaking change.
#[test]
fn tags_added_in_0_17_are_pinned() {
    use metering::classification::SeriesOrigin;
    use metering::holiday::{Bundesland, Holiday};
    use metering::reading::AnomalyKind;
    use metering::sharing::{Bilanzierungsmethode, Zaehlertyp};
    use metering::{SubstituteMethod, SubstitutionReason, VirtualMeterKind};

    // Bundesland — the ISO 3166-2:DE subdivision code, uppercase, no prefix.
    assert_eq!(serde_json::to_string(&Bundesland::By).unwrap(), r#""BY""#);
    assert_eq!(serde_json::to_string(&Bundesland::Nw).unwrap(), r#""NW""#);
    for land in Bundesland::ALL {
        let encoded = serde_json::to_string(&land).unwrap();
        assert_eq!(encoded, format!(r#""{}""#, land.as_str()));
        assert_eq!(
            serde_json::from_str::<Bundesland>(&encoded).unwrap(),
            land,
            "{land} must round-trip"
        );
    }

    assert_eq!(
        serde_json::to_string(&Holiday::BussUndBettag).unwrap(),
        r#""BUSS_UND_BETTAG""#
    );
    assert_eq!(
        serde_json::to_string(&Holiday::ChristiHimmelfahrt).unwrap(),
        r#""CHRISTI_HIMMELFAHRT""#
    );
    assert_eq!(
        serde_json::to_string(&VirtualMeterKind::GgvProportionalAllocation).unwrap(),
        r#""GGV_PROPORTIONAL_ALLOCATION""#
    );
    assert_eq!(
        serde_json::to_string(&AnomalyKind::BackwardsWithoutRegisterWidth).unwrap(),
        r#""BACKWARDS_WITHOUT_REGISTER_WIDTH""#
    );
    assert_eq!(
        serde_json::to_string(&SeriesOrigin::SmartMeterGateway).unwrap(),
        r#""SMART_METER_GATEWAY""#
    );
    assert_eq!(
        serde_json::to_string(&Zaehlertyp::IntelligentesMesssystem).unwrap(),
        r#""INTELLIGENTES_MESSSYSTEM""#
    );
    assert_eq!(
        serde_json::to_string(&Bilanzierungsmethode::Rlm).unwrap(),
        r#""RLM""#
    );
    assert_eq!(
        serde_json::to_string(&SubstituteMethod::PriorPeriodAverage).unwrap(),
        r#""PRIOR_PERIOD_AVERAGE""#
    );
    assert_eq!(
        serde_json::to_string(&SubstitutionReason::GatewayCommFailure).unwrap(),
        r#""GATEWAY_COMM_FAILURE""#
    );

    // Every variant of each new enum round-trips.
    for h in Holiday::ALL {
        round_trip(h);
    }
    for k in VirtualMeterKind::ALL {
        round_trip(k);
    }
    for k in AnomalyKind::ALL {
        round_trip(k);
    }
    for o in SeriesOrigin::ALL {
        round_trip(o);
    }
    for z in Zaehlertyp::ALL {
        round_trip(z);
    }
    for b in Bilanzierungsmethode::ALL {
        round_trip(b);
    }
    for m in SubstituteMethod::ALL {
        round_trip(m);
    }
    for r in SubstitutionReason::ALL {
        round_trip(r);
    }
}

/// The tags and string forms introduced or changed in 0.18.
///
/// The gas `LoadProfile` codes changed **deliberately**: `"EF"`, `"MF"` and
/// `"GHD"` were not BDEW gas profile codes — the real set is HEF/HMF/HKO plus
/// the eleven Gewerbe types. Archives written under the old tags fail to
/// decode, which is the honest outcome; `LoadProfile::parse` still reads
/// `"EF"`/`"MF"` leniently for callers migrating stored strings by hand.
#[test]
fn tags_added_in_0_18_are_pinned() {
    use metering::{LoadProfile, MaloId, MeloId};

    // MaLo/MeLo travel as their canonical strings.
    let malo: MaloId = "41373559241".parse().unwrap();
    assert_eq!(json(&malo), r#""41373559241""#);
    round_trip(malo);
    // ...and the check digit is enforced on the way back in.
    assert!(serde_json::from_str::<MaloId>(r#""41373559240""#).is_err());

    let melo: MeloId = "DE00056266802AO6G56M11SN51G21M24S".parse().unwrap();
    assert_eq!(json(&melo), r#""DE00056266802AO6G56M11SN51G21M24S""#);
    round_trip(melo);

    // LoadProfile: the serde tag now equals the as_str code for every variant
    // — including the gas profiles and CUSTOM, which the derived form spelt
    // differently ("GasEF", "Custom").
    for p in LoadProfile::ALL {
        assert_eq!(json(&p), format!("\"{}\"", p.as_str()), "{p:?}");
        round_trip(p);
    }
    assert_eq!(json(&LoadProfile::GasHEF), r#""HEF""#);
    assert_eq!(json(&LoadProfile::GasGKO), r#""GKO""#);
    assert_eq!(json(&LoadProfile::Custom), r#""CUSTOM""#);
    assert!(
        serde_json::from_str::<LoadProfile>(r#""GARBAGE""#).is_err(),
        "an unknown profile code is an error, never a silent default"
    );
}

/// `MeterReading`'s field names, like `MeterInterval`'s above.
#[test]
fn meter_reading_field_names_are_stable() {
    use metering::reading::MeterReading;
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
    // The lower-camel spelling a hand-written fixture might guess at:
    assert!(serde_json::from_str::<MeasurementSource>(r#"{"Mscons":{}}"#).is_err());
}

/// Tags introduced in 0.19, pinned on arrival so the commitment starts here
/// rather than at the first rename.
#[test]
fn tags_added_in_0_19_are_pinned() {
    use metering::calendar::DayBoundary;
    use metering::lifecycle::{MeterLifecycleEventType, MeterStatus};

    assert_eq!(json(&DayBoundary::Midnight), r#""MIDNIGHT""#);
    assert_eq!(json(&DayBoundary::Gastag), r#""GASTAG""#);

    for v in MeterStatus::ALL {
        assert_eq!(json(&v), format!("\"{}\"", v.as_str()), "{v:?}");
    }
    for v in MeterLifecycleEventType::ALL {
        assert_eq!(json(&v), format!("\"{}\"", v.as_str()), "{v:?}");
    }
    assert_eq!(json(&MeterStatus::Active), r#""ACTIVE""#);
    assert_eq!(
        json(&MeterLifecycleEventType::Recalibrated),
        r#""RECALIBRATED""#
    );
}

/// A 2025 profile table travels as a **list** of day tables, because a JSON
/// object key must be a string and a `(month, day_type)` tuple is not one. The
/// derived map representation compiled and then failed at run time with "key
/// must be a string" — an API that existed only in the type system.
#[test]
fn a_dynamic_slp_profile_travels_as_a_list_of_day_tables() {
    use metering::load_profile::{DynamicSlpProfile, SlpDayType};

    let mut profile = DynamicSlpProfile {
        profile: Some(metering::LoadProfile::G25),
        ..Default::default()
    };
    profile
        .values
        .insert((1, SlpDayType::Samstag), vec![dec!(0.25)]);

    let encoded = json(&profile);
    assert!(
        encoded.contains(r#""values":[{"month":1,"day_type":"SAMSTAG","values":["0.25"]}]"#),
        "{encoded}"
    );
    let back: DynamicSlpProfile = serde_json::from_str(&encoded).expect("reads back");
    assert_eq!(back.values, profile.values);
}

/// The GHD Summenlastprofil — the fifteenth gas profile type, pinned so its tag
/// cannot drift.
#[test]
fn the_ghd_summenlastprofil_is_on_the_wire() {
    use metering::LoadProfile;

    assert_eq!(json(&LoadProfile::GasGHD), r#""GHD""#);
    assert_eq!(
        serde_json::from_str::<LoadProfile>(r#""GHD""#).unwrap(),
        LoadProfile::GasGHD
    );
    assert_eq!(
        LoadProfile::ALL.iter().filter(|p| p.is_gas()).count(),
        15,
        "the Leitfaden publishes fifteen gas profiles"
    );
}

/// `AggregationRule` is internally tagged on `kind`, and that tag is
/// `VirtualMeterKind`'s own spelling.
///
/// External tagging would put the discriminator in a JSON *key* whose depth
/// varies by variant, so a stored rule would need a separate `rule_type` column:
/// a key cannot be indexed or queried as a value.
#[test]
fn an_aggregation_rule_carries_one_discriminator() {
    use metering::{AggregationRule, VirtualMeterKind};

    let rule = AggregationRule::GgvConstantAllocation {
        plant_melo_id: "MELO_PLANT".to_owned(),
        tenant_melo_id: "MELO_T1".to_owned(),
        fraction: dec!(0.10),
    };
    let encoded = json(&rule);

    // The discriminator is a value at a fixed path, not a variant-dependent key.
    let value: serde_json::Value = serde_json::from_str(&encoded).unwrap();
    assert_eq!(value["kind"], "GGV_CONSTANT_ALLOCATION");
    assert_eq!(value["plant_melo_id"], "MELO_PLANT");
    assert_eq!(
        value["fraction"], "0.10",
        "a Decimal travels as its exact string"
    );

    // ...and it is the same spelling `VirtualMeterKind` writes.
    assert_eq!(value["kind"], serde_json::json!(rule.kind()));
    assert_eq!(
        json(&VirtualMeterKind::GgvConstantAllocation),
        r#""GGV_CONSTANT_ALLOCATION""#
    );
    assert_eq!(rule.kind().as_str(), "GGV_CONSTANT_ALLOCATION");

    // Every variant round-trips, and every payload field sits at depth one.
    for rule in [
        AggregationRule::Sum {
            source_malo_ids: vec!["A".to_owned()],
        },
        AggregationRule::Residual {
            total_malo_id: "T".to_owned(),
            subtract_malo_ids: vec!["P".to_owned()],
        },
        AggregationRule::PvSelfConsumption {
            grid_malo_id: "G".to_owned(),
            generation_malo_id: "P".to_owned(),
        },
        rule.clone(),
        AggregationRule::GgvProportionalAllocation {
            plant_melo_id: "P".to_owned(),
            tenant_melo_id: "T1".to_owned(),
            all_tenant_melo_ids: vec!["T1".to_owned()],
        },
    ] {
        let encoded = json(&rule);
        let value: serde_json::Value = serde_json::from_str(&encoded).unwrap();
        assert_eq!(
            value["kind"],
            serde_json::json!(rule.kind().as_str()),
            "{encoded}"
        );
        assert_eq!(
            serde_json::from_str::<AggregationRule>(&encoded).unwrap(),
            rule
        );
    }

    // The old external tagging must not be silently accepted.
    assert!(
        serde_json::from_str::<AggregationRule>(r#"{"Sum":{"source_malo_ids":["A"]}}"#).is_err(),
        "the pre-0.19 shape is gone, not quietly tolerated"
    );
}

/// Everything the 0.20.0 round put on the wire.
///
/// The § 14a quantities, the Modul 3 conformance vocabulary, the
/// Marktpartner-ID and the community allocation are all things a consumer
/// stores: a curated DSO calendar with its verdict, a readiness report, a
/// settlement run. Their tags are covered by semver from here.
#[test]
fn tags_added_in_0_20_are_pinned() {
    use metering::CodeVergabestelle;
    use metering::para14a::{SteuVeFallgruppe, Verursachungsregel};
    use metering::power_quality::Phase;
    use metering::zaehlzeit::{Modul3Conformance, Modul3Finding, Quarter};

    // Every coded enum added this round writes its `as_str` code…
    for v in SteuVeFallgruppe::ALL {
        assert_eq!(json(&v), format!("\"{}\"", v.as_str()), "{v:?}");
    }
    for v in Verursachungsregel::ALL {
        assert_eq!(json(&v), format!("\"{}\"", v.as_str()), "{v:?}");
    }
    for v in Phase::ALL {
        assert_eq!(json(&v), format!("\"{}\"", v.as_str()), "{v:?}");
    }
    for v in Quarter::ALL {
        assert_eq!(json(&v), format!("\"{}\"", v.as_str()), "{v:?}");
    }
    for v in Modul3Finding::ALL {
        assert_eq!(json(&v), format!("\"{}\"", v.as_str()), "{v:?}");
    }
    for v in Modul3Conformance::ALL {
        assert_eq!(json(&v), format!("\"{}\"", v.as_str()), "{v:?}");
    }
    for v in CodeVergabestelle::ALL {
        assert_eq!(json(&v), format!("\"{}\"", v.as_str()), "{v:?}");
    }

    // …and the two whose Rust name and market spelling disagree are pinned
    // literally, because `SCREAMING_SNAKE_CASE` would split them.
    assert_eq!(
        json(&Verursachungsregel::SteuVeZuletzt),
        r#""STEUVE_ZULETZT""#
    );
    assert_eq!(
        json(&Modul3Finding::Modul1NotSelected),
        r#""MODUL_1_NOT_SELECTED""#
    );
    assert_eq!(json(&Quarter::Q1), r#""Q1""#);
    assert_eq!(json(&Phase::L1), r#""L1""#);
    assert_eq!(json(&CodeVergabestelle::Gs1OrOther), r#""GS1_OR_OTHER""#);
}

/// A Marktpartner-ID travels as its digits, like every other identifier here.
#[test]
fn a_bdew_code_is_a_string_on_the_wire() {
    use metering::BdewCode;

    let code: BdewCode = "9900987654321".parse().unwrap();
    assert_eq!(json(&code), r#""9900987654321""#);
    assert_eq!(json(&code), format!("\"{code}\""), "serde is Display");

    let back: BdewCode = serde_json::from_str(r#""9900987654321""#).expect("reads back");
    assert_eq!(back, code);

    // Thirteen digits or nothing — the structure is enforced on the way in.
    assert!(serde_json::from_str::<BdewCode>(r#""99009876543""#).is_err());
}

/// The § 14a inputs and parameters, field by field.
#[test]
fn para14a_field_names_are_stable() {
    use metering::para14a::{Para14aConfig, SteuVe, SteuVeFallgruppe};

    let device = SteuVe::new(SteuVeFallgruppe::Waermepumpe, dec!(20));
    assert_eq!(
        json(&device),
        r#"{"fallgruppe":"WAERMEPUMPE","netzanschlussleistung_kw":"20"}"#,
    );

    let cfg = Para14aConfig::default();
    let value: serde_json::Value = serde_json::from_str(&json(&cfg)).unwrap();
    assert_eq!(value["mindestleistung_kw"], "4.2");
    assert_eq!(value["skalierung_schwelle_kw"], "11");
    assert_eq!(value["skalierungsfaktor"], "0.4");
}

/// Per-phase apparent power, field by field.
#[test]
fn phase_apparent_power_field_names_are_stable() {
    use metering::power_quality::{Phase, PhaseApparentPower};

    let p = PhaseApparentPower::single_phase(Phase::L2, dec!(4.6));
    assert_eq!(json(&p), r#"{"l1_kva":"0","l2_kva":"4.6","l3_kva":"0"}"#);
}

/// An allocation key carries one discriminator at a fixed path, like
/// `AggregationRule` — a settlement stores it and queries on it.
#[test]
fn an_allocation_key_carries_one_discriminator() {
    use metering::AllocationKey;
    use std::collections::BTreeMap;

    let constant = AllocationKey::Constant {
        fractions: BTreeMap::from([("T1".to_owned(), dec!(0.25))]),
    };
    let value: serde_json::Value = serde_json::from_str(&json(&constant)).unwrap();
    assert_eq!(value["kind"], "CONSTANT");
    assert_eq!(value["fractions"]["T1"], "0.25");

    let proportional = AllocationKey::Proportional {
        participants: vec!["T1".to_owned(), "T2".to_owned()],
    };
    let value: serde_json::Value = serde_json::from_str(&json(&proportional)).unwrap();
    assert_eq!(value["kind"], "PROPORTIONAL");
    assert_eq!(value["participants"][1], "T2");

    for key in [constant, proportional] {
        let back: AllocationKey = serde_json::from_str(&json(&key)).expect("round trips");
        assert_eq!(back, key);
    }
}

/// A community allocation is a settlement record, so every field name it
/// carries is part of the wire format.
#[test]
fn community_allocation_field_names_are_stable() {
    use metering::{AllocationKey, MeterInterval, QualityFlag, compute_community_allocation};
    use std::collections::HashMap;

    let iv = |kwh| {
        vec![MeterInterval {
            from: datetime!(2026-06-01 12:00 UTC),
            to: datetime!(2026-06-01 12:15 UTC),
            value: kwh,
            quality: QualityFlag::Measured,
            obis_code: None,
        }]
    };
    let mut sources = HashMap::new();
    sources.insert("PLANT".to_owned(), iv(dec!(10)));
    sources.insert("T1".to_owned(), iv(dec!(1)));

    let key = AllocationKey::Proportional {
        participants: vec!["T1".to_owned()],
    };
    let out = compute_community_allocation("PLANT", &key, &sources).unwrap();
    let value: serde_json::Value = serde_json::from_str(&json(&out[0])).unwrap();

    assert_eq!(value["from"], "2026-06-01T12:00:00Z");
    assert_eq!(value["generation"], "10");
    assert_eq!(value["total_consumption"], "1");
    assert_eq!(value["pool_cap"], "1");
    assert_eq!(value["surplus_to_grid"], "9");
    assert_eq!(value["quality"], "MEASURED");
    assert_eq!(value["participants"][0]["id"], "T1");
    assert_eq!(value["participants"][0]["consumption"], "1");
    assert_eq!(value["participants"][0]["share"], "10");
    assert_eq!(value["participants"][0]["allocated"], "1");
    assert_eq!(value["participants"][0]["net_grid_draw"], "0");
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
    use metering::power_quality::PowerQualityInterval;
    use metering::reading::MeterReading;

    let iv = MeterInterval {
        from: datetime!(2026-06-01 12:00 UTC),
        to: datetime!(2026-06-01 12:15 UTC),
        value: dec!(2.5),
        quality: QualityFlag::Measured,
        obis_code: Some(ObisCode::STROM_BEZUG_TOTAL),
    };
    assert_eq!(
        json(&iv),
        r#"{"from":"2026-06-01T12:00:00Z","to":"2026-06-01T12:15:00Z","value":"2.5","quality":"MEASURED","obis_code":"1-0:1.8.0"}"#,
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
    let milestone = metering::ROLLOUT_MILESTONES
        .iter()
        .find(|m| m.window_from.is_some())
        .expect("a flow milestone");
    let value: serde_json::Value = serde_json::from_str(&json(milestone)).unwrap();
    assert_eq!(value["deadline"], "2026-12-31");
    assert_eq!(value["window_from"], "2025-02-25");

    // `None` is `null`, not a missing key or an empty string.
    let stock = metering::ROLLOUT_MILESTONES
        .iter()
        .find(|m| m.window_from.is_none())
        .expect("a stock milestone");
    let value: serde_json::Value = serde_json::from_str(&json(stock)).unwrap();
    assert!(value["window_from"].is_null());
}

/// Every instant- and date-bearing type reads back what it wrote, through both
/// a human-readable format and a binary one.
#[test]
fn timestamps_round_trip_through_json_and_postcard() {
    use metering::lifecycle::{MeterLifecycleEvent, MeterLifecycleEventType};
    use metering::measurement_series::{ProvenanceEntry, ProvenanceEventType};
    use metering::reading::MeterReading;

    let iv = MeterInterval {
        from: datetime!(2026-06-01 12:00 UTC),
        to: datetime!(2026-06-01 12:15 UTC),
        value: dec!(2.5),
        quality: QualityFlag::Measured,
        obis_code: None,
    };
    let reading = MeterReading::measured(datetime!(2026-03-29 01:00 UTC), dec!(42.125));
    let entry = ProvenanceEntry {
        occurred_at: datetime!(2026-01-02 09:30 UTC),
        event_type: ProvenanceEventType::Ingested,
        actor: "MSCONS".to_owned(),
        note: None,
    };
    let event = MeterLifecycleEvent {
        event_id: "EV-1".to_owned(),
        meter_serial: "1ESY0000".to_owned(),
        melo_id: "DE00056266802AO6G56M11SN51G21M24S".parse().unwrap(),
        event_type: MeterLifecycleEventType::Replaced,
        occurred_at: datetime!(2026-06-01 08:00 UTC),
        reading: Some(dec!(17_845)),
        obis_code: None,
        reason: None,
        triggered_by_pid: Some(23003),
    };

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
    both_ways!(entry);
    both_ways!(event);

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
/// The crate states that trade-off — *"no bincode/postcard for that type; the
/// hot types are unaffected"* — and it was not true. `MeterInterval` carries a
/// `Decimal`, whose default `Deserialize` calls `deserialize_any`, and
/// `deserialize_any` is the one question a format without a self-describing
/// wire cannot answer. Enabling `rust_decimal/serde-str` moves it to
/// `deserialize_str`, changing nothing in JSON — a `Decimal` already travelled
/// as its exact string — and making the claim true.
#[test]
fn the_hot_types_survive_a_binary_format_and_the_tagged_ones_do_not() {
    use metering::{AggregationRule, AllocationKey};

    // Hot path: intervals, channels, readings.
    let iv = MeterInterval {
        from: datetime!(2026-06-01 12:00 UTC),
        to: datetime!(2026-06-01 12:15 UTC),
        value: dec!(2.5),
        quality: QualityFlag::Measured,
        obis_code: Some(ObisCode::STROM_BEZUG_TOTAL),
    };
    let bytes = postcard::to_allocvec(&iv).expect("serialises");
    assert_eq!(
        postcard::from_bytes::<MeterInterval>(&bytes).expect("reads back"),
        iv,
    );

    let code = ObisCode::GAS_BRENNWERT_MONATSMITTEL;
    let bytes = postcard::to_allocvec(&code).expect("serialises");
    assert_eq!(postcard::from_bytes::<ObisCode>(&bytes).unwrap(), code);

    // A bare Decimal, which is what actually blocked the whole hot path.
    let value = dec!(-12345.6789);
    let bytes = postcard::to_allocvec(&value).expect("serialises");
    assert_eq!(
        postcard::from_bytes::<rust_decimal::Decimal>(&bytes).unwrap(),
        value
    );

    // Configuration: internal tagging needs a self-describing format, which is
    // the documented cost of putting the discriminator at a fixed, queryable
    // path. Pinned so the trade-off cannot silently move in either direction.
    let rule = AggregationRule::Sum {
        source_malo_ids: vec!["A".to_owned()],
    };
    let bytes = postcard::to_allocvec(&rule).expect("serialises");
    assert!(
        postcard::from_bytes::<AggregationRule>(&bytes).is_err(),
        "internally tagged types are JSON-shaped on purpose",
    );

    let key = AllocationKey::Proportional {
        participants: vec!["T1".to_owned()],
    };
    let bytes = postcard::to_allocvec(&key).expect("serialises");
    assert!(postcard::from_bytes::<AllocationKey>(&bytes).is_err());
}

/// No timestamp field escapes the wire format.
///
/// `src/wire.rs` only applies where a field asks for it, and a field that
/// forgets falls silently back to `time`'s ordinal tuple — in JSON, in one
/// struct, next to siblings that are RFC 3339 strings. Nothing fails to compile
/// and nothing fails to round-trip; the format is just inconsistent, which is
/// the worst kind of wire bug to find later.
///
/// This is a test, so it may read files; the no-I/O guarantee is about `src/`.
#[test]
fn no_timestamp_field_escapes_the_wire_format() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut missing: Vec<String> = Vec::new();
    let mut checked = 0usize;

    for entry in std::fs::read_dir(root.join("src")).expect("src/ is readable") {
        let path = entry.expect("readable entry").path();
        if path.extension().is_none_or(|e| e != "rs") {
            continue;
        }
        let file = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        // Normalised, because a Windows checkout hands out `\r\n` and every
        // line-shaped pattern below would miss.
        let text = std::fs::read_to_string(&path)
            .expect("readable source")
            .replace("\r\n", "\n");

        // A line-by-line walk rather than a search over the whole text: the
        // question is "which attributes sit directly above this item", and only
        // adjacency answers it.
        let mut attrs = String::new();
        let mut inside_serde_struct = false;
        for line in text.lines() {
            let trimmed = line.trim();

            if inside_serde_struct {
                if trimmed == "}" {
                    inside_serde_struct = false;
                    attrs.clear();
                } else if trimmed.starts_with("#[") {
                    attrs.push_str(trimmed);
                } else if !trimmed.starts_with("///") {
                    if let Some(decl) = trimmed.strip_prefix("pub ")
                        && (decl.ends_with(": OffsetDateTime,")
                            || decl.ends_with(": Option<OffsetDateTime>,")
                            || decl.ends_with(": Date,")
                            || decl.ends_with(": Option<Date>,"))
                    {
                        checked += 1;
                        if !attrs.contains("crate::wire") {
                            missing.push(format!("{file}: {decl}"));
                        }
                    }
                    attrs.clear();
                }
                continue;
            }

            if trimmed.starts_with("#[") {
                attrs.push_str(trimmed);
            } else if trimmed.starts_with("pub struct ") && trimmed.ends_with('{') {
                inside_serde_struct = attrs.contains("Serialize");
                attrs.clear();
            } else if !trimmed.starts_with("//") {
                attrs.clear();
            }
        }
    }

    assert!(
        checked > 15,
        "the scan found only {checked} timestamp fields — it has stopped working, not the crate",
    );
    assert!(
        missing.is_empty(),
        "timestamp fields with no wire format, so they travel as `time`'s \
         ordinal tuple while their siblings are RFC 3339: {missing:#?}",
    );
}
