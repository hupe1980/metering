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

use metering::session::{MeterSample, SessionSplitConfig, split_session};
use metering::{
    AggregationConfig, FillGapsConfig, IntervalResolution, MeterInterval, QualityConfig,
    QualityFlag, ResampleConfig, ValidationConfig, aggregate, calendar, fill_gaps, resample,
    score_intervals, validate_intervals,
};
use rust_decimal::{Decimal, dec};
use time::Duration;
use time::macros::date;

/// Generous enough that only a change of complexity class can exceed it.
const BUDGET: StdDuration = StdDuration::from_secs(60);

/// One Berlin year of quarter-hours, with a plausible daily shape.
fn year_2026() -> Vec<MeterInterval> {
    let start = calendar::day_start_utc(date!(2026 - 01 - 01));
    let end = calendar::day_start_utc(date!(2027 - 01 - 01));
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
            MeterInterval::quarter_hour(start + Duration::minutes(15 * i), kwh)
        })
        .collect()
}

#[test]
fn a_settlement_year_runs_in_linear_time() {
    let started = Instant::now();
    let mut year = year_2026();
    assert_eq!(year.len(), 35_040, "2026 is not a leap year");

    // Plant a hundred scattered outages: each one makes `fill_gaps` measure a
    // run and look ahead for its closing value.
    for i in (0..year.len()).step_by(347) {
        year[i].quality = QualityFlag::Faulty;
    }

    // 1. Validation, including the Hampel window over every point.
    let report = validate_intervals(
        &year,
        &ValidationConfig::default().over_period(year[0].from, year[year.len() - 1].to),
    );
    assert!(report.evaluated.len() >= 8);

    // 2. Grading, which runs validation again and folds it.
    let graded = score_intervals(&year, &QualityConfig::default());
    assert_eq!(graded.intervals_analysed, year.len());

    // 3. Aggregation over the whole year.
    let period = aggregate(&year, &AggregationConfig::rlm());
    assert!(period.uniform_resolution);
    assert!(period.benutzungsdauer_h().is_some());

    // 4. Resampling to every coarser grid.
    assert_eq!(resample(&year, &ResampleConfig::to_hourly()).len(), 8_760);
    assert_eq!(resample(&year, &ResampleConfig::to_daily()).len(), 365);
    assert_eq!(resample(&year, &ResampleConfig::to_monthly()).len(), 12);
    assert_eq!(resample(&year, &ResampleConfig::to_yearly()).len(), 1);

    // 5. Gap filling across the whole year: 35 040 slots, a hundred holes.
    let billable: Vec<MeterInterval> = year
        .iter()
        .filter(|iv| iv.quality.is_billable())
        .cloned()
        .collect();
    let filled = fill_gaps(
        &billable,
        &FillGapsConfig::new(
            IntervalResolution::QuarterHour,
            year[0].from,
            year[year.len() - 1].to,
        ),
    );
    assert_eq!(filled.intervals.len(), year.len());
    assert_eq!(filled.substituted_count(), 101);
    assert!(filled.placed_everything());

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
    let start = calendar::day_start_utc(date!(2026 - 06 - 01));
    let samples: Vec<MeterSample> = (0..=1_440)
        .map(|m| MeterSample::new(start + Duration::minutes(m), Decimal::from(m)))
        .collect();

    let slots = split_session(
        start,
        start + Duration::hours(24),
        Decimal::from(1_440u32),
        &samples,
        &SessionSplitConfig::quarter_hourly(),
    )
    .expect("a well-formed day");

    assert_eq!(slots.len(), 96);
    // The conservation identity, at scale.
    assert_eq!(
        slots.iter().map(|s| s.value).sum::<Decimal>(),
        Decimal::from(1_440u32)
    );
    // Every slot is bounded by two samples, so every slot is measured.
    assert!(slots.iter().all(|s| s.quality == QualityFlag::Measured));

    assert!(started.elapsed() < BUDGET, "{:?}", started.elapsed());
}
