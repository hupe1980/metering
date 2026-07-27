//! Compile-and-assert the README's claimed snippets.
//!
//! The README makes concrete numerical claims (92, 100, 2 972, the UTC offsets).
//! This file is those claims, verbatim, so the README cannot drift from the code.

use metering::{
    AggregationConfig, IntervalResolution, MeasurementSeries, MeasurementSource, MeterInterval,
    QualityFlag, Sparte, aggregate, calendar,
};
use rust_decimal::dec;
use time::macros::{date, datetime};

#[test]
fn quick_start() {
    let intervals = vec![MeterInterval {
        from: datetime!(2026-06-01 0:00 UTC),
        to: datetime!(2026-06-01 0:15 UTC),
        value_kwh: dec!(2.345),
        quality: QualityFlag::Measured,
        obis_code: Some("1-0:1.8.0*255".parse().unwrap()),
    }];

    let period = aggregate(&intervals, &AggregationConfig::rlm_strom());
    assert_eq!(period.arbeitsmenge_kwh, dec!(2.345));
    assert!(period.spitzenleistung_kw.is_some());
}

#[test]
fn calendar_section() {
    assert_eq!(
        calendar::intervals_in_day(date!(2026 - 03 - 29), IntervalResolution::QuarterHour),
        Some(92)
    );
    assert_eq!(
        calendar::intervals_in_day(date!(2026 - 10 - 25), IntervalResolution::QuarterHour),
        Some(100)
    );
    assert_eq!(
        calendar::intervals_in_month(date!(2026 - 03 - 01), IntervalResolution::QuarterHour),
        Some(2_972)
    );
    assert_eq!(
        calendar::day_start_utc(date!(2026 - 01 - 15)),
        datetime!(2026-01-14 23:00 UTC)
    );
    assert_eq!(
        calendar::day_start_utc(date!(2026 - 07 - 15)),
        datetime!(2026-07-14 22:00 UTC)
    );
    assert_eq!(
        calendar::day_length(date!(2026 - 10 - 25)).whole_hours(),
        25
    );
}

#[test]
fn determinism_section() {
    let series = MeasurementSeries::new(
        "51238696780",
        None,
        vec![],
        MeasurementSource::ManualEntry {
            operator_id: "ops-1".into(),
            reason: "correction".into(),
        },
        datetime!(2026-01-02 09:30 UTC),
    );
    assert_eq!(
        series.provenance[0].occurred_at,
        datetime!(2026-01-02 09:30 UTC)
    );
}

#[test]
fn string_forms_section() {
    assert_eq!(QualityFlag::Substituted.as_str(), "SUBSTITUTED");
    assert_eq!(
        "substituted".parse::<QualityFlag>().unwrap(),
        QualityFlag::Substituted
    );
    assert_eq!(IntervalResolution::QuarterHour.to_string(), "PT15M");
    assert_eq!(
        "PT900S".parse::<IntervalResolution>().unwrap(),
        IntervalResolution::QuarterHour
    );
    assert_eq!(
        Sparte::Gas.billing_unit(),
        metering::MeasurementUnit::KiloWattHour
    );
}

#[test]
fn gas_conversion_section() {
    let kwh = metering::gas_m3_to_kwh_hs(dec!(100), dec!(10.55), dec!(0.9764));
    assert!(kwh > dec!(1000));
}

#[test]
fn imbalance_section() {
    let saldo = metering::compute_imbalance(dec!(1050), dec!(1000));
    assert_eq!(saldo.minder_kwh, dec!(50));
    assert!(saldo.is_minder() && !saldo.is_mehr());
}

#[test]
fn quality_section() {
    use metering::{QualityConfig, score_intervals};
    let base = calendar::day_start_utc(date!(2026 - 06 - 01));
    let intervals: Vec<_> = (0..96)
        .map(|i| MeterInterval {
            from: base + time::Duration::minutes(15 * i),
            to: base + time::Duration::minutes(15 * (i + 1)),
            value_kwh: dec!(2.3),
            quality: QualityFlag::Measured,
            obis_code: None,
        })
        .collect();
    let report = score_intervals(&intervals, QualityConfig::for_sparte(Sparte::Strom));
    assert_eq!(report.grade, metering::QualityGrade::A);
    assert!(report.coverage_pct > 0.0);
}

#[test]
fn parse_error_section() {
    use metering::{ObisCode, ParseError};

    fn decode(
        sparte: &str,
        quality: &str,
        obis: &str,
    ) -> Result<(Sparte, QualityFlag, ObisCode), ParseError> {
        Ok((sparte.parse()?, quality.parse()?, obis.parse()?))
    }

    assert!(decode("STROM", "MEASURED", "1-0:1.8.0*255").is_ok());
    let err = decode("STROM", "MEASURED", "nope").unwrap_err();
    assert_eq!(err.type_name(), "ObisCode");
}

#[test]
fn calendar_days_between_section() {
    let from = calendar::day_start_utc(date!(2026 - 03 - 23));
    let to = calendar::day_start_utc(date!(2026 - 04 - 06));
    assert_eq!((to - from).whole_days(), 13);
    assert_eq!(calendar::days_between(from, to), 14);
}
