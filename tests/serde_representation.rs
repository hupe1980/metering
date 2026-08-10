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
        sender_mp_id: "9900357000004".to_owned(),
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
/// Changed in 0.16.0: the wire form was `"1-0:1.8.0*255"`, which no consumer
/// could produce by typing the code or by copying it out of an MSCONS message.
/// See `tests/string_canonicalisation.rs` for the invariants that fixes.
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
/// Changed in 0.16.0: the derived form used the Rust variant names
/// (`"QuarterHour"`, `{"Custom":300}`), so the type had two spellings per value
/// and the serde one was a rename away from silently invalidating stored data.
/// ISO 8601 is an external standard that no refactor here can rename.
#[test]
fn interval_resolution_shape_is_stable() {
    assert_eq!(json(&IntervalResolution::QuarterHour), "\"PT15M\"");
    assert_eq!(json(&IntervalResolution::Day), "\"P1D\"");
    assert_eq!(json(&IntervalResolution::Month), "\"P1M\"");
    assert_eq!(json(&IntervalResolution::Year), "\"P1Y\"");
    assert_eq!(json(&IntervalResolution::Custom(300)), "\"PT300S\"");

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
        IntervalResolution::Custom(300),
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
