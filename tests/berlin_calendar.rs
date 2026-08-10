//! End-to-end DST behaviour, from the calendar primitives up through resampling.
//!
//! The numbers here are the ones a downstream completeness check has to agree
//! with, so they are asserted as literals rather than derived: 92 quarter-hours
//! on the spring-forward day, 100 on the fall-back day, 2 972 in March 2026.

use metering::{
    IntervalResolution, MeterInterval, QualityFlag, ResampleConfig, calendar, resample,
};
use rust_decimal::{Decimal, dec};
use time::macros::{date, datetime};
use time::{Date, Duration, OffsetDateTime};

fn quarter_hour(from: OffsetDateTime, kwh: Decimal) -> MeterInterval {
    MeterInterval {
        from,
        to: from + Duration::minutes(15),
        value: kwh,
        quality: QualityFlag::Measured,
        obis_code: None,
    }
}

/// One full Berlin calendar day of quarter-hours, however long that day is.
fn full_day(day: Date) -> Vec<MeterInterval> {
    let count = calendar::intervals_in_day(day, IntervalResolution::QuarterHour)
        .expect("a day divides into quarter-hours");
    let start = calendar::day_start_utc(day);
    (0..i64::from(count))
        .map(|i| quarter_hour(start + Duration::minutes(15 * i), dec!(1)))
        .collect()
}

/// The reference table: every DST transition day in the 2025–2027 window.
#[test]
fn transition_day_lengths_match_the_reference_table() {
    let table = [
        (date!(2025 - 03 - 30), 23, 92),
        (date!(2025 - 10 - 26), 25, 100),
        (date!(2026 - 03 - 29), 23, 92),
        (date!(2026 - 07 - 20), 24, 96),
        (date!(2026 - 10 - 25), 25, 100),
        (date!(2027 - 03 - 28), 23, 92),
        (date!(2027 - 10 - 31), 25, 100),
    ];
    for (day, hours, quarters) in table {
        assert_eq!(calendar::day_length(day).whole_hours(), hours, "{day}");
        assert_eq!(
            calendar::intervals_in_day(day, IntervalResolution::QuarterHour),
            Some(quarters),
            "{day}"
        );
    }
}

/// March 2026 totals 2 972 quarter-hours, not 2 976.
#[test]
fn march_2026_is_four_intervals_short_of_thirty_one_days() {
    assert_eq!(
        calendar::intervals_in_month(date!(2026 - 03 - 01), IntervalResolution::QuarterHour),
        Some(2_972)
    );
    assert_eq!(31 * 96 - 2_972, 4, "exactly the lost hour");
}

/// A complete day resamples to one complete bucket at either transition.
#[test]
fn a_complete_dst_day_resamples_as_complete() {
    for day in [date!(2026 - 03 - 29), date!(2026 - 10 - 25)] {
        let intervals = full_day(day);
        let buckets = resample(&intervals, &ResampleConfig::to_daily());

        assert_eq!(buckets.len(), 1, "{day} is one bucket");
        let bucket = &buckets[0];
        assert_eq!(bucket.from, calendar::day_start_utc(day), "{day} start");
        assert_eq!(bucket.to, calendar::day_end_utc(day), "{day} end");
        assert_eq!(
            bucket.expected_count,
            Some(bucket.interval_count),
            "{day} must be complete"
        );
        assert_eq!(bucket.is_complete(), Some(true), "{day}");
        assert!(!bucket.has_missing_data(), "{day}");
        assert!(!bucket.has_surplus_data(), "{day}");
        assert_eq!(bucket.total, Decimal::from(bucket.interval_count));
    }
}

/// The autumn failure mode the whole module exists to prevent: 96 intervals on
/// a 25-hour day is a four-interval gap, and must not read as a full day.
#[test]
fn ninety_six_intervals_on_the_long_day_is_a_gap() {
    let day = date!(2026 - 10 - 25);
    let start = calendar::day_start_utc(day);
    let intervals: Vec<_> = (0..96)
        .map(|i| quarter_hour(start + Duration::minutes(15 * i), dec!(1)))
        .collect();

    let buckets = resample(&intervals, &ResampleConfig::to_daily());
    assert_eq!(buckets[0].expected_count, Some(100));
    assert_eq!(buckets[0].interval_count, 96);
    assert!(
        buckets[0].has_missing_data(),
        "a flat 96 would have hidden four missing intervals"
    );
    assert!((buckets[0].coverage_pct().unwrap() - 96.0).abs() < 0.01);
}

/// The spring mirror image: 96 intervals on a 23-hour day overshoots into the
/// next day rather than filling this one.
#[test]
fn ninety_six_intervals_on_the_short_day_spills_over() {
    let day = date!(2026 - 03 - 29);
    let start = calendar::day_start_utc(day);
    let intervals: Vec<_> = (0..96)
        .map(|i| quarter_hour(start + Duration::minutes(15 * i), dec!(1)))
        .collect();

    let buckets = resample(&intervals, &ResampleConfig::to_daily());
    assert_eq!(buckets.len(), 2, "the last four intervals are the 30th");
    assert_eq!(buckets[0].interval_count, 92);
    assert_eq!(
        buckets[0].is_complete(),
        Some(true),
        "the 29th is full at 92"
    );
    assert_eq!(buckets[1].interval_count, 4);
}

/// A German month is not a UTC month: the first hour belongs to the new month.
#[test]
fn month_totals_use_the_german_boundary() {
    // 23:00 UTC on 31 December is 00:00 on 1 January in Berlin.
    let new_year = datetime!(2025-12-31 23:00 UTC);
    assert_eq!(calendar::local_day(new_year), date!(2026 - 01 - 01));

    let intervals = vec![
        quarter_hour(new_year - Duration::minutes(15), dec!(10)), // still December
        quarter_hour(new_year, dec!(1)),                          // January
    ];
    let buckets = resample(&intervals, &ResampleConfig::to_monthly());
    assert_eq!(buckets.len(), 2);
    assert_eq!(buckets[0].total, dec!(10), "December");
    assert_eq!(buckets[1].total, dec!(1), "January");
    assert_eq!(buckets[1].from, calendar::year_start_utc(2026));
}

/// Days tile a whole year exactly: no interval is dropped or double-counted at
/// either transition, and the two cancel over the year.
#[test]
fn a_year_of_days_tiles_without_loss() {
    let mut day = date!(2026 - 01 - 01);
    let mut total = 0u32;
    let mut cursor = calendar::day_start_utc(day);

    while day.year() == 2026 {
        assert_eq!(calendar::day_start_utc(day), cursor, "{day} must abut");
        total += calendar::intervals_in_day(day, IntervalResolution::QuarterHour).unwrap();
        cursor = calendar::day_end_utc(day);
        day = day.next_day().unwrap();
    }

    assert_eq!(cursor, calendar::year_end_utc(2026));
    assert_eq!(total, 365 * 96);
    assert_eq!(
        calendar::intervals_in_year(2026, IntervalResolution::QuarterHour),
        Some(total)
    );
}
