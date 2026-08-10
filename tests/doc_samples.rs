//! Compile-and-assert the snippets in the README and on the documentation site.
//!
//! Both make concrete numerical claims — 92, 100, 2 972, the UTC offsets, the
//! eneregio G 685 example, the EN 50160 shares. Prose cannot be type-checked and
//! a static-site generator will happily publish a snippet that does not compile,
//! so the claims live here as executable assertions and the documents quote what
//! this file proves.

use metering::{
    AggregationConfig, IntervalResolution, MeasurementSeries, MeasurementSource, MeterInterval,
    ObisCode, QualityFlag, Sparte, aggregate, calendar,
};
use rust_decimal::dec;
use time::macros::{date, datetime};

#[test]
fn quick_start() {
    let intervals = vec![MeterInterval {
        from: datetime!(2026-06-01 0:00 UTC),
        to: datetime!(2026-06-01 0:15 UTC),
        value: dec!(2.345),
        quality: QualityFlag::Measured,
        obis_code: Some("1-0:1.8.0".parse().unwrap()),
    }];

    let period = aggregate(&intervals, &AggregationConfig::rlm());
    assert_eq!(period.arbeitsmenge, dec!(2.345));
    assert!(period.spitzenleistung_kw.is_some());
    assert_eq!(
        period.spitzenleistung_at,
        Some(datetime!(2026-06-01 0:00 UTC))
    );
}

/// Docs — "Holidays and day types".
#[test]
fn holiday_section() {
    use metering::{Bundesland, Holiday, SlpDayType, slp_day_type};

    let fronleichnam = date!(2026 - 06 - 04);
    assert_eq!(
        Bundesland::By.holiday(fronleichnam),
        Some(Holiday::Fronleichnam)
    );
    assert_eq!(Bundesland::Be.holiday(fronleichnam), None);
    assert_eq!(
        slp_day_type(fronleichnam, Bundesland::By),
        SlpDayType::SonnFeiertag
    );
    assert_eq!(
        slp_day_type(fronleichnam, Bundesland::Be),
        SlpDayType::Werktag
    );
}

/// Docs — a Feiertag books into the off-peak register where the Land
/// observes it.
#[test]
fn holiday_tariff_register_section() {
    use metering::Bundesland;
    use metering::zaehlzeit::{HT, NT, Zaehlzeitdefinition};

    let zzd = Zaehlzeitdefinition::ht_nt("NB-1", date!(2026 - 01 - 01), 6 * 60, 22 * 60);
    let midday = datetime!(2026-06-04 8:00 UTC); // 10:00 CEST on Fronleichnam

    assert_eq!(zzd.register_for(midday), Some(HT));
    assert_eq!(
        zzd.clone().in_land(Bundesland::By).register_for(midday),
        Some(NT)
    );
    assert_eq!(
        zzd.clone().in_land(Bundesland::Be).register_for(midday),
        Some(HT)
    );
}

/// Docs — §14a Modul 3 resolves three registers.
#[test]
fn modul_3_section() {
    use metering::zaehlzeit::{HT, NT, ST, Zaehlzeitdefinition};

    let zzd = Zaehlzeitdefinition::modul_3(
        "NB-14A-3",
        date!(2026 - 01 - 01),
        (17 * 60, 20 * 60),
        (0, 6 * 60),
    );
    assert_eq!(zzd.registers(), vec![HT, NT, ST]);
    assert_eq!(zzd.register_for(datetime!(2026-01-05 17:00 UTC)), Some(HT));
    assert_eq!(zzd.register_for(datetime!(2026-01-05 2:00 UTC)), Some(NT));
    assert_eq!(zzd.register_for(datetime!(2026-01-05 9:00 UTC)), Some(ST));
}

/// Docs — "Declare the period, or gaps at the edges are invisible".
#[test]
fn validation_period_section() {
    use metering::{ValidationConfig, validate_intervals};

    let delivered = vec![MeterInterval {
        from: datetime!(2026-06-01 0:00 UTC),
        to: datetime!(2026-06-01 0:15 UTC),
        value: dec!(2.0),
        quality: QualityFlag::Measured,
        obis_code: None,
    }];
    assert!(validate_intervals(&delivered, &ValidationConfig::default()).is_clean());

    let cfg = ValidationConfig::default().over_period(
        datetime!(2026-06-01 0:00 UTC),
        datetime!(2026-06-01 2:00 UTC),
    );
    assert!(validate_intervals(&delivered, &cfg).has_errors());
}

/// Docs — "Ersatzwertbildung": interior interpolation fractions.
#[test]
fn substitute_section() {
    use metering::{FillGapsConfig, SubstituteMethod, fill_gaps};

    let series = vec![
        MeterInterval {
            from: datetime!(2026-01-01 0:00 UTC),
            to: datetime!(2026-01-01 0:15 UTC),
            value: dec!(0),
            quality: QualityFlag::Measured,
            obis_code: None,
        },
        MeterInterval {
            from: datetime!(2026-01-01 1:00 UTC),
            to: datetime!(2026-01-01 1:15 UTC),
            value: dec!(100),
            quality: QualityFlag::Measured,
            obis_code: None,
        },
    ];
    let filled = fill_gaps(
        &series,
        &FillGapsConfig::new(
            IntervalResolution::QuarterHour,
            datetime!(2026-01-01 0:00 UTC),
            datetime!(2026-01-01 1:15 UTC),
        )
        .short_gap_threshold(10),
    );
    let values: Vec<_> = filled.intervals.iter().map(|iv| iv.value).collect();
    assert_eq!(
        values,
        vec![dec!(0), dec!(25), dec!(50), dec!(75), dec!(100)]
    );
    assert!(
        filled
            .substitutions
            .iter()
            .all(|e| e.method == SubstituteMethod::LinearInterpolation)
    );
}

/// Docs — "Zählerstand → Lastgang": a six-digit register wrapping.
#[test]
fn reading_rollover_section() {
    use metering::reading::{LastgangConfig, MeterReading, to_lastgang};

    let zsg = vec![
        MeterReading::measured(datetime!(2026-06-01 0:00 UTC), dec!(999998.5)),
        MeterReading::measured(datetime!(2026-06-01 0:15 UTC), dec!(1.5)),
    ];

    let blind = to_lastgang(&zsg, &LastgangConfig::strom());
    assert!(blind.intervals.is_empty());
    assert_eq!(blind.anomalies.len(), 1);

    let cfg = LastgangConfig::strom().with_register_digits(6);
    let wrapped = to_lastgang(&zsg, &cfg);
    assert_eq!(wrapped.intervals[0].value, dec!(3.0));
    assert_eq!(wrapped.rollovers.len(), 1);
}

/// Docs — the published eneregio G 685 worked example.
#[test]
fn g685_rounding_section() {
    use metering::{G685FinalRounding, G685Rounding, gas_m3_to_kwh_hs_rounded};

    let kwh = gas_m3_to_kwh_hs_rounded(
        dec!(895),
        dec!(11.369),
        dec!(0.9543),
        G685Rounding {
            final_rounding: G685FinalRounding::WholeKwh,
            ..G685Rounding::default()
        },
    );
    assert_eq!(kwh, dec!(9710));
}

/// Docs — "Unit normalisation refuses to guess".
#[test]
fn unit_normalisation_section() {
    use metering::{GasConversionParams, normalize_to_kwh};

    let gas = GasConversionParams {
        hs_kwh_per_m3: dec!(10.55),
        zustandszahl: dec!(0.98),
    };
    assert_eq!(
        normalize_to_kwh(dec!(100), "m3", Some(&gas), None).unwrap(),
        dec!(1033.900)
    );
    assert_eq!(
        normalize_to_kwh(dec!(3.6), "GJ", None, None).unwrap(),
        dec!(1000)
    );
    assert_eq!(
        normalize_to_kwh(dec!(48), "kW", None, Some(900)).unwrap(),
        dec!(12)
    );
    assert!(normalize_to_kwh(dec!(1), "furlong", None, None).is_err());
}

/// Docs — "OBIS codes": direction is group C alone.
#[test]
fn obis_lastgang_section() {
    let lastgang: ObisCode = "1-0:1.29.0".parse().unwrap();
    assert!(lastgang.is_import());
    assert!(lastgang.is_lastgang());
    assert!(!lastgang.is_maximum());
}

/// Docs — "Power quality": EN 50160 is a statistical test, so one excursion
/// passes the 95 % band and fails the absolute one.
#[test]
fn en50160_section() {
    use metering::power_quality::{En50160Limits, PowerQualityInterval, assess_en50160};

    let mut series: Vec<PowerQualityInterval> = (0..1008)
        .map(|i| {
            let from = datetime!(2026-06-01 0:00 UTC) + time::Duration::minutes(i * 10);
            PowerQualityInterval {
                voltage_l1_v: Some(dec!(231)),
                ..PowerQualityInterval::empty(from, from + time::Duration::minutes(10))
            }
        })
        .collect();
    series[500].voltage_l1_v = Some(dec!(260));

    let report = assess_en50160(&series, &En50160Limits::LOW_VOLTAGE);
    assert!(report.is_conclusive());
    assert!(report.voltage_band.compliant);
    assert!(!report.voltage_absolute.compliant);
    assert!(!report.compliant());
}

/// Docs — "Resampling": 96 intervals on the 25-hour day is a gap.
#[test]
fn resample_dst_section() {
    use metering::{ResampleConfig, resample};

    let base = calendar::day_start_utc(date!(2026 - 10 - 25));
    let short: Vec<MeterInterval> = (0..96)
        .map(|i| MeterInterval {
            from: base + time::Duration::minutes(15 * i),
            to: base + time::Duration::minutes(15 * i + 15),
            value: dec!(1),
            quality: QualityFlag::Measured,
            obis_code: None,
        })
        .collect();
    let buckets = resample(&short, &ResampleConfig::to_daily());
    assert_eq!(buckets[0].expected_count, Some(100));
    assert!(buckets[0].has_missing_data());
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
    assert_eq!(ObisCode::STROM_BEZUG_TOTAL.to_string(), "1-0:1.8.0");
}

/// README → "OBIS value groups C, D and E".
#[test]
fn obis_value_group_section() {
    // Zählerstand, Vorschub, Lastgang, Maximum — all Bezug.
    for code in ["1-0:1.8.0", "1-0:1.9.0", "1-0:1.29.0", "1-0:1.6.0"] {
        assert!(code.parse::<ObisCode>().unwrap().is_import());
    }

    assert!(ObisCode::STROM_BEZUG_LASTGANG.is_lastgang());
    assert!(ObisCode::STROM_BEZUG_MAXIMUM.is_maximum());

    // E = 63 is the Fehlerregister, not tariff 63.
    let fehler: ObisCode = "1-0:1.8.63".parse().unwrap();
    assert!(fehler.is_fehlerregister());
    assert_eq!(fehler.tariff_register(), None);

    // Reactive is C = 3..=8, and only for electricity.
    for c in 3..=8u8 {
        assert!(
            format!("1-0:{c}.8.0")
                .parse::<ObisCode>()
                .unwrap()
                .is_reactive()
        );
    }
    assert!(!ObisCode::GAS_VOLUME_M3.is_reactive());
}

/// README → "Parsing is lenient; writing is not".
#[test]
fn obis_canonicalisation_section() {
    for raw in [
        "1-0:1.8.0",
        "1-0:1.8.0*255",
        "  1-0:1.8.0 ",
        "01-00:01.08.00",
    ] {
        assert_eq!(ObisCode::normalize(raw).unwrap(), "1-0:1.8.0");
    }

    let code = ObisCode::STROM_BEZUG_TOTAL;
    assert_eq!(code.to_full_string(), "1-0:1.8.0*255");
    assert_eq!(format!("{code:#}"), "1-0:1.8.0*255");
    assert_eq!(code.to_full_string().parse::<ObisCode>().unwrap(), code);

    // A real billing-period register keeps its suffix and stays a distinct code.
    let historical: ObisCode = "1-0:1.8.0*1".parse().unwrap();
    assert_eq!(historical.to_string(), "1-0:1.8.0*1");
    assert_ne!(historical, code);
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
            value: dec!(2.3),
            quality: QualityFlag::Measured,
            obis_code: None,
        })
        .collect();
    let report = score_intervals(&intervals, &QualityConfig::for_sparte(Sparte::Strom));
    assert_eq!(report.grade, metering::QualityGrade::A);
    assert!(report.issues.is_empty());
    assert!(report.coverage_pct > 0.0);
}

#[test]
fn parse_error_section() {
    use metering::ParseError;

    fn decode(
        sparte: &str,
        quality: &str,
        obis: &str,
    ) -> Result<(Sparte, QualityFlag, ObisCode), ParseError> {
        Ok((sparte.parse()?, quality.parse()?, obis.parse()?))
    }

    assert!(decode("STROM", "MEASURED", "1-0:1.8.0").is_ok());
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
