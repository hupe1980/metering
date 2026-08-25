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
    assert_eq!(wrapped.rollovers[0].from, datetime!(2026-06-01 0:00 UTC));
    assert_eq!(wrapped.rollovers[0].to, datetime!(2026-06-01 0:15 UTC));
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
        "51238696781".parse().unwrap(),
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

/// Docs — "Market identifiers".
#[test]
fn identifiers_section() {
    use metering::{MaloId, MeloId};

    let malo: MaloId = "41373559241".parse().unwrap();
    assert_eq!(malo.check_digit(), 1);
    assert!("41373559214".parse::<MaloId>().is_err());

    let melo: MeloId = "DE00056266802AO6G56M11SN51G21M24S".parse().unwrap();
    assert_eq!(melo.netzbetreiber_nr(), "000562");
}

/// Docs — "The gas SLP — SigLinDe, published in full".
#[test]
fn gas_slp_section() {
    use metering::gas_daily_quantity;
    use metering::gas_slp::{SigLinDe, allocation_temperature};

    let theta = allocation_temperature(dec!(5.0), dec!(2.5), dec!(2.5), dec!(5.0));
    assert_eq!(theta, dec!(4));

    let h = SigLinDe::DE_HEF34.h_value(theta);
    let q = gas_daily_quantity(dec!(60.3423), h, dec!(1));
    assert!(q > dec!(90));
}

/// Docs — "The Gastag is a different day" / "The Gastag runs 06:00 to 06:00".
#[test]
fn gastag_section() {
    assert_eq!(
        calendar::gas_day_start_utc(date!(2026 - 01 - 15)),
        datetime!(2026-01-15 5:00 UTC)
    );
    assert_eq!(
        calendar::local_gas_day(datetime!(2026-07-15 3:30 UTC)),
        date!(2026 - 07 - 14)
    );
    // The long Gastag is Saturday's, not the transition Sunday's.
    let saturday = calendar::gas_day_end_utc(date!(2026 - 10 - 24))
        - calendar::gas_day_start_utc(date!(2026 - 10 - 24));
    assert_eq!(saturday.whole_hours(), 25);
}

/// Docs — "The boundary travels with the calculation".
#[test]
fn day_boundary_section() {
    use metering::calendar::DayBoundary;
    use metering::{FillGapsConfig, ResampleConfig};

    let cfg = ResampleConfig::to_gas_daily();
    assert_eq!(cfg.day_boundary, DayBoundary::Gastag);

    let fill = FillGapsConfig::new(
        IntervalResolution::Day,
        calendar::gas_day_start_utc(date!(2026 - 10 - 23)),
        calendar::gas_day_start_utc(date!(2026 - 10 - 27)),
    )
    .on(DayBoundary::Gastag);
    assert_eq!(fill.day_boundary, DayBoundary::Gastag);

    // A gas month is a whole number of Gastage.
    assert_eq!(
        DayBoundary::Gastag.month_start_utc(date!(2026 - 02 - 14)),
        calendar::gas_day_start_utc(date!(2026 - 02 - 01))
    );
}

/// Docs — "A local time that does not exist, and one that happens twice".
#[test]
fn skipped_local_time_section() {
    let back = calendar::shift_back_days(datetime!(2026-03-30 0:30 UTC), 1);
    assert_eq!(back, datetime!(2026-03-29 1:30 UTC));
    assert_eq!(calendar::to_berlin(back).hour(), 3);
}

/// Docs — "one value per second count".
#[test]
fn custom_resolution_section() {
    use metering::CustomSeconds;

    assert_eq!(CustomSeconds::new(900), None, "900 s is the QuarterHour");
    assert_eq!(
        IntervalResolution::from_seconds(900),
        Some(IntervalResolution::QuarterHour)
    );
    // A custom length always writes seconds — one canonical spelling — while
    // the parser still accepts the hour form.
    let two_hours = IntervalResolution::from_seconds(7200).unwrap();
    assert_eq!(two_hours.to_string(), "PT7200S");
    assert_eq!("PT2H".parse::<IntervalResolution>(), Ok(two_hours));
    assert_eq!(
        two_hours.to_string().parse::<IntervalResolution>(),
        Ok(two_hours)
    );
}

/// Docs — "V01 reports any uncovered span".
#[test]
fn off_grid_gap_section() {
    use metering::{ValidationConfig, ValidationRuleId, validate_intervals};

    let series = vec![
        MeterInterval {
            from: datetime!(2026-06-01 0:00 UTC),
            to: datetime!(2026-06-01 0:15 UTC),
            value: dec!(2.0),
            quality: QualityFlag::Measured,
            obis_code: None,
        },
        MeterInterval {
            from: datetime!(2026-06-01 0:20 UTC),
            to: datetime!(2026-06-01 0:35 UTC),
            value: dec!(2.0),
            quality: QualityFlag::Measured,
            obis_code: None,
        },
    ];
    let report = validate_intervals(&series, &ValidationConfig::default());
    assert_eq!(report.by_rule(ValidationRuleId::GapDetected).count(), 1);
    assert!(report.has_errors());
}

/// Docs — "on every fall-back day the series spans".
#[test]
fn multi_day_v07_section() {
    use metering::{ValidationConfig, ValidationRuleId, validate_intervals};

    let mut span: Vec<MeterInterval> = (0..96 + 100)
        .map(|i| {
            let from = datetime!(2026-10-23 22:00 UTC) + time::Duration::minutes(15 * i);
            MeterInterval {
                from,
                to: from + time::Duration::minutes(15),
                value: dec!(1),
                quality: QualityFlag::Measured,
                obis_code: None,
            }
        })
        .collect();
    span.retain(|iv| {
        !(iv.from >= datetime!(2026-10-25 1:00 UTC) && iv.from < datetime!(2026-10-25 2:00 UTC))
    });

    let report = validate_intervals(&span, &ValidationConfig::default());
    assert_eq!(report.by_rule(ValidationRuleId::DstAmbiguity).count(), 1);
}

/// Docs — "The Gastag runs 06:00 to 06:00": the boundary on a whole grid.
#[test]
fn gas_day_resample_section() {
    use metering::{ResampleConfig, resample};
    use time::Duration;

    let start = calendar::gas_day_start_utc(date!(2026 - 01 - 15));
    let series: Vec<MeterInterval> = (0..48)
        .map(|i| MeterInterval {
            from: start + Duration::hours(i),
            to: start + Duration::hours(i + 1),
            value: dec!(1),
            quality: QualityFlag::Measured,
            obis_code: None,
        })
        .collect();

    let gas_days = resample(&series, &ResampleConfig::to_gas_daily());
    assert_eq!(gas_days.len(), 2);
    assert_eq!(gas_days[0].total, dec!(24));
    assert_eq!(gas_days[0].is_complete(), Some(true));

    let calendar_days = resample(
        &series,
        &ResampleConfig::new(IntervalResolution::Hour, IntervalResolution::Day),
    );
    assert_eq!(calendar_days.len(), 3);
    assert!(calendar_days[0].has_missing_data());
}

/// README — "...and gas does not start its day at midnight".
#[test]
fn readme_gas_day_section() {
    use metering::{ResampleConfig, resample};
    use time::Duration;

    let start = calendar::gas_day_start_utc(date!(2026 - 01 - 15));
    let series: Vec<MeterInterval> = (0..48)
        .map(|i| MeterInterval {
            from: start + Duration::hours(i),
            to: start + Duration::hours(i + 1),
            value: dec!(1),
            quality: QualityFlag::Measured,
            obis_code: None,
        })
        .collect();

    let gas_days = resample(&series, &ResampleConfig::to_gas_daily());
    assert_eq!(gas_days.len(), 2);
    assert_eq!(gas_days[0].is_complete(), Some(true));

    let calendar_days = resample(
        &series,
        &ResampleConfig::new(IntervalResolution::Hour, IntervalResolution::Day),
    );
    assert_eq!(calendar_days.len(), 3);
}

/// Docs — "Fifteen profiles, and the one this crate wrongly deleted".
#[test]
fn gas_ghd_section() {
    use metering::LoadProfile;

    let ghd = LoadProfile::parse("GHD").expect("a real profile");
    assert!(ghd.is_gas() && ghd.is_commercial());
    assert!(ghd.is_gas_aggregate(), "the only aggregate of the fifteen");
    assert_eq!(LoadProfile::ALL.iter().filter(|p| p.is_gas()).count(), 15);
}

/// Docs — "The allocated amount, not just the net".
#[test]
fn ggv_allocation_section() {
    use metering::{AggregationRule, compute_ggv_allocation};
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

    let rule = AggregationRule::GgvConstantAllocation {
        plant_melo_id: "PLANT".to_owned(),
        tenant_melo_id: "T1".to_owned(),
        fraction: dec!(0.5),
    };
    let out = compute_ggv_allocation(&rule, &sources).unwrap();

    assert_eq!(out[0].share, dec!(5.0));
    assert_eq!(out[0].allocated, dec!(1));
    assert_eq!(out[0].net_grid_draw, dec!(0));
    assert!(out[0].capped());
    assert_eq!(out[0].surplus_to_grid(), dec!(4.0));
}

/// Docs — "A clean report is not the same as a clean series".
#[test]
fn disabled_rules_section() {
    use metering::{QualityConfig, ValidationRuleId};

    let cfg = QualityConfig::for_sparte(Sparte::Strom);
    assert_eq!(cfg.validation.disabled_rules().to_string(), "V08, V12");
    assert_eq!(
        ValidationRuleId::ImplausiblePower.enabling_field(),
        Some("max_plant_power_kw")
    );
}

/// Docs — "The derived series is a different channel".
#[test]
fn messart_conversion_section() {
    let bezug: ObisCode = "1-0:1.8.0".parse().unwrap();
    let einspeisung: ObisCode = "1-0:2.8.0".parse().unwrap();

    assert_eq!(bezug.as_lastgang(), Some(ObisCode::STROM_BEZUG_LASTGANG));
    assert_eq!(
        einspeisung.as_lastgang(),
        Some(ObisCode::STROM_EINSPEISUNG_LASTGANG)
    );
    assert_eq!(bezug.as_lastgang().unwrap().as_zaehlerstand(), Some(bezug));
    assert_eq!("1-0:1.8.1".parse::<ObisCode>().unwrap().as_lastgang(), None);
    assert_eq!(ObisCode::GAS_VOLUME_M3.as_lastgang(), None);
}

/// Docs — "Cadence comes from the readings".
#[test]
fn reading_cadence_section() {
    use metering::reading::{LastgangConfig, MeterReading, detect_reading_cadence};
    use rust_decimal::Decimal;
    use time::Duration;

    let zsg: Vec<MeterReading> = (0..8)
        .map(|i| {
            MeterReading::measured(
                datetime!(2026-06-01 0:00 UTC) + Duration::minutes(15 * i),
                dec!(1000) + Decimal::from(i),
            )
        })
        .collect();

    let cadence = detect_reading_cadence(&zsg).unwrap();
    assert_eq!(cadence, IntervalResolution::QuarterHour);

    let cfg = LastgangConfig::strom().with_capacity_kw(dec!(30), cadence);
    assert_eq!(cfg.max_delta, Some(dec!(7.5)));
}
