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
//! The day is **25 October 2026**, the autumn DST transition: 25 hours, **100**
//! quarter-hours. Every stage resolves the day through
//! [`metering::time::calendar`] rather than assuming a length.
//!
//! Two defects are planted in the readings: a six-digit register that wraps
//! past 999 999, and a corrupt reading that makes two spans un-differenceable.

use metering::billing::zaehlzeit::{HT, NT, Quarter, RegisterSplit, ST, Zaehlzeitdefinition};
use metering::series::reading::{LastgangConfig, MeterReading, to_lastgang};
use metering::time::calendar;
use metering::vee::substitute::{Policy, SubstitutionReason, substitute};
use metering::vee::validation::{Grade, Rules, Severity, validate};
use metering::{
    DayBoundary, Resolution, Series, billing::aggregation::aggregate, time::holiday::Bundesland,
};
use rust_decimal::Decimal;
use time::Duration;
use time::macros::{date, datetime};

fn main() {
    let day = date!(2026 - 10 - 25);
    let liefertag = DayBoundary::Strom.day(day).expect("a supported year");
    let (period_from, period_to) = liefertag.range();

    // The day's real length, resolved from the tz database — never assumed.
    let expected = liefertag
        .count(Resolution::QUARTER_HOUR)
        .expect("a quarter-hour divides a Berlin day");

    println!(
        "Liefertag {day} — {} h, {expected} quarter-hours",
        liefertag.duration().whole_hours()
    );
    println!("  {period_from} … {period_to} (UTC)\n");

    // ── 1. The Zählerstandsgang the gateway delivered ────────────────────────
    let readings = build_zaehlerstandsgang(period_from, expected);
    println!("1. Zählerstandsgang: {} readings", readings.len());

    // ── 2. Difference it into a Lastgang ─────────────────────────────────────
    //
    // The register width reconstructs a wrap past 999 999; the capacity cap
    // keeps an undocumented meter exchange from passing as one.
    let lastgang = to_lastgang(
        &readings,
        &LastgangConfig::strom()
            .register_digits(6)
            .capacity_kw(Decimal::from(30), Resolution::QUARTER_HOUR)
            .expect("no overflow"),
    )
    .into_iter()
    .next()
    .expect("one register, one Lastgang");
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
    // The declared period makes gap detection cover the head and tail of the
    // day. `now` is passed in; the crate reads no clock.
    let series = lastgang
        .to_series(Resolution::QUARTER_HOUR, DayBoundary::Strom)
        .expect("readings on the quarter-hour grid");
    let rules = Rules::strom(
        liefertag,
        datetime!(2026-11-01 0:00 UTC),
        Some(Decimal::from(30)),
    );
    let report = validate(&series, &rules);
    println!(
        "3. Validation: grade {}, {} finding(s)",
        report.grade(),
        report.findings.len(),
    );
    for f in report.at_least(Severity::Error) {
        println!("     ✗ {} {}", f.rule, f.message);
    }

    // ── 4. Ersatzwertbildung ─────────────────────────────────────────────────
    //
    // Each substitute records its method, the MSCONS codes (STS+Z32, STS+Z40)
    // and the values it was formed from.
    let policy = Policy::strom(Bundesland::Nw, SubstitutionReason::ImplausibleValue);
    let filled = substitute(&series, &report, &policy).expect("the report is this series'");
    println!(
        "4. Ersatzwerte: {} substituted, {} refused",
        filled.substitutes.len(),
        filled.refusals.len(),
    );
    for s in &filled.substitutes {
        println!(
            "     + {} {} {:?}/{} from {} value(s)",
            s.from,
            s.method,
            s.code,
            s.reason.code(),
            s.references.len(),
        );
    }

    // ── 5. The billing period ────────────────────────────────────────────────
    let period = aggregate(&filled.series, liefertag).expect("no overflow");
    println!("\n5. Abrechnung");
    println!("     Arbeitsmenge     {} kWh", period.arbeitsmenge);
    println!(
        "     Spitzenleistung  {} kW at {}",
        period.spitzenleistung_kw.unwrap_or_default(),
        period
            .spitzenleistung_at
            .map_or_else(|| "—".to_owned(), |t| calendar::to_berlin(t).to_string()),
    );
    println!(
        "     Coverage         {} %",
        period.coverage.pct().unwrap_or_default()
    );

    // ── 6. Split across the §14a Modul 3 registers ───────────────────────────
    //
    // Three levels. The Niedertarif band crosses midnight; HT/NT apply in Q1
    // and Q4 only (the Netzbetreiber's Wahlrecht).
    let zzd = Zaehlzeitdefinition::modul_3(
        "NB-14A-3",
        date!(2026 - 01 - 01),
        (17 * 60, 20 * 60), // Hochtarif   17:00–20:00 local
        (22 * 60, 6 * 60),  // Niedertarif 22:00–06:00 local, wrapping
        &[Quarter::Q1, Quarter::Q4],
    )
    .expect("valid bands")
    .in_land(Bundesland::Nw);

    let registers = zzd
        .split_energy(filled.series.as_slice())
        .expect("no overflow");
    println!("\n6. Zählzeitregister");
    for name in [HT, NT, ST] {
        let kwh = registers
            .per_register
            .get(name)
            .copied()
            .unwrap_or_default();
        println!("     {name}  {kwh:>10} kWh");
    }

    // ── 7. Validate again: the filled day grades clean ───────────────────────
    let graded = validate(&filled.series, &rules);
    println!(
        "\n7. Grade {} — {} findings",
        graded.grade(),
        graded.findings.len()
    );
    assert!(graded.grade() <= Grade::B, "{:?}", graded.findings);

    check_invariants(&filled.series, &period, &registers, expected);
}

/// A day of quarter-hourly Zählerstände with two planted defects.
fn build_zaehlerstandsgang(start: time::OffsetDateTime, count: u32) -> Vec<MeterReading> {
    // A six-digit register close to its 999 999 ceiling, so it wraps mid-day.
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

/// The properties a billing run depends on.
fn check_invariants(
    intervals: &Series,
    period: &metering::billing::aggregation::BillingPeriod,
    registers: &RegisterSplit<'_>,
    expected: u32,
) {
    assert_eq!(
        intervals.len(),
        expected as usize,
        "the filled series covers the 25-hour day exactly"
    );
    assert!(
        period.coverage.is_complete(),
        "every slot is accounted for after Ersatzwertbildung"
    );
    assert_eq!(
        registers.per_register.values().sum::<Decimal>(),
        period.arbeitsmenge,
        "the register split reconstructs the Arbeitsmenge exactly"
    );
    assert!(
        registers.is_complete(),
        "the Modul 3 fallback covers every instant of the day, and no quarter-hour straddles a band"
    );

    // The repeated 02:00–03:00 hour lies in the 22:00–06:00 Niedertarif band:
    // four more NT quarter-hours than an ordinary day.
    let nt_intervals = intervals
        .iter()
        .filter(|iv| {
            let h = calendar::to_berlin(iv.from()).hour();
            !(6..22).contains(&h)
        })
        .count();
    assert_eq!(nt_intervals, 36, "8 h of NT plus the repeated hour");

    println!("\n✓ invariants hold");
}
