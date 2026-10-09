//! A full settlement year through the pipeline, once.
//!
//! Not a benchmark: there is no timing to compare against and no measurement to
//! report. It is a **smoke alarm for accidental quadratic behaviour**, which is
//! the one performance defect that turns a working library into an unusable one
//! and which reading alone has already missed once — `split_session` rescanned
//! its segment list per slot until a self-audit found it.
//!
//! A year of quarter-hours is 35 040 intervals. Every entry point below is
//! linear or `n log n` in that, so the whole file runs in well under a second in
//! release and a few seconds in debug. A quadratic path is 1.2 billion
//! operations and takes long enough that the budget catches it without being
//! tight enough to fail on a loaded machine.

use std::time::{Duration as StdDuration, Instant};

use metering::allocation::session::{MeterSample, SessionSplitConfig, split_session};
use metering::time::holiday::Bundesland;
use metering::vee::substitute::{Policy, SubstitutionReason, substitute};
use metering::vee::validation::{Grade, Rules, validate};
use metering::{
    DayBoundary, MeterInterval, QualityFlag, Resolution, Series, billing::aggregation::aggregate,
    series::resample::resample,
};
use rust_decimal::{Decimal, dec};
use time::Duration;
use time::macros::date;

/// Generous enough that only a change of complexity class can exceed it.
const BUDGET: StdDuration = StdDuration::from_secs(60);

/// One Berlin year of quarter-hours, with a plausible daily shape.
fn year_2026() -> Vec<MeterInterval> {
    let start = metering::DayBoundary::Strom
        .day(date!(2026 - 01 - 01))
        .unwrap()
        .start();
    let end = metering::DayBoundary::Strom
        .day(date!(2027 - 01 - 01))
        .unwrap()
        .start();
    let count = (end - start).whole_seconds() / 900;
    (0..count)
        .map(|i| {
            // A coarse shape: a night trough and a daytime plateau, on a grid
            // coarse enough that ties are common — the same reason
            // `order_independence`'s generator is coarse.
            let quarter_of_day = i % 96;
            let kwh = if (24..80).contains(&quarter_of_day) {
                dec!(2.5)
            } else {
                dec!(0.5)
            };
            MeterInterval::quarter_hour(
                start + Duration::minutes(15 * i),
                kwh,
                metering::QualityFlag::Measured,
            )
            .unwrap()
        })
        .collect()
}

#[test]
fn a_settlement_year_runs_in_linear_time() {
    let started = Instant::now();
    let mut year = year_2026();
    assert_eq!(year.len(), 35_040, "2026 is not a leap year");

    // Plant a hundred scattered faulty values and one long outage: the
    // first needs interpolation, the second the Vergleichswert over the year.
    for i in (1..year.len()).step_by(347) {
        year[i] = year[i].clone().with_quality(QualityFlag::Faulty);
    }
    year.drain(20_000..20_200);
    let series = Series::new(Resolution::QUARTER_HOUR, DayBoundary::Strom, year).unwrap();
    let period = DayBoundary::Strom.year(2026).unwrap();
    let rules = Rules::strom(period, period.end(), None).seasonal(Bundesland::Be, 4);

    // 1. Validation, including the Hampel window and the seasonal references.
    let report = validate(&series, &rules);
    assert_eq!(report.grade(), Grade::C);

    // 2. Substitution of every rejected and missing slot.
    let policy = Policy::strom(Bundesland::Be, SubstitutionReason::CommunicationFailure);
    let filled = substitute(&series, &report, &policy).unwrap();
    assert_eq!(filled.series.len(), 35_040);
    assert_eq!(filled.substitutes.len(), 300);
    assert!(filled.refusals.is_empty());

    // 3. Aggregation over the whole year.
    let billed = aggregate(&filled.series, period).unwrap();
    assert!(billed.coverage.is_complete());
    assert!(billed.benutzungsdauer_h().is_some());

    // 4. Resampling to every coarser grid.
    let s = &filled.series;
    assert_eq!(resample(s, Resolution::Hour).unwrap().len(), 8_760);
    assert_eq!(resample(s, Resolution::Day).unwrap().len(), 365);
    assert_eq!(resample(s, Resolution::Month).unwrap().len(), 12);
    assert_eq!(resample(s, Resolution::Year).unwrap().len(), 1);

    assert!(
        started.elapsed() < BUDGET,
        "a settlement year took {:?} — that is a change of complexity class, not a slow machine",
        started.elapsed()
    );
}

/// A day-long device log sampled every minute, placed on the quarter-hour grid.
///
/// 1 440 samples against 96 slots is where a per-slot rescan of the segment list
/// shows up: linear it is 1 440 steps, quadratic it is 138 240.
#[test]
fn a_minute_sampled_day_places_in_linear_time() {
    let started = Instant::now();
    let start = metering::DayBoundary::Strom
        .day(date!(2026 - 06 - 01))
        .unwrap()
        .start();
    let samples: Vec<MeterSample> = (0..=1_440)
        .map(|m| MeterSample::new(start + Duration::minutes(m), Decimal::from(m)))
        .collect();

    let series =
        split_session(&samples, &SessionSplitConfig::quarter_hourly()).expect("a well-formed day");
    let slots = series.as_slice();

    assert_eq!(slots.len(), 96);
    // The conservation identity, at scale.
    assert_eq!(
        slots.iter().map(|s| s.value()).sum::<Decimal>(),
        Decimal::from(1_440u32)
    );
    // Every slot is bounded by two samples, so every slot is measured.
    assert!(slots.iter().all(|s| s.quality() == QualityFlag::Measured));

    assert!(started.elapsed() < BUDGET, "{:?}", started.elapsed());
}
