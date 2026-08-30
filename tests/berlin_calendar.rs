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

// ── the same properties, over generated dates ────────────────────────────────
//
// The literals above pin the numbers a completeness check has to agree with,
// and they pin them for 2026. These say the *structure* holds for any date:
// consecutive periods abut, an instant lands in the period that contains it,
// and a coarse count is the sum of the fine ones it is made of. A tz-database
// update that moved a transition, or an off-by-one in a month count, shows up
// here rather than in a customer's Jahresabrechnung.

use metering::DayBoundary;
use proptest::prelude::*;

/// Any day from 1996 to 2065 — inside `Date`'s range, and either side of every
/// rule change the tz database records for this zone.
fn arb_day() -> impl Strategy<Value = Date> {
    (1996i32..2065, 1u16..=366).prop_filter_map("a real ordinal", |(year, ordinal)| {
        Date::from_ordinal_date(year, ordinal).ok()
    })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    /// Consecutive days tile the timeline, on both boundaries, and an instant
    /// belongs to the day whose bounds contain it.
    #[test]
    fn days_tile_and_contain_their_own_instants(day in arb_day()) {
        for boundary in DayBoundary::ALL {
            let start = boundary.day_start_utc(day);
            let end = boundary.day_end_utc(day);
            prop_assert!(start < end, "{boundary:?} {day}");

            let next = day.next_day().expect("in range");
            prop_assert_eq!(
                end,
                boundary.day_start_utc(next),
                "{:?}: {} must end where {} begins",
                boundary,
                day,
                next,
            );

            // Every instant of the day maps back to it — including the last.
            for probe in [start, start + Duration::seconds(1), end - Duration::seconds(1)] {
                prop_assert_eq!(boundary.local_day(probe), day, "{:?} {}", boundary, probe);
            }
            prop_assert_eq!(boundary.local_day(end), next, "the end is exclusive");

            // A German day is 23, 24 or 25 hours. Nothing else, ever.
            let hours = boundary.day_length(day).whole_hours();
            prop_assert!((23..=25).contains(&hours), "{boundary:?} {day}: {hours} h");
        }
    }

    /// A coarse interval count is the sum of the fine ones inside it — which is
    /// the whole reason `intervals_in_month` exists rather than `days × 96`.
    #[test]
    fn interval_counts_compose(day in arb_day()) {
        let first = Date::from_calendar_date(day.year(), day.month(), 1).expect("valid");
        let mut cursor = first;
        let mut quarters = 0u32;
        let mut days = 0u32;
        while cursor.month() == day.month() && cursor.year() == day.year() {
            quarters += calendar::intervals_in_day(cursor, IntervalResolution::QuarterHour)
                .expect("a day divides into quarter-hours");
            days += 1;
            cursor = cursor.next_day().expect("in range");
        }
        prop_assert_eq!(
            calendar::intervals_in_month(day, IntervalResolution::QuarterHour),
            Some(quarters),
        );
        prop_assert_eq!(
            calendar::intervals_in_month(day, IntervalResolution::Day),
            Some(days),
        );
        prop_assert_eq!(u32::from(calendar::days_in_month(day)), days);

        // …and the month tiles into the year.
        prop_assert_eq!(
            calendar::month_end_utc(day) - calendar::month_start_utc(day),
            calendar::month_length(day),
        );
        prop_assert!(calendar::month_start_utc(day) <= calendar::day_start_utc(day));
        prop_assert!(calendar::day_start_utc(day) < calendar::month_end_utc(day));
    }

    /// A year is the sum of its months and of its days, and the two DST
    /// transitions cancel inside it.
    #[test]
    fn a_year_is_the_sum_of_its_parts(year in 1996i32..2065) {
        let mut quarters = 0u32;
        let mut day = Date::from_ordinal_date(year, 1).expect("valid");
        let mut cursor = calendar::day_start_utc(day);
        while day.year() == year {
            prop_assert_eq!(calendar::day_start_utc(day), cursor, "{} must abut", day);
            quarters += calendar::intervals_in_day(day, IntervalResolution::QuarterHour)
                .expect("divides");
            cursor = calendar::day_end_utc(day);
            day = day.next_day().expect("in range");
        }
        prop_assert_eq!(cursor, calendar::year_end_utc(year));
        prop_assert_eq!(
            calendar::intervals_in_year(year, IntervalResolution::QuarterHour),
            Some(quarters),
        );
        prop_assert_eq!(
            u32::from(calendar::days_in_year(year)) * 96,
            quarters,
            "the transitions cancel over a full year",
        );
    }

    /// Stepping back `n` calendar days and counting forward again returns `n`,
    /// whatever lies in between — which is the property a Vergleichstag window
    /// and a Jahresprognose both rest on.
    #[test]
    fn stepping_back_and_counting_forward_agree(
        day in arb_day(),
        minute in 0i64..1440,
        back in 1i64..400,
    ) {
        let instant = calendar::day_start_utc(day) + Duration::minutes(minute);
        let shifted = calendar::shift_back_days(instant, back);
        prop_assert!(shifted < instant, "the shift must go backwards");
        prop_assert_eq!(calendar::days_between(shifted, instant), back);
        prop_assert_eq!(calendar::days_between(instant, shifted), -back, "antisymmetric");

        // The local wall clock is preserved, except where the clocks skipped
        // that time — there it resolves forward, never backward.
        let want = calendar::to_berlin(instant).time();
        let got = calendar::to_berlin(shifted).time();
        prop_assert!(got >= want, "a skipped hour resolves forward: {got} < {want}");
    }

    /// `local_day` and `local_gas_day` differ by exactly the six-hour cut:
    /// before 06:00 local, the Gastag is the previous calendar day.
    #[test]
    fn the_gastag_is_the_calendar_day_cut_six_hours_later(
        day in arb_day(),
        minute in 0i64..1440,
    ) {
        let instant = calendar::day_start_utc(day) + Duration::minutes(minute);
        let calendar_day = calendar::local_day(instant);
        let gas_day = calendar::local_gas_day(instant);
        let before_six = calendar::to_berlin(instant).time() < time::macros::time!(6:00);

        prop_assert_eq!(
            gas_day,
            if before_six {
                calendar_day.previous_day().expect("in range")
            } else {
                calendar_day
            },
        );
        prop_assert!(calendar::gas_day_start_utc(gas_day) <= instant);
        prop_assert!(instant < calendar::gas_day_end_utc(gas_day));
    }
}
