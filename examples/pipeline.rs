//! An end-to-end Messstellenbetreiber pipeline for one Liefertag.
//!
//! ```text
//! Zählerstandsgang ─► Lastgang ─► validate ─► Ersatzwerte ─► Abrechnung
//!                     (reading)   (validation)  (substitute)   (aggregation
//!                                                               + zaehlzeit)
//! ```
//!
//! Run it with:
//!
//! ```console
//! cargo run --example pipeline
//! ```
//!
//! The day chosen is **25 October 2026**, the autumn DST transition. It is 25
//! hours long and holds **100** quarter-hours, not 96. Every stage below gets
//! that right without being told, because each one resolves the day through
//! [`metering::calendar`] rather than assuming a length — which is the single
//! thing this crate exists to do.
//!
//! Two defects are planted in the readings so the pipeline has something to
//! find: a six-digit register that wraps past 999 999, and a corrupt reading
//! that makes two spans un-differenceable.

use metering::reading::{LastgangConfig, MeterReading, to_lastgang};
use metering::zaehlzeit::{HT, NT, ST, Zaehlzeitdefinition};
use metering::{
    AggregationConfig, Bundesland, FillGapsConfig, IntervalResolution, QualityConfig,
    SubstitutionReason, ValidationSeverity, aggregate, calendar, score_intervals,
    validate_intervals,
};
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive as _;
use time::Duration;
use time::macros::date;

fn main() {
    let day = date!(2026 - 10 - 25);
    let period_from = calendar::day_start_utc(day);
    let period_to = calendar::day_end_utc(day);

    // The day's real length, resolved from the tz database — never assumed.
    let expected = calendar::intervals_in_day(day, IntervalResolution::QuarterHour)
        .expect("a quarter-hour divides a Berlin day");

    println!(
        "Liefertag {day} — {} h, {expected} quarter-hours",
        calendar::day_length(day).whole_hours()
    );
    println!("  {period_from} … {period_to} (UTC)\n");

    // ── 1. The Zählerstandsgang the gateway delivered ────────────────────────
    let readings = build_zaehlerstandsgang(period_from, expected);
    println!("1. Zählerstandsgang: {} readings", readings.len());

    // ── 2. Difference it into a Lastgang ─────────────────────────────────────
    //
    // The register width lets a wrap past 999 999 be reconstructed; the
    // capacity cap keeps that from turning an undocumented meter exchange into
    // 200 000 kWh. Without both, the conversion refuses rather than guessing.
    let lastgang = to_lastgang(
        &readings,
        &LastgangConfig::strom()
            .with_register_digits(6)
            .with_capacity_kw(Decimal::from(30), IntervalResolution::QuarterHour),
    );
    println!(
        "2. Lastgang:  {} intervals, {} rollover(s) reconstructed, {} anomaly/-ies refused",
        lastgang.intervals.len(),
        lastgang.rollovers.len(),
        lastgang.anomalies.len(),
    );
    for a in &lastgang.anomalies {
        println!("     ! {a}");
    }

    // ── 3. Validate against the declared period ──────────────────────────────
    //
    // The period matters: without it, gap detection sees only the holes
    // *between* the intervals that arrived, and a delivery missing its tail
    // validates clean.
    let quality_cfg = QualityConfig::default().over_period(period_from, period_to);
    let report = validate_intervals(&lastgang.intervals, &quality_cfg.validation);
    println!(
        "3. Validation: {} finding(s), {} blocking",
        report.issues.len(),
        report.billing_block_count(),
    );
    for issue in report.by_severity(ValidationSeverity::Error) {
        println!("     ✗ {} {}", issue.rule_id, issue.message);
    }

    // ── 4. Ersatzwertbildung ─────────────────────────────────────────────────
    //
    // The grid is `IntervalResolution::QuarterHour` over the declared period,
    // so leading and trailing gaps are filled too. Every synthesised value
    // records the method that actually produced it.
    let filled = fill(&lastgang.intervals, period_from, period_to);
    println!(
        "4. Ersatzwerte: {} of {} intervals substituted ({:.1} % measured)",
        filled.substituted_count(),
        filled.intervals.len(),
        filled.measured_pct(),
    );
    for entry in &filled.substitutions {
        println!(
            "     + {} {:?} ({} reference value(s))",
            entry.interval.from, entry.method, entry.reference_count,
        );
    }

    // ── 5. The billing period ────────────────────────────────────────────────
    let period = aggregate(
        &filled.intervals,
        &AggregationConfig::rlm().over_period(period_from, period_to),
    );
    println!("\n5. Abrechnung");
    println!("     Arbeitsmenge     {} kWh", period.arbeitsmenge);
    println!(
        "     Spitzenleistung  {} kW at {}",
        period.spitzenleistung_kw.unwrap_or_default(),
        period
            .spitzenleistung_at
            .map_or_else(|| "—".to_owned(), |t| calendar::to_berlin(t).to_string()),
    );
    println!("     Coverage         {:.2} %", period.coverage_pct);

    // ── 6. Split across the §14a Modul 3 registers ───────────────────────────
    //
    // Mandatory for every Netzbetreiber since 1 April 2025: three levels, not
    // two. The Niedertarif band crosses midnight, so it is two windows.
    let zzd = Zaehlzeitdefinition::modul_3(
        "NB-14A-3",
        date!(2026 - 01 - 01),
        (17 * 60, 20 * 60), // Hochtarif   17:00–20:00 local
        (22 * 60, 6 * 60),  // Niedertarif 22:00–06:00 local, wrapping
    )
    .in_land(Bundesland::Nw);

    let registers = zzd.split_energy(&filled.intervals);
    println!("\n6. Zählzeitregister");
    for name in [HT, NT, ST] {
        let kwh = registers
            .get(&Some(name.to_owned()))
            .copied()
            .unwrap_or_default();
        println!("     {name}  {kwh:>10} kWh");
    }

    // ── 7. One grade for the whole thing ─────────────────────────────────────
    let graded = score_intervals(&filled.intervals, &quality_cfg);
    println!(
        "\n7. Grade {} — {} findings, {:.2} % coverage{}",
        graded.grade,
        graded.issues.len(),
        graded.coverage_pct,
        if graded.blocks_billing() {
            " (blocks automated billing)"
        } else {
            ""
        },
    );

    check_invariants(&filled.intervals, &period, &registers, expected);
}

/// A day of quarter-hourly Zählerstände with two planted defects.
fn build_zaehlerstandsgang(start: time::OffsetDateTime, count: u32) -> Vec<MeterReading> {
    // A six-digit register, deliberately close to its 999 999 ceiling so it
    // wraps partway through the day.
    let mut register = Decimal::from(999_988);

    (0..=count)
        .map(|i| {
            let at = start + Duration::minutes(15 * i64::from(i));
            // A plausible household-ish profile: quiet at night, busier by day.
            let local_hour = calendar::to_berlin(at).hour();
            let step = match local_hour {
                0..=5 => Decimal::new(4, 2),            // 0.04 kWh — 0.16 kW
                6..=8 | 17..=21 => Decimal::new(45, 2), // 0.45 kWh — 1.8 kW
                _ => Decimal::new(18, 2),               // 0.18 kWh
            };

            let value = if i == 60 {
                // Defect two: a corrupt reading. Both spans touching it become
                // un-differenceable and are refused rather than invented.
                Decimal::from(500)
            } else {
                let v = register;
                register += step;
                // Defect one: the six-digit register wraps.
                if register >= Decimal::from(1_000_000) {
                    register -= Decimal::from(1_000_000);
                }
                v
            };

            MeterReading::measured(at, value)
        })
        .collect()
}

fn fill(
    intervals: &[metering::MeterInterval],
    from: time::OffsetDateTime,
    to: time::OffsetDateTime,
) -> metering::FilledSeries {
    metering::fill_gaps(
        intervals,
        &FillGapsConfig::new(IntervalResolution::QuarterHour, from, to)
            .because(SubstitutionReason::PlausibilityCheckFailed),
    )
}

/// The properties a billing run depends on. An example that only prints is a
/// demo; one that asserts is documentation you cannot silently break.
fn check_invariants(
    intervals: &[metering::MeterInterval],
    period: &metering::BillingPeriod,
    registers: &std::collections::BTreeMap<Option<String>, Decimal>,
    expected: u32,
) {
    assert_eq!(
        intervals.len(),
        expected as usize,
        "the filled series covers the 25-hour day exactly"
    );
    assert!(
        (period.coverage_pct - 100.0).abs() < 1e-9,
        "every slot is accounted for after Ersatzwertbildung"
    );
    assert_eq!(
        registers.values().sum::<Decimal>(),
        period.arbeitsmenge,
        "the register split reconstructs the Arbeitsmenge exactly"
    );
    assert!(
        !registers.contains_key(&None),
        "the Modul 3 fallback covers every instant of the day"
    );

    // The autumn day repeats 02:00–03:00 local, which sits inside the 22:00–06:00
    // Niedertarif band — so it holds four more NT quarter-hours than an ordinary
    // day would. A fixed 96-interval assumption gets this wrong twice over.
    let nt_intervals = intervals
        .iter()
        .filter(|iv| {
            let h = calendar::to_berlin(iv.from).hour();
            !(6..22).contains(&h)
        })
        .count();
    assert_eq!(nt_intervals, 36, "8 h of NT plus the repeated hour");

    let _ = period.arbeitsmenge.to_f64();
    println!("\n✓ invariants hold");
}
