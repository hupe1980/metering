//! Billing: aggregation into Arbeitsmenge and Spitzenleistung, the
//! Mehr-/Mindermenge saldo, the Zählzeit registers, and which values a bill
//! may rest on.
//!
//! The numbers below are chosen for this crate, not printed in a source —
//! those are in `published_examples`. Each test pins one rule with values
//! small enough to check by hand.

use metering::billing::aggregation::{BillingPeriod, aggregate};
use metering::billing::imbalance::ImbalanceSaldo;
use metering::billing::zaehlzeit::{HT, NT, ST, Zaehlzeitdefinition};
use metering::gas::conversion::{GasConversionParams, gas_m3_to_kwh_hs};
use metering::vee::classification::Messtyp;
use metering::{DayBoundary, MeterInterval, QualityFlag, Resolution, Series};
use rust_decimal::{Decimal, dec};
use time::macros::{date, datetime};

/// Aggregate `intervals` as one Strom series over the year they fall in.
fn bill(intervals: &[MeterInterval]) -> BillingPeriod {
    let resolution = Resolution::from_observed_seconds(intervals[0].duration_secs()).unwrap();
    let year = DayBoundary::Strom
        .year(
            DayBoundary::Strom
                .day_of(intervals[0].from())
                .unwrap()
                .year(),
        )
        .unwrap();
    let series = Series::new(resolution, DayBoundary::Strom, intervals.to_vec()).unwrap();
    aggregate(&series, year).unwrap()
}

/// Consecutive quarter-hours from `from`, one per value.
fn quarter_hours(
    from: time::OffsetDateTime,
    values: &[Decimal],
    quality: &[QualityFlag],
) -> Vec<MeterInterval> {
    values
        .iter()
        .zip(quality)
        .enumerate()
        .map(|(i, (v, q))| {
            let at = from + time::Duration::minutes(15 * i as i64);
            MeterInterval::new(at, at + time::Duration::minutes(15), *v, *q).unwrap()
        })
        .collect()
}

// ── Spitzenleistung and Arbeitsmenge ────────────────────────────────────────

/// The Spitzenleistung is the highest quarter-hour demand: 5 kWh in a quarter
/// hour is 20 kW.
#[test]
fn the_spitzenleistung_is_the_highest_quarter_hour() {
    let m = QualityFlag::Measured;
    let intervals = quarter_hours(
        datetime!(2026-07-01 10:00 UTC),
        &[dec!(2.5), dec!(5.0), dec!(1.25)],
        &[m, m, m],
    );
    let period = bill(&intervals);
    assert_eq!(period.spitzenleistung_kw, Some(dec!(20)));
    assert_eq!(period.arbeitsmenge, dec!(8.75));
}

/// A single daily value is a quantity without a demand: an SLP bill has no
/// Spitzenleistung.
#[test]
fn a_daily_value_has_no_spitzenleistung() {
    let day = DayBoundary::Strom.day(date!(2026 - 07 - 01)).unwrap();
    let intervals = vec![
        MeterInterval::new(day.start(), day.end(), dec!(24.0), QualityFlag::Measured).unwrap(),
    ];
    let period = bill(&intervals);
    assert_eq!(period.spitzenleistung_kw, None);
    assert_eq!(period.arbeitsmenge, dec!(24.0));
}

/// A value of unknown quality enters neither the Arbeitsmenge nor the
/// Spitzenleistung — even when it would be the peak.
#[test]
fn a_non_billable_value_enters_neither_total() {
    let intervals = quarter_hours(
        datetime!(2026-07-01 10:00 UTC),
        &[dec!(5.0), dec!(100.0)],
        &[QualityFlag::Measured, QualityFlag::Unknown],
    );
    let period = bill(&intervals);
    assert_eq!(period.spitzenleistung_kw, Some(dec!(20)), "not 400 kW");
    assert_eq!(period.arbeitsmenge, dec!(5.0));
    assert_eq!(period.excluded_count, 1);
}

/// A full RLM month of quarter-hours: 2 880 intervals, every one billable,
/// complete coverage.
#[test]
fn a_rlm_month_aggregates_completely() {
    let june = DayBoundary::Strom.month(2026, time::Month::June).unwrap();
    let intervals: Vec<MeterInterval> = (0..2880_i64)
        .map(|i| {
            MeterInterval::quarter_hour(
                june.start() + time::Duration::minutes(i * 15),
                dec!(2.5),
                QualityFlag::Measured,
            )
            .unwrap()
        })
        .collect();

    let series = Series::new(Resolution::QUARTER_HOUR, DayBoundary::Strom, intervals).unwrap();
    let period = aggregate(&series, june).unwrap();

    assert_eq!(period.arbeitsmenge, dec!(7200.0));
    assert_eq!(period.spitzenleistung_kw, Some(dec!(10)));
    assert_eq!(period.billable_count, 2880);
    assert_eq!(period.excluded_count, 0);
    assert!(period.coverage.is_complete());
}

/// Gas read hourly, converted per hour and aggregated: the Arbeitsmenge is the
/// exact sum of the converted values, and a gas bill carries no
/// Spitzenleistung.
#[test]
fn a_gas_day_converts_and_aggregates_exactly() {
    let params = GasConversionParams::new(dec!(10.55), dec!(0.9764));
    let base = datetime!(2026-06-01 0:00 UTC);
    let intervals: Vec<MeterInterval> = (0..24)
        .map(|i| {
            let kwh =
                gas_m3_to_kwh_hs(dec!(10), params.hs_kwh_per_m3(), params.zustandszahl()).unwrap();
            MeterInterval::new(
                base + time::Duration::hours(i),
                base + time::Duration::hours(i + 1),
                kwh,
                QualityFlag::Measured,
            )
            .unwrap()
            .with_obis("7-10:3.1.0".parse().expect("valid Gas OBIS"))
        })
        .collect();

    let period = bill(&intervals);
    // 24 × 10 m³ × 10.55 kWh/m³ × 0.9764 = 24 × 103.0102
    assert_eq!(period.arbeitsmenge, dec!(2472.2448));
    assert_eq!(period.spitzenleistung_kw, None);
}

/// Demand is energy over duration: 2.5 kWh in 15 minutes is 10 kW.
#[test]
fn demand_is_energy_over_the_interval() {
    let iv = MeterInterval::new(
        datetime!(2026-01-01 0:00 UTC),
        datetime!(2026-01-01 0:15 UTC),
        dec!(2.5),
        QualityFlag::Measured,
    )
    .unwrap();
    assert_eq!(iv.demand_kw(), Some(dec!(10)));
}

/// A zero-length interval cannot be built, so `demand_kw` never divides by
/// zero.
#[test]
fn a_zero_duration_interval_is_refused() {
    let ts = datetime!(2026-01-01 0:00 UTC);
    assert!(MeterInterval::new(ts, ts, dec!(5.0), QualityFlag::Measured).is_err());
}

/// Every quality a bill may rest on — measured, substituted, calculated,
/// corrected, preliminary and estimated values alike — and the two it may
/// not: faulty and unknown.
#[test]
fn only_faulty_and_unknown_values_are_not_billable() {
    for flag in QualityFlag::ALL {
        let expected = !matches!(flag, QualityFlag::Faulty | QualityFlag::Unknown);
        assert_eq!(flag.is_billable(), expected, "{flag}");
    }
    // Pinned by name as well, so a new flag cannot slip in as billable by
    // default without this list being read.
    assert!(QualityFlag::Estimated.is_billable());
    assert!(QualityFlag::Substituted.is_billable());
}

/// Of the three metering types, only an iMSys can carry a dynamic tariff
/// (§ 41a EnWG).
#[test]
fn only_an_imsys_supports_a_dynamic_tariff() {
    assert!(Messtyp::IMsys.supports_dynamic_tariff());
    assert!(!Messtyp::Rlm.supports_dynamic_tariff());
    assert!(!Messtyp::Slp.supports_dynamic_tariff());
}

// ── Mehr- und Mindermengen ──────────────────────────────────────────────────

/// Consumption above the profile is a **Minder**menge: the network operator
/// supplied the shortfall and invoices it.
#[test]
fn over_consumption_is_a_mindermenge() {
    let saldo = ImbalanceSaldo::new(dec!(1050), dec!(1000)).unwrap();
    assert_eq!(saldo.minder_kwh(), dec!(50));
    assert_eq!(saldo.mehr_kwh(), Decimal::ZERO);
    assert!(saldo.is_minder());
    assert!(!saldo.is_mehr());
    assert_eq!(saldo.delta_pct(), Some(dec!(5)), "50 on 1 000 bilanziert");
}

/// Consumption below the profile is a **Mehr**menge: the network operator
/// absorbed the surplus and reimburses it.
#[test]
fn under_consumption_is_a_mehrmenge() {
    let saldo = ImbalanceSaldo::new(dec!(950), dec!(1000)).unwrap();
    assert_eq!(saldo.mehr_kwh(), dec!(50));
    assert_eq!(saldo.minder_kwh(), Decimal::ZERO);
    assert!(saldo.is_mehr());
    assert!(!saldo.is_minder());
}

/// A balanced period is neither, with a delta of exactly 0 %.
#[test]
fn a_balanced_period_has_no_imbalance() {
    let saldo = ImbalanceSaldo::new(dec!(1000), dec!(1000)).unwrap();
    assert!(saldo.is_balanced());
    assert_eq!(saldo.delta_pct(), Some(Decimal::ZERO));
}

// ── Zählzeit registers ──────────────────────────────────────────────────────

/// HT/NT 06:00–22:00 on a weekday: a quarter-hour at 10:00 Berlin is HT, one
/// at 00:00 is NT, and the registers reconstruct the Arbeitsmenge.
#[test]
fn ht_nt_splits_a_weekday() {
    let m = QualityFlag::Measured;
    let mut intervals = quarter_hours(datetime!(2026-01-05 9:00 UTC), &[dec!(4.0)], &[m]);
    intervals.extend(quarter_hours(
        datetime!(2026-01-05 23:00 UTC),
        &[dec!(1.0)],
        &[m],
    ));
    let zzd = Zaehlzeitdefinition::ht_nt("NB-1", date!(2026 - 01 - 01), 6 * 60, 22 * 60).unwrap();
    let split = zzd.split_energy(&intervals).unwrap();
    assert_eq!(split.per_register[HT], dec!(4.0));
    assert_eq!(split.per_register[NT], dec!(1.0));
    assert_eq!(
        split.per_register.values().sum::<Decimal>(),
        bill(&intervals).arbeitsmenge
    );
}

/// A Saturday is NT all day.
#[test]
fn ht_nt_puts_the_weekend_in_nt() {
    let intervals = quarter_hours(
        datetime!(2026-01-03 10:00 UTC),
        &[dec!(3.0)],
        &[QualityFlag::Measured],
    );
    let zzd = Zaehlzeitdefinition::ht_nt("NB-1", date!(2026 - 01 - 01), 6 * 60, 22 * 60).unwrap();
    let split = zzd.split_energy(&intervals).unwrap();
    assert!(!split.per_register.contains_key(HT));
    assert_eq!(split.per_register[NT], dec!(3.0));
}

/// § 14a EnWG Modul 3: three registers, and a Niedertarif window that crosses
/// midnight. HT 17:00–20:00 is 3 h, NT 22:00–06:00 is 8 h, ST the remaining
/// 13 h.
#[test]
fn modul_3_splits_a_day_into_three_registers() {
    let zzd = Zaehlzeitdefinition::modul_3(
        "NB-14A-3",
        date!(2026 - 01 - 01),
        (17 * 60, 20 * 60),
        (22 * 60, 6 * 60),
        &metering::billing::zaehlzeit::Quarter::ALL,
    )
    .unwrap();
    assert_eq!(zzd.registers(), vec![HT, NT, ST]);

    let start = DayBoundary::Strom
        .day(date!(2026 - 01 - 05))
        .unwrap()
        .start();
    let intervals = quarter_hours(start, &[dec!(1); 96], &[QualityFlag::Measured; 96]);

    let split = zzd.split_energy(&intervals).unwrap();
    assert_eq!(split.per_register[HT], dec!(12));
    assert_eq!(split.per_register[NT], dec!(32));
    assert_eq!(split.per_register[ST], dec!(52));
    assert_eq!(
        split.per_register.values().sum::<Decimal>(),
        bill(&intervals).arbeitsmenge
    );
}
