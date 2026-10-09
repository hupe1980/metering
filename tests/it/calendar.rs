//! End-to-end DST behaviour, from the calendar primitives up through resampling.
//!
//! The numbers here are the ones a downstream completeness check has to agree
//! with, so they are asserted as literals rather than derived: 92 quarter-hours
//! on the spring-forward day, 100 on the fall-back day, 2 972 in March 2026.

use metering::vee::validation::{Rule, Rules, validate};
use metering::{
    DayBoundary, MeterInterval, QualityFlag, Resolution, Series, series::resample::resample,
    time::calendar,
};
use rust_decimal::{Decimal, dec};
use time::macros::{date, datetime};
use time::{Date, Duration, OffsetDateTime};

fn quarter_hour(from: OffsetDateTime, kwh: Decimal) -> MeterInterval {
    MeterInterval::new(
        from,
        from + Duration::minutes(15),
        kwh,
        QualityFlag::Measured,
    )
    .unwrap()
}

/// One full Berlin calendar day of quarter-hours, however long that day is.
fn full_day(day: Date) -> Vec<MeterInterval> {
    let count = metering::DayBoundary::Strom
        .day(day)
        .unwrap()
        .count(Resolution::QUARTER_HOUR)
        .expect("a day divides into quarter-hours");
    let start = metering::DayBoundary::Strom.day(day).unwrap().start();
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
        assert_eq!(
            metering::DayBoundary::Strom
                .day(day)
                .unwrap()
                .duration()
                .whole_hours(),
            hours,
            "{day}"
        );
        assert_eq!(
            metering::DayBoundary::Strom
                .day(day)
                .unwrap()
                .count(Resolution::QUARTER_HOUR),
            Some(quarters),
            "{day}"
        );
    }
}

/// March 2026 totals 2 972 quarter-hours, not 2 976.
#[test]
fn march_2026_is_four_intervals_short_of_thirty_one_days() {
    assert_eq!(
        metering::DayBoundary::Strom
            .month(
                (date!(2026 - 03 - 01)).year(),
                (date!(2026 - 03 - 01)).month()
            )
            .unwrap()
            .count(Resolution::QUARTER_HOUR),
        Some(2_972)
    );
    assert_eq!(31 * 96 - 2_972, 4, "exactly the lost hour");
}

fn qh_series(intervals: Vec<MeterInterval>) -> Series {
    Series::new(Resolution::QUARTER_HOUR, DayBoundary::Strom, intervals).unwrap()
}

/// A complete day resamples to one bucket at either transition.
#[test]
fn a_complete_dst_day_resamples_as_complete() {
    for day in [date!(2026 - 03 - 29), date!(2026 - 10 - 25)] {
        let intervals = full_day(day);
        let count = intervals.len();
        let buckets = resample(&qh_series(intervals), Resolution::Day).unwrap();

        assert_eq!(buckets.len(), 1, "{day} is one bucket");
        let bucket = &buckets.as_slice()[0];
        let period = DayBoundary::Strom.day(day).unwrap();
        assert_eq!((bucket.from(), bucket.to()), period.range(), "{day}");
        assert_eq!(bucket.value(), Decimal::from(count));
    }
}

/// The autumn failure mode the whole module exists to prevent: 96 intervals on
/// a 25-hour day is a four-interval gap, and must not read as a full day.
#[test]
fn ninety_six_intervals_on_the_long_day_is_a_gap() {
    let day = date!(2026 - 10 - 25);
    let period = DayBoundary::Strom.day(day).unwrap();
    let intervals: Vec<_> = (0..96)
        .map(|i| quarter_hour(period.start() + Duration::minutes(15 * i), dec!(1)))
        .collect();
    let series = qh_series(intervals);

    assert!(
        resample(&series, Resolution::Day).unwrap().is_empty(),
        "an incomplete day is no daily total"
    );
    let report = validate(
        &series,
        &Rules::strom(period, datetime!(2027-01-01 0:00 UTC), None),
    );
    let gap = report.by_rule(Rule::Gap).next().unwrap();
    assert_eq!(gap.reference, Some(dec!(4)), "a flat 96 would hide four");
    assert_eq!(report.coverage.pct(), Some(dec!(96)));
}

/// The spring mirror image: 96 intervals on a 23-hour day overshoots into the
/// next day rather than filling this one.
#[test]
fn ninety_six_intervals_on_the_short_day_spills_over() {
    let day = date!(2026 - 03 - 29);
    let start = DayBoundary::Strom.day(day).unwrap().start();
    let intervals: Vec<_> = (0..96)
        .map(|i| quarter_hour(start + Duration::minutes(15 * i), dec!(1)))
        .collect();

    let buckets = resample(&qh_series(intervals), Resolution::Day).unwrap();
    assert_eq!(
        buckets.len(),
        1,
        "the 29th is full at 92; four slots of the 30th are not a day"
    );
    assert_eq!(buckets.as_slice()[0].value(), dec!(92));
}

/// A German month is not a UTC month: the first hour belongs to the new month.
#[test]
fn month_totals_use_the_german_boundary() {
    // 23:00 UTC on 31 December is 00:00 on 1 January in Berlin.
    let new_year = datetime!(2025-12-31 23:00 UTC);
    assert_eq!(
        DayBoundary::Strom.day_of(new_year).unwrap(),
        date!(2026 - 01 - 01)
    );

    let days: Vec<MeterInterval> = (0..62)
        .map(|d| {
            let p = DayBoundary::Strom
                .day(date!(2025 - 12 - 01) + Duration::days(d))
                .unwrap();
            let kwh = if d < 31 { dec!(10) } else { dec!(1) };
            MeterInterval::new(p.start(), p.end(), kwh, QualityFlag::Measured).unwrap()
        })
        .collect();
    let daily = Series::new(Resolution::Day, DayBoundary::Strom, days).unwrap();
    let months = resample(&daily, Resolution::Month).unwrap();
    assert_eq!(months.len(), 2);
    assert_eq!(months.as_slice()[0].value(), dec!(310), "December");
    assert_eq!(months.as_slice()[1].value(), dec!(31), "January");
    assert_eq!(
        months.as_slice()[1].from(),
        DayBoundary::Strom.year(2026).unwrap().start()
    );
}

/// Days tile a whole year exactly: no interval is dropped or double-counted at
/// either transition, and the two cancel over the year.
#[test]
fn a_year_of_days_tiles_without_loss() {
    let mut day = date!(2026 - 01 - 01);
    let mut total = 0u32;
    let mut cursor = metering::DayBoundary::Strom.day(day).unwrap().start();

    while day.year() == 2026 {
        assert_eq!(
            metering::DayBoundary::Strom.day(day).unwrap().start(),
            cursor,
            "{day} must abut"
        );
        total += metering::DayBoundary::Strom
            .day(day)
            .unwrap()
            .count(Resolution::QUARTER_HOUR)
            .unwrap();
        cursor = metering::DayBoundary::Strom.day(day).unwrap().end();
        day = day.next_day().unwrap();
    }

    assert_eq!(
        cursor,
        metering::DayBoundary::Strom.year(2026).unwrap().end()
    );
    assert_eq!(total, 365 * 96);
    assert_eq!(
        metering::DayBoundary::Strom
            .year(2026)
            .unwrap()
            .count(Resolution::QUARTER_HOUR),
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
            let start = boundary.day(day).unwrap().start();
            let end = boundary.day(day).unwrap().end();
            prop_assert!(start < end, "{boundary:?} {day}");

            let next = day.next_day().expect("in range");
            prop_assert_eq!(
                end,
                boundary.day(next).unwrap().start(),
                "{:?}: {} must end where {} begins",
                boundary,
                day,
                next,
            );

            // Every instant of the day maps back to it — including the last.
            for probe in [start, start + Duration::seconds(1), end - Duration::seconds(1)] {
                prop_assert_eq!(boundary.day_of(probe).unwrap(), day, "{:?} {}", boundary, probe);
            }
            prop_assert_eq!(boundary.day_of(end).unwrap(), next, "the end is exclusive");

            // A German day is 23, 24 or 25 hours. Nothing else, ever.
            let hours = boundary.day(day).unwrap().duration().whole_hours();
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
            quarters += metering::DayBoundary::Strom.day(cursor).unwrap().count(Resolution::QUARTER_HOUR)
                .expect("a day divides into quarter-hours");
            days += 1;
            cursor = cursor.next_day().expect("in range");
        }
        prop_assert_eq!(
            metering::DayBoundary::Strom.month((day).year(), (day).month()).unwrap().count(Resolution::QUARTER_HOUR),
            Some(quarters),
        );
        prop_assert_eq!(
            metering::DayBoundary::Strom.month((day).year(), (day).month()).unwrap().count(Resolution::Day),
            Some(days),
        );
        prop_assert_eq!(u32::from(time::util::days_in_month((day).month(), (day).year())), days);

        // …and the month tiles into the year.
        prop_assert_eq!(
            metering::DayBoundary::Strom.month((day).year(), (day).month()).unwrap().end() - metering::DayBoundary::Strom.month((day).year(), (day).month()).unwrap().start(),
            metering::DayBoundary::Strom.month((day).year(), (day).month()).unwrap().duration(),
        );
        prop_assert!(metering::DayBoundary::Strom.month((day).year(), (day).month()).unwrap().start() <= metering::DayBoundary::Strom.day(day).unwrap().start());
        prop_assert!(metering::DayBoundary::Strom.day(day).unwrap().start() < metering::DayBoundary::Strom.month((day).year(), (day).month()).unwrap().end());
    }

    /// A year is the sum of its months and of its days, and the two DST
    /// transitions cancel inside it.
    #[test]
    fn a_year_is_the_sum_of_its_parts(year in 1996i32..2065) {
        let mut quarters = 0u32;
        let mut day = Date::from_ordinal_date(year, 1).expect("valid");
        let mut cursor = metering::DayBoundary::Strom.day(day).unwrap().start();
        while day.year() == year {
            prop_assert_eq!(metering::DayBoundary::Strom.day(day).unwrap().start(), cursor, "{} must abut", day);
            quarters += metering::DayBoundary::Strom.day(day).unwrap().count(Resolution::QUARTER_HOUR)
                .expect("divides");
            cursor = metering::DayBoundary::Strom.day(day).unwrap().end();
            day = day.next_day().expect("in range");
        }
        prop_assert_eq!(cursor, metering::DayBoundary::Strom.year(year).unwrap().end());
        prop_assert_eq!(
            metering::DayBoundary::Strom.year(year).unwrap().count(Resolution::QUARTER_HOUR),
            Some(quarters),
        );
        prop_assert_eq!(
            u32::from(time::util::days_in_year(year)) * 96,
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
        let instant = metering::DayBoundary::Strom.day(day).unwrap().start() + Duration::minutes(minute);
        let shifted = metering::time::calendar::shift_back_days(instant, back).unwrap();
        prop_assert!(shifted < instant, "the shift must go backwards");
        prop_assert_eq!(metering::DayBoundary::Strom.days_between(shifted, instant).unwrap(), back);
        prop_assert_eq!(metering::DayBoundary::Strom.days_between(instant, shifted).unwrap(), -back, "antisymmetric");

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
        let instant = metering::DayBoundary::Strom.day(day).unwrap().start() + Duration::minutes(minute);
        let calendar_day = metering::DayBoundary::Strom.day_of(instant).unwrap();
        let gas_day = metering::DayBoundary::Gas.day_of(instant).unwrap();
        let before_six = calendar::to_berlin(instant).time() < time::macros::time!(6:00);

        prop_assert_eq!(
            gas_day,
            if before_six {
                calendar_day.previous_day().expect("in range")
            } else {
                calendar_day
            },
        );
        prop_assert!(metering::DayBoundary::Gas.day(gas_day).unwrap().start() <= instant);
        prop_assert!(instant < metering::DayBoundary::Gas.day(gas_day).unwrap().end());
    }
}
