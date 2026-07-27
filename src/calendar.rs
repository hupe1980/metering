//! Europe/Berlin calendar arithmetic — the German market's day, month and year.
//!
//! ## Why this module exists
//!
//! German metering periods are **local calendar periods**, not UTC ones. A
//! Bilanzierungstag, a Liefermonat and a §13 StromNZV Abrechnungsmonat all begin
//! at 00:00 Europe/Berlin, which is 23:00 UTC the previous day in winter and
//! 22:00 UTC in summer. Grouping intervals by their UTC date therefore books the
//! first hour of every German day into the previous one — silently, for every
//! metering point, on every day of the year.
//!
//! The DST transitions add a second failure mode. A German calendar day is
//! **not** 24 hours long:
//!
//! | Day | Length | Quarter-hours |
//! |---|---|---|
//! | ordinary | 24 h | 96 |
//! | last Sunday in March (spring forward) | **23 h** | **92** |
//! | last Sunday in October (fall back) | **25 h** | **100** |
//!
//! A completeness check built on a hard-coded 96 raises a false alarm every
//! spring and — worse — **masks a genuine four-interval gap every autumn**,
//! because 96 of an expected 100 intervals looks complete.
//!
//! ## Scope
//!
//! Everything here is Europe/Berlin, which is the market area this crate models
//! (§ 20 StromNZV / GPKE settle in German local time). The rules come from the
//! IANA tz database via `time-tz`, so historical transitions are correct too —
//! this is not a hard-coded "last Sunday in March" approximation.
//! [`berlin`] exposes the timezone for callers needing something else.
//!
//! ## Example
//!
//! ```rust
//! use metering::calendar;
//! use metering::IntervalResolution;
//! use time::macros::date;
//!
//! // The spring-forward day is 23 hours long — 92 quarter-hours, not 96.
//! assert_eq!(
//!     calendar::intervals_in_day(date!(2026 - 03 - 29), IntervalResolution::QuarterHour),
//!     Some(92),
//! );
//! // March 2026 therefore holds 2 972 quarter-hours, not 31 × 96 = 2 976.
//! assert_eq!(
//!     calendar::intervals_in_month(date!(2026 - 03 - 01), IntervalResolution::QuarterHour),
//!     Some(2_972),
//! );
//! ```

use time::{Date, Duration, Month, OffsetDateTime, PrimitiveDateTime, Time};
use time_tz::{OffsetDateTimeExt as _, OffsetResult, PrimitiveDateTimeExt as _, Tz, timezones};

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use crate::resolution::IntervalResolution;

// ── timezone ──────────────────────────────────────────────────────────────────

/// The Europe/Berlin timezone, from the IANA tz database.
///
/// Exposed so callers can perform conversions this module does not cover
/// without taking their own `time-tz` dependency (and risking a different tz
/// database version than the one this crate computed with).
#[must_use]
pub fn berlin() -> &'static Tz {
    timezones::db::europe::BERLIN
}

/// Convert a UTC instant to Europe/Berlin local time.
#[must_use]
pub fn to_berlin(instant: OffsetDateTime) -> OffsetDateTime {
    instant.to_timezone(berlin())
}

// ── instant → calendar period ─────────────────────────────────────────────────

/// The Berlin calendar day an instant falls on.
///
/// This is the correct grouping key for daily aggregation. Using
/// `instant.date()` instead groups by UTC day and misbooks the first
/// hour (winter) or two hours (summer) of every German day.
///
/// ```rust
/// use metering::calendar;
/// use time::macros::{date, datetime};
///
/// // 23:30 UTC on 14 July is already 01:30 on 15 July in Berlin (CEST).
/// assert_eq!(calendar::local_day(datetime!(2026-07-14 23:30 UTC)), date!(2026 - 07 - 15));
/// ```
#[must_use]
pub fn local_day(instant: OffsetDateTime) -> Date {
    to_berlin(instant).date()
}

/// The Berlin calendar month an instant falls in, as its first day.
#[must_use]
pub fn local_month(instant: OffsetDateTime) -> Date {
    first_of_month(local_day(instant))
}

/// The Berlin calendar year an instant falls in.
#[must_use]
pub fn local_year(instant: OffsetDateTime) -> i32 {
    local_day(instant).year()
}

// ── calendar period → UTC instant ─────────────────────────────────────────────

/// The UTC instant at which a Berlin calendar day begins (00:00 local).
///
/// ```rust
/// use metering::calendar;
/// use time::macros::{date, datetime};
///
/// // Winter: CET = UTC+1, so the day starts at 23:00 UTC the day before.
/// assert_eq!(calendar::day_start_utc(date!(2026 - 01 - 15)), datetime!(2026-01-14 23:00 UTC));
/// // Summer: CEST = UTC+2.
/// assert_eq!(calendar::day_start_utc(date!(2026 - 07 - 15)), datetime!(2026-07-14 22:00 UTC));
/// ```
#[must_use]
pub fn day_start_utc(day: Date) -> OffsetDateTime {
    local_midnight_utc(day).to_offset(time::UtcOffset::UTC)
}

/// The UTC instant at which a Berlin calendar day ends (exclusive).
///
/// Equal to `day_start_utc` of the following day, so consecutive days tile the
/// timeline without gaps or overlaps across DST transitions.
#[must_use]
pub fn day_end_utc(day: Date) -> OffsetDateTime {
    day_start_utc(day.next_day().unwrap_or(day))
}

/// The UTC instant at which the Berlin calendar month containing `day` begins.
#[must_use]
pub fn month_start_utc(day: Date) -> OffsetDateTime {
    day_start_utc(first_of_month(day))
}

/// The UTC instant at which the Berlin calendar month containing `day` ends
/// (exclusive) — i.e. the start of the following month.
#[must_use]
pub fn month_end_utc(day: Date) -> OffsetDateTime {
    day_start_utc(first_of_next_month(day))
}

/// The UTC instant at which a Berlin calendar year begins.
#[must_use]
pub fn year_start_utc(year: i32) -> OffsetDateTime {
    day_start_utc(jan_first(year))
}

/// The UTC instant at which a Berlin calendar year ends (exclusive).
#[must_use]
pub fn year_end_utc(year: i32) -> OffsetDateTime {
    day_start_utc(jan_first(year + 1))
}

// ── period lengths ────────────────────────────────────────────────────────────

/// How long a Berlin calendar day lasts: 23 h, 24 h or 25 h.
///
/// ```rust
/// use metering::calendar;
/// use time::macros::date;
///
/// assert_eq!(calendar::day_length(date!(2026 - 03 - 29)).whole_hours(), 23);
/// assert_eq!(calendar::day_length(date!(2026 - 07 - 20)).whole_hours(), 24);
/// assert_eq!(calendar::day_length(date!(2026 - 10 - 25)).whole_hours(), 25);
/// ```
#[must_use]
pub fn day_length(day: Date) -> Duration {
    day_end_utc(day) - day_start_utc(day)
}

/// How long the Berlin calendar month containing `day` lasts.
///
/// Never a whole number of days in a month holding a DST transition: March is
/// 31 days minus one hour, October 31 days plus one hour.
#[must_use]
pub fn month_length(day: Date) -> Duration {
    month_end_utc(day) - month_start_utc(day)
}

/// How long a Berlin calendar year lasts (365 or 366 days, ±0 across DST since
/// both transitions fall inside the year and cancel out).
#[must_use]
pub fn year_length(year: i32) -> Duration {
    year_end_utc(year) - year_start_utc(year)
}

// ── DST classification ────────────────────────────────────────────────────────

/// What kind of Berlin calendar day this is, with respect to DST.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "SCREAMING_SNAKE_CASE"))]
pub enum DayKind {
    /// An ordinary 24-hour day.
    Normal,
    /// Spring forward — the day is 23 hours long (CET → CEST, last Sunday in March).
    ShortDay,
    /// Fall back — the day is 25 hours long (CEST → CET, last Sunday in October).
    LongDay,
}

impl DayKind {
    /// `true` for the two DST transition days.
    #[must_use]
    pub fn is_dst_transition(self) -> bool {
        !matches!(self, Self::Normal)
    }
}

/// Classify a Berlin calendar day by its length.
///
/// Any day that is neither 23, 24 nor 25 hours long (possible only for
/// historical offset changes before 1980) reports as [`DayKind::Normal`]; use
/// [`day_length`] when the exact duration matters.
#[must_use]
pub fn day_kind(day: Date) -> DayKind {
    match day_length(day).whole_hours() {
        23 => DayKind::ShortDay,
        25 => DayKind::LongDay,
        _ => DayKind::Normal,
    }
}

// ── expected interval counts ──────────────────────────────────────────────────

/// Number of intervals of `resolution` in a Berlin calendar day.
///
/// 96 quarter-hours on an ordinary day, **92** on the spring-forward day and
/// **100** on the fall-back day. `None` when the resolution does not divide the
/// day evenly, or is coarser than a day.
///
/// This is the number a completeness or coverage check must compare against.
#[must_use]
pub fn intervals_in_day(day: Date, resolution: IntervalResolution) -> Option<u32> {
    match resolution {
        IntervalResolution::Day => Some(1),
        IntervalResolution::Month | IntervalResolution::Year => None,
        other => divide(day_length(day), other),
    }
}

/// Number of intervals of `resolution` in the Berlin calendar month containing `day`.
///
/// Accounts for both the month's day count and any DST transition inside it:
/// March 2026 holds 2 972 quarter-hours, October 2026 holds 2 980.
#[must_use]
pub fn intervals_in_month(day: Date, resolution: IntervalResolution) -> Option<u32> {
    match resolution {
        IntervalResolution::Day => Some(u32::from(days_in_month(day))),
        IntervalResolution::Month => Some(1),
        IntervalResolution::Year => None,
        other => divide(month_length(day), other),
    }
}

/// Number of intervals of `resolution` in a Berlin calendar year.
#[must_use]
pub fn intervals_in_year(year: i32, resolution: IntervalResolution) -> Option<u32> {
    match resolution {
        IntervalResolution::Day => Some(u32::from(days_in_year(year))),
        IntervalResolution::Month => Some(12),
        IntervalResolution::Year => Some(1),
        other => divide(year_length(year), other),
    }
}

/// Number of intervals of `resolution` between two UTC instants.
///
/// Only defined for resolutions with a fixed length
/// ([`IntervalResolution::fixed_seconds`]) — a count of calendar days, months or
/// years is not a division, so use [`intervals_in_month`] and friends for those.
/// `None` when `to < from`, when the span is not a whole multiple of the
/// resolution, or when the resolution has no fixed length.
#[must_use]
pub fn intervals_between(
    from: OffsetDateTime,
    to: OffsetDateTime,
    resolution: IntervalResolution,
) -> Option<u32> {
    if to < from {
        return None;
    }
    divide(to - from, resolution)
}

/// Whole Berlin calendar days between two instants.
///
/// **Not** `(to - from).whole_days()`, which truncates: fourteen calendar days
/// spanning the spring-forward transition are 335 hours, and integer division
/// reports thirteen. A daily average computed on that count is 7.7 % too high,
/// and so is any annual projection built on it.
///
/// ```rust
/// use metering::calendar;
/// use time::macros::date;
///
/// let from = calendar::day_start_utc(date!(2026 - 03 - 23));
/// let to = calendar::day_start_utc(date!(2026 - 04 - 06));
/// assert_eq!((to - from).whole_days(), 13); // the naive count, one short
/// assert_eq!(calendar::days_between(from, to), 14); // the calendar count
/// ```
#[must_use]
pub fn days_between(from: OffsetDateTime, to: OffsetDateTime) -> i64 {
    (local_day(to) - local_day(from)).whole_days()
}

/// The same Berlin wall-clock time one year earlier.
///
/// Used to align an observation window with the matching prior-year period.
/// Subtracting a fixed 365 days drifts by a day across a leap year and lands on
/// a different clock time across a DST transition; this keeps the local date and
/// time of day, so "the first two weeks of March" maps to the first two weeks of
/// March. **29 February maps to 28 February**, the convention German billing
/// practice uses for anniversary dates.
#[must_use]
pub fn shift_back_one_year(instant: OffsetDateTime) -> OffsetDateTime {
    let local = to_berlin(instant);
    let date = local.date();
    let target_year = date.year() - 1;
    let day = date
        .day()
        .min(time::util::days_in_month(date.month(), target_year));
    let Ok(shifted) = Date::from_calendar_date(target_year, date.month(), day) else {
        return instant;
    };
    let naive = PrimitiveDateTime::new(shifted, local.time());
    match naive.assume_timezone(berlin()) {
        OffsetResult::Some(t) => t,
        OffsetResult::Ambiguous(first, _) => first,
        OffsetResult::None => naive.assume_timezone_utc(berlin()),
    }
    .to_offset(time::UtcOffset::UTC)
}

/// Days in the Berlin calendar month containing `day` (28–31).
#[must_use]
pub fn days_in_month(day: Date) -> u8 {
    time::util::days_in_month(day.month(), day.year())
}

/// Days in a calendar year: 365, or 366 in a leap year.
#[must_use]
pub fn days_in_year(year: i32) -> u16 {
    time::util::days_in_year(year)
}

/// `true` when `year` is a Gregorian leap year.
#[must_use]
pub fn is_leap_year(year: i32) -> bool {
    time::util::is_leap_year(year)
}

// ── helpers ───────────────────────────────────────────────────────────────────

/// Resolve 00:00 Berlin local on `day` to the instant it occurs at.
///
/// Europe/Berlin has not moved its clocks at midnight since the tz database
/// began, so the ordinary branch is the only one taken in practice. The other
/// two are still handled rather than unwrapped: an ambiguous midnight resolves
/// to the **earlier** instant, so consecutive days tile without overlap, and a
/// skipped midnight resolves to the instant the clock jumps, so no day is empty.
fn local_midnight_utc(day: Date) -> OffsetDateTime {
    let naive = PrimitiveDateTime::new(day, Time::MIDNIGHT);
    match naive.assume_timezone(berlin()) {
        OffsetResult::Some(t) => t,
        OffsetResult::Ambiguous(first, _) => first,
        OffsetResult::None => naive.assume_timezone_utc(berlin()),
    }
}

/// `span / resolution`, but only for resolutions with a fixed second count and
/// only when the division is exact.
fn divide(span: Duration, resolution: IntervalResolution) -> Option<u32> {
    let secs = resolution.fixed_seconds()?;
    let span_secs = span.whole_seconds();
    if span_secs < 0 {
        return None;
    }
    // `fixed_seconds` never yields 0, so the modulo below cannot trap.
    let secs = i64::from(secs);
    if span_secs % secs != 0 {
        return None;
    }
    u32::try_from(span_secs / secs).ok()
}

fn first_of_month(day: Date) -> Date {
    Date::from_calendar_date(day.year(), day.month(), 1).unwrap_or(day)
}

fn first_of_next_month(day: Date) -> Date {
    let (year, month) = if day.month() == Month::December {
        (day.year() + 1, Month::January)
    } else {
        (day.year(), day.month().next())
    };
    Date::from_calendar_date(year, month, 1).unwrap_or(day)
}

fn jan_first(year: i32) -> Date {
    Date::from_calendar_date(year, Month::January, 1).unwrap_or(Date::MIN)
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::{date, datetime};

    /// The transition days, from the IANA tz database. These are the numbers a
    /// completeness check has to know: 92 in spring, 100 in autumn.
    #[test]
    fn dst_transition_days_are_23_and_25_hours() {
        let cases = [
            (date!(2025 - 03 - 30), 23, 92, DayKind::ShortDay),
            (date!(2025 - 10 - 26), 25, 100, DayKind::LongDay),
            (date!(2026 - 03 - 29), 23, 92, DayKind::ShortDay),
            (date!(2026 - 10 - 25), 25, 100, DayKind::LongDay),
            (date!(2027 - 03 - 28), 23, 92, DayKind::ShortDay),
            (date!(2027 - 10 - 31), 25, 100, DayKind::LongDay),
            (date!(2026 - 07 - 20), 24, 96, DayKind::Normal),
            (date!(2026 - 01 - 15), 24, 96, DayKind::Normal),
        ];
        for (day, hours, quarters, kind) in cases {
            assert_eq!(day_length(day).whole_hours(), hours, "{day} length");
            assert_eq!(
                intervals_in_day(day, IntervalResolution::QuarterHour),
                Some(quarters),
                "{day} quarter-hours"
            );
            assert_eq!(day_kind(day), kind, "{day} kind");
        }
    }

    /// March is short an hour and October long one, so neither month is a whole
    /// number of 24-hour days.
    #[test]
    fn dst_months_are_not_whole_days() {
        let march = intervals_in_month(date!(2026 - 03 - 01), IntervalResolution::QuarterHour);
        assert_eq!(
            march,
            Some(2_972),
            "31 × 96 = 2976, minus 4 for the lost hour"
        );

        let october = intervals_in_month(date!(2026 - 10 - 01), IntervalResolution::QuarterHour);
        assert_eq!(
            october,
            Some(2_980),
            "31 × 96 = 2976, plus 4 for the extra hour"
        );

        // A month with no transition is exactly days × 96.
        let july = intervals_in_month(date!(2026 - 07 - 01), IntervalResolution::QuarterHour);
        assert_eq!(july, Some(31 * 96));

        // February 2028 is a leap February.
        let feb = intervals_in_month(date!(2028 - 02 - 01), IntervalResolution::QuarterHour);
        assert_eq!(feb, Some(29 * 96));
    }

    /// The two transitions cancel, so a year is exactly its day count — but the
    /// day count itself is not always 365.
    #[test]
    fn years_account_for_leap_days() {
        assert_eq!(days_in_year(2026), 365);
        assert_eq!(days_in_year(2028), 366);
        assert!(!is_leap_year(2026));
        assert!(is_leap_year(2028));

        assert_eq!(
            intervals_in_year(2026, IntervalResolution::QuarterHour),
            Some(365 * 96)
        );
        assert_eq!(
            intervals_in_year(2028, IntervalResolution::QuarterHour),
            Some(366 * 96)
        );
        assert_eq!(intervals_in_year(2026, IntervalResolution::Day), Some(365));
        assert_eq!(intervals_in_year(2026, IntervalResolution::Month), Some(12));
    }

    /// The whole point: a German day does not start at 00:00 UTC.
    #[test]
    fn day_boundaries_are_local_not_utc() {
        // Winter — CET, UTC+1.
        assert_eq!(
            day_start_utc(date!(2026 - 01 - 15)),
            datetime!(2026-01-14 23:00 UTC)
        );
        // Summer — CEST, UTC+2.
        assert_eq!(
            day_start_utc(date!(2026 - 07 - 15)),
            datetime!(2026-07-14 22:00 UTC)
        );
        // The last instant of the UTC day before is already the next German day.
        assert_eq!(
            local_day(datetime!(2026-07-14 23:30 UTC)),
            date!(2026 - 07 - 15)
        );
        assert_eq!(
            local_day(datetime!(2026-01-14 22:59 UTC)),
            date!(2026 - 01 - 14)
        );
    }

    /// Consecutive days must tile the timeline exactly, including across both
    /// transitions — no gap, no overlap, no lost quarter-hour.
    #[test]
    fn days_tile_the_year_without_gaps() {
        let mut day = date!(2026 - 01 - 01);
        let mut cursor = day_start_utc(day);
        let mut quarters = 0u32;
        while day.year() == 2026 {
            assert_eq!(
                day_start_utc(day),
                cursor,
                "{day} must start where the previous ended"
            );
            quarters += intervals_in_day(day, IntervalResolution::QuarterHour)
                .unwrap_or_else(|| panic!("{day} must divide into quarter-hours"));
            cursor = day_end_utc(day);
            day = day.next_day().unwrap();
        }
        assert_eq!(cursor, year_end_utc(2026));
        assert_eq!(
            quarters,
            365 * 96,
            "the transitions cancel over a full year"
        );
    }

    #[test]
    fn month_and_year_boundaries_are_local() {
        assert_eq!(
            month_start_utc(date!(2026 - 03 - 17)),
            datetime!(2026-02-28 23:00 UTC),
            "March starts at 00:00 CET"
        );
        assert_eq!(
            month_end_utc(date!(2026 - 03 - 17)),
            datetime!(2026-03-31 22:00 UTC),
            "April starts at 00:00 CEST"
        );
        assert_eq!(year_start_utc(2026), datetime!(2025-12-31 23:00 UTC));
        assert_eq!(
            local_month(datetime!(2026-02-28 23:30 UTC)),
            date!(2026 - 03 - 01),
            "23:30 UTC on 28 Feb is already 1 March in Berlin"
        );
        assert_eq!(local_year(datetime!(2025-12-31 23:30 UTC)), 2026);
    }

    #[test]
    fn coarse_resolutions_have_no_sub_day_count() {
        let day = date!(2026 - 06 - 01);
        assert_eq!(intervals_in_day(day, IntervalResolution::Day), Some(1));
        assert_eq!(intervals_in_day(day, IntervalResolution::Month), None);
        assert_eq!(intervals_in_day(day, IntervalResolution::Year), None);
        assert_eq!(intervals_in_month(day, IntervalResolution::Day), Some(30));
        assert_eq!(intervals_in_month(day, IntervalResolution::Year), None);
    }

    #[test]
    fn uneven_resolutions_do_not_divide() {
        // 3 600 s divides a 24 h day but not a 23 h one? It does — 23 hours is a
        // whole number of hours. A 7-minute interval divides neither.
        assert_eq!(
            intervals_in_day(date!(2026 - 03 - 29), IntervalResolution::Hour),
            Some(23)
        );
        assert_eq!(
            intervals_in_day(date!(2026 - 06 - 01), IntervalResolution::Custom(420)),
            None,
            "420 s does not divide 86 400 s"
        );
        assert_eq!(
            intervals_in_day(date!(2026 - 06 - 01), IntervalResolution::Custom(0)),
            None
        );
    }

    #[test]
    fn intervals_between_is_fixed_resolution_only() {
        let from = datetime!(2026-01-01 00:00 UTC);
        let to = datetime!(2026-01-01 06:00 UTC);
        assert_eq!(
            intervals_between(from, to, IntervalResolution::QuarterHour),
            Some(24)
        );
        assert_eq!(intervals_between(to, from, IntervalResolution::Hour), None);
        assert_eq!(intervals_between(from, to, IntervalResolution::Month), None);
    }

    /// Counting days by dividing a duration loses one across the spring
    /// transition — the bug that inflated the Jahresprognose by 7.7 %.
    #[test]
    fn days_between_counts_calendar_days_not_durations() {
        let from = day_start_utc(date!(2026 - 03 - 23));
        let to = day_start_utc(date!(2026 - 04 - 06));
        assert_eq!((to - from).whole_days(), 13, "the naive count truncates");
        assert_eq!(days_between(from, to), 14, "fourteen calendar days");

        // The autumn transition rounds the other way but is equally wrong.
        let autumn_from = day_start_utc(date!(2026 - 10 - 19));
        let autumn_to = day_start_utc(date!(2026 - 11 - 02));
        assert_eq!(days_between(autumn_from, autumn_to), 14);

        // Ordinary spans are unaffected, and the count is signed.
        assert_eq!(
            days_between(
                datetime!(2026-06-01 05:00 UTC),
                datetime!(2026-06-08 23:00 UTC)
            ),
            8,
            "23:00 UTC on the 8th is already the 9th in Berlin"
        );
        assert_eq!(days_between(to, from), -14);
    }

    #[test]
    fn shifting_back_a_year_keeps_the_local_clock_time() {
        // Midsummer: same local date and time, one year earlier.
        assert_eq!(
            shift_back_one_year(datetime!(2026-07-15 10:00 UTC)), // 12:00 CEST
            datetime!(2025-07-15 10:00 UTC)                       // 12:00 CEST
        );

        // Across a winter/summer boundary the UTC instant moves by an hour so
        // the *local* time is preserved — which is what an anniversary means.
        let winter = shift_back_one_year(datetime!(2026-01-15 23:30 UTC)); // 00:30 on the 16th
        assert_eq!(local_day(winter), date!(2025 - 01 - 16));
        assert_eq!(
            to_berlin(winter).time(),
            to_berlin(datetime!(2026-01-15 23:30 UTC)).time()
        );

        // 29 February has no counterpart; it maps to the 28th.
        let leap_day = day_start_utc(date!(2028 - 02 - 29));
        assert_eq!(
            local_day(shift_back_one_year(leap_day)),
            date!(2027 - 02 - 28)
        );

        // A fixed 365-day subtraction drifts whenever the intervening year
        // contains a leap day: 2028-01-15 → 2029-01-15 spans 29 February 2028,
        // so it is 366 days long and stepping back 365 overshoots by one.
        let after_leap = day_start_utc(date!(2029 - 01 - 15));
        assert_eq!(
            local_day(shift_back_one_year(after_leap)),
            date!(2028 - 01 - 15),
            "the calendar shift lands on the anniversary"
        );
        assert_eq!(
            local_day(after_leap - Duration::days(365)),
            date!(2028 - 01 - 16),
            "the naive shift drifts by the leap day"
        );
    }

    /// The tz database, not a hard-coded EU rule: Germany's 1980 transition was
    /// on 6 April, not the last Sunday in March.
    #[test]
    fn historical_transitions_come_from_the_tz_database() {
        assert_eq!(day_length(date!(1980 - 04 - 06)).whole_hours(), 23);
        assert_eq!(day_length(date!(1980 - 03 - 30)).whole_hours(), 24);
    }
}
