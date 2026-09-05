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
//! Everything here is Europe/Berlin, which is the market area this crate
//! models. EDI@Energy *Allgemeine Festlegungen* v6.1c, Kap. 3 states the split
//! this module exists to manage: *"Die Angabe von Zeiten in einer EDIFACT
//! Nachricht erfolgt in koordinierter Weltzeit (Coordinated Universal Time,
//! UTC). In Deutschland gilt die Mitteleuropäische Zeit (MEZ) bzw. die
//! Mitteleuropäische Sommerzeit (MESZ) als gesetzliche deutsche Zeit. Alle in
//! den Prozessen genannten Zeitpunkte … nutzen die gesetzliche deutsche
//! Zeit."* Timestamps on the wire are UTC; periods and deadlines are MEZ/MESZ.
//! The rules come from the IANA tz database via `time-tz`, so historical
//! transitions are correct too — this is not a hard-coded "last Sunday in
//! March" approximation. [`berlin`] exposes the timezone for callers needing
//! something else.
//!
//! Kap. 3.1 goes on to pin the four day-start codings this module produces, and
//! `tests/regulatory_showcase.rs` asserts them against it:
//!
//! | Sparte | MEZ | MESZ |
//! |---|---|---|
//! | Strom (00:00 local) | `2300` — 23:00 UTC | `2200` — 22:00 UTC |
//! | Gas (06:00 local) | `0500` — 05:00 UTC | `0400` — 04:00 UTC |
//!
//! ## Leap seconds are not represented
//!
//! Kap. 3.9 permits a second-precision timestamp to name a leap second
//! (`23:59:60`). [`time::OffsetDateTime`] has no such second, so a value that
//! named one could not be constructed at all — it would fail at the parse,
//! where the message is still available to report, rather than silently
//! becoming `23:59:59`. Interval boundaries are quarter-hours and never land
//! there; none has been inserted since 2016, and the CGPM resolved in 2022 to
//! stop inserting them by 2035.
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

use time::{Date, Duration, Month, OffsetDateTime, PrimitiveDateTime, Time, macros::time};
use time_tz::{
    Offset as _, OffsetDateTimeExt as _, OffsetResult, PrimitiveDateTimeExt as _, TimeZone as _,
    Tz, timezones,
};

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

// ── the Gastag ────────────────────────────────────────────────────────────────

/// The Berlin wall-clock time a Gastag begins at: 06:00.
const GAS_DAY_START: Time = time!(6:00);

/// Which daily boundary a period is cut on.
///
/// German electricity settles on the calendar day and German gas on the
/// **Gastag**, which runs 06:00 to 06:00 local (GaBi Gas, following Art. 3
/// Nr. 6 VO (EU) 312/2014). Both are "one day"; they simply start six hours
/// apart, so the distinction is a *boundary*, not a length — which is why it
/// lives here and not in [`IntervalResolution`], whose canonical string is an
/// ISO 8601 duration and has nothing to say about phase.
///
/// It carries all the way up to the month, and that is the market's own rule,
/// not an extrapolation. EDI@Energy *Allgemeine Festlegungen* v6.1c, Kap. 3.1:
/// *"Die Angabe des Bilanzierungsmonats erfolgt unter Angabe von Jahr und
/// Monat (z. B. Juni 2021), sodass damit der Zeitraum vom 01.06.2021 00:00 Uhr
/// bis 01.07.2021 00:00 Uhr gesetzlicher deutscher Zeit abgedeckt ist, wenn es
/// sich um den Bilanzierungsmonat in der Sparte Strom handelt, in der Sparte
/// Gas ist damit der Zeitraum vom 01.06.2021 06:00 Uhr bis 01.07.2021 06:00
/// Uhr gesetzlicher deutscher Zeit abgedeckt."*
///
/// Pass it to [`crate::ResampleConfig::on`] or
/// [`crate::FillGapsConfig::on`] to move a daily, monthly or yearly grid onto
/// gas days without touching anything else.
///
/// ```rust
/// use metering::calendar::DayBoundary;
/// use time::macros::{date, datetime};
///
/// // The same calendar date, two different windows.
/// assert_eq!(
///     DayBoundary::Midnight.day_start_utc(date!(2026 - 01 - 15)),
///     datetime!(2026-01-14 23:00 UTC),
/// );
/// assert_eq!(
///     DayBoundary::Gastag.day_start_utc(date!(2026 - 01 - 15)),
///     datetime!(2026-01-15 5:00 UTC),
/// );
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "SCREAMING_SNAKE_CASE"))]
pub enum DayBoundary {
    /// 00:00 Europe/Berlin — the Liefertag of the electricity market.
    #[default]
    Midnight,
    /// 06:00 Europe/Berlin — the Gastag of the gas market.
    Gastag,
}

impl DayBoundary {
    /// Every variant, in declaration order.
    pub const ALL: [Self; 2] = [Self::Midnight, Self::Gastag];

    /// Stable DB/wire label. Matches the `serde` tag and
    /// [`FromStr`](std::str::FromStr) input.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Midnight => "MIDNIGHT",
            Self::Gastag => "GASTAG",
        }
    }

    /// The UTC instant the day named `day` begins at.
    #[must_use]
    pub fn day_start_utc(self, day: Date) -> OffsetDateTime {
        match self {
            Self::Midnight => day_start_utc(day),
            Self::Gastag => gas_day_start_utc(day),
        }
    }

    /// The UTC instant the day named `day` ends at (exclusive).
    #[must_use]
    pub fn day_end_utc(self, day: Date) -> OffsetDateTime {
        match self {
            Self::Midnight => day_end_utc(day),
            Self::Gastag => gas_day_end_utc(day),
        }
    }

    /// The day an instant belongs to under this boundary.
    #[must_use]
    pub fn local_day(self, instant: OffsetDateTime) -> Date {
        match self {
            Self::Midnight => local_day(instant),
            Self::Gastag => local_gas_day(instant),
        }
    }

    /// How long that day lasts: 23 h, 24 h or 25 h.
    ///
    /// The transition falls in a **different** named day for the two
    /// boundaries: the clocks move at 02:00/03:00 local, before 06:00, so the
    /// long or short Gastag is the one named after the Saturday.
    #[must_use]
    pub fn day_length(self, day: Date) -> Duration {
        self.day_end_utc(day) - self.day_start_utc(day)
    }

    /// Number of intervals of `resolution` in that day, or `None` when the
    /// resolution does not divide it evenly or is coarser than a day.
    #[must_use]
    pub fn intervals_in_day(self, day: Date, resolution: IntervalResolution) -> Option<u32> {
        match resolution {
            IntervalResolution::Day => Some(1),
            IntervalResolution::Month | IntervalResolution::Year => None,
            other => divide(self.day_length(day), other),
        }
    }

    /// The month containing `day`, as its first day.
    ///
    /// A gas month runs from 06:00 on the first to 06:00 on the first of the
    /// next — the boundary carries all the way up, so a monthly gas total is
    /// not a calendar-month total shifted, it is a whole number of Gastage.
    #[must_use]
    pub fn local_month(self, instant: OffsetDateTime) -> Date {
        first_of_month(self.local_day(instant))
    }

    /// The UTC instant the month containing `day` begins at.
    #[must_use]
    pub fn month_start_utc(self, day: Date) -> OffsetDateTime {
        self.day_start_utc(first_of_month(day))
    }

    /// The UTC instant the month containing `day` ends at (exclusive).
    #[must_use]
    pub fn month_end_utc(self, day: Date) -> OffsetDateTime {
        self.day_start_utc(first_of_next_month(day))
    }

    /// The year an instant falls in.
    #[must_use]
    pub fn local_year(self, instant: OffsetDateTime) -> i32 {
        self.local_day(instant).year()
    }

    /// The UTC instant a year begins at.
    #[must_use]
    pub fn year_start_utc(self, year: i32) -> OffsetDateTime {
        self.day_start_utc(jan_first(year))
    }

    /// The UTC instant a year ends at (exclusive).
    #[must_use]
    pub fn year_end_utc(self, year: i32) -> OffsetDateTime {
        self.day_start_utc(jan_first(year + 1))
    }

    /// The day `day` as a half-open UTC range `[start, end)`.
    ///
    /// The same two instants [`day_start_utc`](Self::day_start_utc) and
    /// [`day_end_utc`](Self::day_end_utc) return, as the pair every period
    /// consumer here takes — [`AggregationConfig::over_period`](crate::AggregationConfig::over_period), a validation
    /// window, a resample bound.
    #[must_use]
    pub fn day_range_utc(self, day: Date) -> (OffsetDateTime, OffsetDateTime) {
        (self.day_start_utc(day), self.day_end_utc(day))
    }

    /// The month containing `day` as a half-open UTC range `[start, end)`.
    #[must_use]
    pub fn month_range_utc(self, day: Date) -> (OffsetDateTime, OffsetDateTime) {
        (self.month_start_utc(day), self.month_end_utc(day))
    }

    /// The year `year` as a half-open UTC range `[start, end)`.
    #[must_use]
    pub fn year_range_utc(self, year: i32) -> (OffsetDateTime, OffsetDateTime) {
        (self.year_start_utc(year), self.year_end_utc(year))
    }

    /// The **Bilanzierungsmonat** named by a year and a month, as a half-open
    /// UTC range.
    ///
    /// The settlement month of MaBiS and GaBi, addressed the way the market
    /// addresses it — *"Juni 2021"* — rather than by a date the caller has to
    /// construct first. Both boundaries are the same passage of EDI@Energy
    /// *Allgemeine Festlegungen* v6.1c Kap. 3.1, quoted on this type: Strom
    /// runs 00:00 to 00:00, Gas 06:00 to 06:00, and neither is a fixed number
    /// of hours because the month may contain a DST transition.
    ///
    /// ```rust
    /// use metering::calendar::DayBoundary;
    /// use time::Month;
    /// use time::macros::datetime;
    ///
    /// // March 2026 contains the spring-forward Sunday, so the electricity
    /// // Bilanzierungsmonat is one hour short of 31 days.
    /// let (from, to) = DayBoundary::Midnight.bilanzierungsmonat(2026, Month::March);
    /// assert_eq!(from, datetime!(2026-02-28 23:00 UTC));
    /// assert_eq!(to, datetime!(2026-03-31 22:00 UTC));
    /// assert_eq!((to - from).whole_hours(), 31 * 24 - 1);
    ///
    /// // The gas month is the same span, shifted six hours.
    /// let (gas_from, gas_to) = DayBoundary::Gastag.bilanzierungsmonat(2026, Month::March);
    /// assert_eq!(gas_from, datetime!(2026-03-01 5:00 UTC));
    /// assert_eq!(gas_to, datetime!(2026-04-01 4:00 UTC));
    /// ```
    #[must_use]
    pub fn bilanzierungsmonat(self, year: i32, month: Month) -> (OffsetDateTime, OffsetDateTime) {
        let first = first_of(year, month);
        (self.day_start_utc(first), self.month_end_utc(first))
    }

    /// The half-open bucket `[start, end)` of `resolution` that contains `ts`.
    ///
    /// Day, month and year buckets are Europe/Berlin calendar periods cut on
    /// this boundary, so their duration is whatever the calendar says — 23, 24
    /// or 25 hours for a day; 28 to 31 days ±1 hour for a month. Sub-daily
    /// buckets snap in UTC, which coincides with local snapping because every
    /// Berlin offset is a whole number of hours.
    ///
    /// One implementation, shared by [`crate::resample()`] and
    /// [`crate::session::split_session`]: the grid a series is aggregated onto
    /// and the grid a session is distributed across have to be the same grid,
    /// or energy moves between slots on the way through.
    ///
    /// ```rust
    /// use metering::calendar::DayBoundary;
    /// use metering::IntervalResolution;
    /// use time::macros::datetime;
    ///
    /// let (from, to) = DayBoundary::Midnight
    ///     .bucket_bounds(datetime!(2026-06-01 12:07 UTC), IntervalResolution::QuarterHour);
    /// assert_eq!(from, datetime!(2026-06-01 12:00 UTC));
    /// assert_eq!(to, datetime!(2026-06-01 12:15 UTC));
    /// ```
    #[must_use]
    pub fn bucket_bounds(
        self,
        ts: OffsetDateTime,
        resolution: IntervalResolution,
    ) -> (OffsetDateTime, OffsetDateTime) {
        match resolution {
            IntervalResolution::Day => self.day_range_utc(self.local_day(ts)),
            IntervalResolution::Month => self.month_range_utc(self.local_day(ts)),
            IntervalResolution::Year => self.year_range_utc(self.local_year(ts)),
            // Fixed-length resolutions: snap the Unix timestamp onto the grid.
            // `fixed_seconds` answers `Some` for every one of them — a
            // `CustomSeconds` cannot be zero — so the fallback is unreachable
            // and exists only so this function is total without an `expect`.
            fixed => {
                let Some(secs) = fixed.fixed_seconds() else {
                    return (ts, ts + Duration::seconds(1));
                };
                let s = i64::from(secs);
                let snapped = ts.unix_timestamp().div_euclid(s) * s;
                let start = OffsetDateTime::from_unix_timestamp(snapped).unwrap_or(ts);
                (start, start + Duration::seconds(s))
            }
        }
    }
}

/// The UTC instant at which the **Gastag** `day` begins — 06:00 Europe/Berlin.
///
/// The German gas market balances on gas days, not calendar days: a Gastag
/// runs from 06:00 local time to 06:00 the next morning (GaBi Gas, following
/// the EU-wide convention of Art. 3 Nr. 6 VO (EU) 312/2014; the BDEW/VKU/GEODE
/// Leitfaden *SLP Gas* forms its daily mean temperatures over the same span).
/// Summing a gas Lastgang over the *calendar* day books the 00:00–06:00 draw
/// into the wrong Bilanzierungstag — six hours, every day.
///
/// Berlin has never moved its clocks at 06:00, so this instant is always
/// unambiguous; a Gastag containing a DST transition is 23 or 25 hours long,
/// exactly as a calendar day is. Note **which** Gastag that is: the clocks
/// change at 02:00/03:00 local, which lies before 06:00 — so the long or
/// short Gastag is the one *named after the Saturday*, not the transition
/// Sunday. The SLP-Gas Leitfaden calls this out: *"Daher ist die Zeitumstellung
/// in den Werten für den Samstag vor der Umstellung zu berücksichtigen."*
///
/// ```rust
/// use metering::calendar;
/// use time::macros::{date, datetime};
///
/// // Winter: 06:00 CET is 05:00 UTC.
/// assert_eq!(calendar::gas_day_start_utc(date!(2026 - 01 - 15)), datetime!(2026-01-15 5:00 UTC));
/// // The 25-hour Gastag is Saturday's — the transition happens at 03:00
/// // local on Sunday, inside the gas day that began Saturday 06:00.
/// let saturday = calendar::gas_day_end_utc(date!(2026 - 10 - 24))
///     - calendar::gas_day_start_utc(date!(2026 - 10 - 24));
/// assert_eq!(saturday.whole_hours(), 25);
/// let sunday = calendar::gas_day_end_utc(date!(2026 - 10 - 25))
///     - calendar::gas_day_start_utc(date!(2026 - 10 - 25));
/// assert_eq!(sunday.whole_hours(), 24);
/// ```
#[must_use]
pub fn gas_day_start_utc(day: Date) -> OffsetDateTime {
    resolve_local(PrimitiveDateTime::new(day, GAS_DAY_START)).to_offset(time::UtcOffset::UTC)
}

/// The UTC instant at which the Gastag `day` ends (exclusive) — 06:00
/// Europe/Berlin on the following calendar day.
#[must_use]
pub fn gas_day_end_utc(day: Date) -> OffsetDateTime {
    gas_day_start_utc(day.next_day().unwrap_or(day))
}

/// The Gastag an instant belongs to.
///
/// An instant before 06:00 Berlin local time still belongs to the *previous*
/// Gastag — the counterpart of [`local_day`] for gas balancing.
///
/// ```rust
/// use metering::calendar;
/// use time::macros::{date, datetime};
///
/// // 04:30 UTC on 15 July is 06:30 local — already the Gastag of the 15th...
/// assert_eq!(calendar::local_gas_day(datetime!(2026-07-15 4:30 UTC)), date!(2026 - 07 - 15));
/// // ...but 03:30 UTC (05:30 local) is still the Gastag of the 14th.
/// assert_eq!(calendar::local_gas_day(datetime!(2026-07-15 3:30 UTC)), date!(2026 - 07 - 14));
/// ```
#[must_use]
pub fn local_gas_day(instant: OffsetDateTime) -> Date {
    let local = to_berlin(instant);
    let day = local.date();
    if local.time() < GAS_DAY_START {
        day.previous_day().unwrap_or(day)
    } else {
        day
    }
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
    /// Every kind, in declaration order.
    pub const ALL: [Self; 3] = [Self::Normal, Self::ShortDay, Self::LongDay];

    /// Stable DB/wire label. Matches the `serde` tag and
    /// [`FromStr`](std::str::FromStr) input.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Normal => "NORMAL",
            Self::ShortDay => "SHORT_DAY",
            Self::LongDay => "LONG_DAY",
        }
    }

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

/// The instant the UTC offset changes on a Berlin calendar day.
///
/// `None` on an ordinary day. On a transition day this is the moment the clock
/// jumps: 01:00 UTC on both the spring and autumn Sundays under the current EU
/// rule, though the tz database is what actually decides.
///
/// It is the anchor for the **repeated hour**. On the fall-back day, local
/// 02:00–03:00 happens twice — once at UTC+2 before this instant and once at
/// UTC+1 after it — so the two passes together occupy
/// `[transition − 1 h, transition + 1 h)` in UTC.
///
/// ```rust
/// use metering::calendar;
/// use time::macros::{date, datetime};
///
/// // Autumn 2026: CEST ends at 01:00 UTC = 03:00 local, which becomes 02:00.
/// assert_eq!(
///     calendar::dst_transition_utc(date!(2026 - 10 - 25)),
///     Some(datetime!(2026-10-25 1:00 UTC)),
/// );
/// assert_eq!(calendar::dst_transition_utc(date!(2026 - 07 - 20)), None);
/// ```
#[must_use]
pub fn dst_transition_utc(day: Date) -> Option<OffsetDateTime> {
    let start = day_start_utc(day);
    let end = day_end_utc(day);
    let first_offset = to_berlin(start).offset();
    if to_berlin(end - Duration::seconds(1)).offset() == first_offset {
        return None;
    }
    // Every transition in the tz database for this zone lands on a whole hour,
    // so stepping hours finds it exactly rather than approximately.
    let mut cursor = start;
    while cursor < end {
        let next = cursor + Duration::hours(1);
        if to_berlin(next).offset() != to_berlin(cursor).offset() {
            return Some(next);
        }
        cursor = next;
    }
    None
}

crate::codes::string_codes! {
    DayKind;
    DayBoundary;
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
    resolve_local(PrimitiveDateTime::new(shifted, local.time())).to_offset(time::UtcOffset::UTC)
}

/// The same Berlin wall-clock time `days` calendar days earlier.
///
/// The day-granular counterpart of [`shift_back_one_year`], and for the same
/// reason: subtracting `Duration::days(n)` is a fixed `n × 24` hours, which
/// lands one hour off whenever a DST transition lies in between. Concretely,
/// "one week earlier, same local time" is 167 UTC hours across the
/// spring-forward and **169 across the fall-back** — the failure that made
/// [`crate::substitute`]'s Vergleichstag window exclude the matching slot for
/// a week every October.
///
/// An ambiguous local time (the repeated autumn hour) resolves to the earlier
/// instant; a skipped one (the spring hour) is pushed forward by the gap, so
/// 02:30 on the transition Sunday becomes 03:30 — the convention `java.time`,
/// `chrono` and Python's `zoneinfo` share.
///
/// ```rust
/// use metering::calendar;
/// use time::macros::datetime;
///
/// // Wed 2026-10-28 12:00 CET, one week back: Wed 2026-10-21 12:00 CEST —
/// // 169 hours in UTC, because the 25-hour day lies between.
/// let back = calendar::shift_back_days(datetime!(2026-10-28 11:00 UTC), 7);
/// assert_eq!(back, datetime!(2026-10-21 10:00 UTC));
/// assert_eq!((datetime!(2026-10-28 11:00 UTC) - back).whole_hours(), 169);
/// ```
#[must_use]
pub fn shift_back_days(instant: OffsetDateTime, days: i64) -> OffsetDateTime {
    let local = to_berlin(instant);
    let Some(date) = local.date().checked_sub(Duration::days(days)) else {
        return instant;
    };
    resolve_local(PrimitiveDateTime::new(date, local.time())).to_offset(time::UtcOffset::UTC)
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

/// Resolve a Berlin wall-clock date and time to the instant it occurs at.
///
/// The one place this crate turns a local time into an instant, so the two
/// awkward cases are decided once:
///
/// - **Ambiguous** — the autumn hour that happens twice — resolves to the
///   **earlier** instant. Consecutive periods then tile without overlap.
/// - **Skipped** — the spring hour that never happens — is **pushed forward by
///   the length of the gap**, so 02:30 on the transition Sunday becomes 03:30.
///   That is the convention `java.time`, `chrono` and Python's `zoneinfo` all
///   use, and it keeps the map from local times to instants monotonic. The
///   previous fallback reinterpreted the naive time as UTC, which landed
///   *before* the gap: "02:30, one day earlier" came back as 01:30 local.
///
/// Neither case arises at 00:00 or 06:00 — Europe/Berlin has never moved its
/// clocks at either — so the ordinary branch is the only one taken by
/// [`day_start_utc`] and [`gas_day_start_utc`]. [`shift_back_days`] and
/// [`shift_back_one_year`] can land anywhere, and do reach them.
fn resolve_local(naive: PrimitiveDateTime) -> OffsetDateTime {
    match naive.assume_timezone(berlin()) {
        OffsetResult::Some(t) => t,
        OffsetResult::Ambiguous(first, _) => first,
        OffsetResult::None => {
            // Inside a forward transition the local time does not exist. The
            // offset in force *before* it — sampled a day earlier, since the
            // zone transitions at most once a day — maps the naive time onto
            // the first instant at or after the jump, which is the same as
            // adding the gap to the wall clock.
            let before = naive.assume_utc() - Duration::days(1);
            naive.assume_offset(berlin().get_offset_utc(&before).to_utc())
        }
    }
}

/// Resolve 00:00 Berlin local on `day` to the instant it occurs at.
fn local_midnight_utc(day: Date) -> OffsetDateTime {
    resolve_local(PrimitiveDateTime::new(day, Time::MIDNIGHT))
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
    first_of(year, Month::January)
}

/// The first of `month` in `year`.
///
/// Total, like every other date helper here: the only input
/// `Date::from_calendar_date` rejects for a first-of-month is a year outside
/// `time`'s own range, and answering `Date::MIN` there keeps the whole
/// calendar API free of an `Option` nothing in the German market can produce.
fn first_of(year: i32, month: Month) -> Date {
    Date::from_calendar_date(year, month, 1).unwrap_or(Date::MIN)
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
            intervals_in_day(
                date!(2026 - 06 - 01),
                IntervalResolution::from_seconds(420).unwrap()
            ),
            None,
            "420 s does not divide 86 400 s"
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

    /// A calendar day count, not a duration divided by 86 400.
    ///
    /// A German spring day is 82 800 s and an autumn day 90 000 s, so a
    /// division truncates one day away each March and adds one each October.
    /// The Jahresprognose scales an observed quantity by this count, so an
    /// error here is a percentage error on a forecast.
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

    /// The transition instant anchors the repeated hour, so it is pinned on
    /// both transitions and asserted absent on ordinary days.
    #[test]
    fn dst_transitions_are_located_to_the_instant() {
        for (day, expected) in [
            (date!(2026 - 03 - 29), Some(datetime!(2026-03-29 1:00 UTC))),
            (date!(2026 - 10 - 25), Some(datetime!(2026-10-25 1:00 UTC))),
            (date!(2025 - 10 - 26), Some(datetime!(2025-10-26 1:00 UTC))),
            (date!(2026 - 07 - 20), None),
            (date!(2026 - 01 - 15), None),
        ] {
            assert_eq!(dst_transition_utc(day), expected, "{day}");
        }

        // On the fall-back day the hour either side of the transition is the
        // same local wall-clock hour, at two different offsets.
        let t = dst_transition_utc(date!(2026 - 10 - 25)).unwrap();
        let before = to_berlin(t - Duration::minutes(30));
        let after = to_berlin(t + Duration::minutes(30));
        assert_eq!(before.hour(), 2);
        assert_eq!(after.hour(), 2);
        assert_ne!(before.offset(), after.offset());
    }

    /// The tz database, not a hard-coded EU rule: Germany's 1980 transition was
    /// on 6 April, not the last Sunday in March.
    #[test]
    fn historical_transitions_come_from_the_tz_database() {
        assert_eq!(day_length(date!(1980 - 04 - 06)).whole_hours(), 23);
        assert_eq!(day_length(date!(1980 - 03 - 30)).whole_hours(), 24);
    }

    /// "Same local time, n days earlier" is not n × 24 hours across a DST
    /// transition — 167 hours in spring, 169 in autumn.
    #[test]
    fn shifting_back_days_keeps_the_local_clock_time() {
        // Ordinary week: exactly 168 h.
        let plain = datetime!(2026-06-17 10:00 UTC);
        assert_eq!(plain - shift_back_days(plain, 7), Duration::hours(168));

        // Autumn: Wed 12:00 CET back to Wed 12:00 CEST is 169 h.
        let autumn = datetime!(2026-10-28 11:00 UTC); // 12:00 CET
        let back = shift_back_days(autumn, 7);
        assert_eq!(back, datetime!(2026-10-21 10:00 UTC)); // 12:00 CEST
        assert_eq!(autumn - back, Duration::hours(169));
        assert_eq!(to_berlin(back).time(), to_berlin(autumn).time());

        // Spring: 167 h.
        let spring = datetime!(2026-04-01 10:00 UTC); // 12:00 CEST
        assert_eq!(spring - shift_back_days(spring, 7), Duration::hours(167));

        // Zero days is the identity.
        assert_eq!(shift_back_days(plain, 0), plain);
    }

    /// A local time inside the spring gap does not exist. It is pushed
    /// **forward** by the gap — the java.time / chrono / zoneinfo convention.
    /// Reinterpreting it as UTC instead would land at 01:30 local, an hour
    /// *before* the time that was asked for.
    #[test]
    fn a_skipped_local_time_is_pushed_forward_by_the_gap() {
        // Monday 2026-03-30 02:30 CEST, one day back: Sunday 02:30, which the
        // clocks skip. 03:30 CEST is the first instant that exists.
        let back = shift_back_days(datetime!(2026-03-30 0:30 UTC), 1);
        assert_eq!(back, datetime!(2026-03-29 1:30 UTC));
        let local = to_berlin(back);
        assert_eq!(local.date(), date!(2026 - 03 - 29));
        assert_eq!(local.hour(), 3);
        assert_eq!(local.minute(), 30);

        // The same rule through the annual shift: 2027-03-29 02:30 CEST maps
        // to 2026-03-29 02:30, which is the hour that year skips.
        let year = shift_back_one_year(datetime!(2027-03-29 0:30 UTC));
        assert_eq!(year, datetime!(2026-03-29 1:30 UTC));
        assert_eq!(to_berlin(year).hour(), 3, "{year}");

        // ...and the shift never moves backwards past the requested wall clock.
        for minute in [0i64, 15, 30, 45] {
            let src = datetime!(2026-03-30 0:00 UTC) + Duration::minutes(minute);
            let shifted = shift_back_days(src, 1);
            assert!(shifted < src, "the shift must go back a day");
            assert!(
                to_berlin(shifted).hour() >= 2,
                "a skipped hour resolves forward, not back: {shifted}"
            );
        }
    }

    /// The repeated autumn hour resolves to the **earlier** pass, so the map
    /// from local times to instants stays single-valued and consecutive
    /// periods tile.
    #[test]
    fn an_ambiguous_local_time_resolves_to_the_earlier_pass() {
        // Monday 2026-10-26 02:30 CET, one day back: Sunday 02:30, which
        // happens twice — at 00:30 UTC (CEST) and 01:30 UTC (CET).
        let back = shift_back_days(datetime!(2026-10-26 1:30 UTC), 1);
        assert_eq!(back, datetime!(2026-10-25 0:30 UTC));
        assert_eq!(
            to_berlin(back).offset(),
            time::UtcOffset::from_hms(2, 0, 0).unwrap()
        );
    }

    /// The Gastag is a boundary, not a length: `DayBoundary` moves a daily
    /// window six hours without changing anything else.
    #[test]
    fn day_boundaries_cut_the_same_day_two_ways() {
        let day = date!(2026 - 01 - 15);
        assert_eq!(
            DayBoundary::Midnight.day_start_utc(day),
            datetime!(2026-01-14 23:00 UTC)
        );
        assert_eq!(
            DayBoundary::Gastag.day_start_utc(day),
            datetime!(2026-01-15 5:00 UTC)
        );
        assert_eq!(DayBoundary::default(), DayBoundary::Midnight);

        for boundary in DayBoundary::ALL {
            // Consecutive days tile, and the day an instant falls in contains it.
            let mut cursor = boundary.day_start_utc(date!(2026 - 10 - 23));
            let mut d = date!(2026 - 10 - 23);
            while d < date!(2026 - 10 - 28) {
                assert_eq!(boundary.day_start_utc(d), cursor, "{boundary:?} {d}");
                assert_eq!(boundary.local_day(cursor), d, "{boundary:?} {d}");
                cursor = boundary.day_end_utc(d);
                d = d.next_day().unwrap();
            }
        }

        // The long day is named after the Saturday for gas and the Sunday for
        // the calendar — the transition falls at 03:00 local, before 06:00.
        assert_eq!(
            DayBoundary::Midnight
                .day_length(date!(2026 - 10 - 25))
                .whole_hours(),
            25
        );
        assert_eq!(
            DayBoundary::Gastag
                .day_length(date!(2026 - 10 - 24))
                .whole_hours(),
            25
        );
        assert_eq!(
            DayBoundary::Gastag
                .intervals_in_day(date!(2026 - 10 - 24), IntervalResolution::QuarterHour),
            Some(100)
        );
        assert_eq!(
            DayBoundary::Gastag.intervals_in_day(date!(2026 - 10 - 24), IntervalResolution::Day),
            Some(1)
        );
        assert_eq!(
            DayBoundary::Gastag.intervals_in_day(date!(2026 - 10 - 24), IntervalResolution::Year),
            None
        );
    }

    /// The Gastag runs 06:00–06:00 local, tiles the timeline, and the DST
    /// transition lands in **Saturday's** gas day — the clocks move at
    /// 02:00/03:00, before the 06:00 boundary.
    #[test]
    fn gas_days_run_0600_to_0600_and_tile() {
        // Winter and summer boundary instants.
        assert_eq!(
            gas_day_start_utc(date!(2026 - 01 - 15)),
            datetime!(2026-01-15 5:00 UTC)
        );
        assert_eq!(
            gas_day_start_utc(date!(2026 - 07 - 15)),
            datetime!(2026-07-15 4:00 UTC)
        );

        // Consecutive gas days tile without gap or overlap across both
        // transitions.
        let mut day = date!(2026 - 03 - 25);
        let mut cursor = gas_day_start_utc(day);
        while day < date!(2026 - 04 - 02) {
            assert_eq!(gas_day_start_utc(day), cursor, "{day}");
            cursor = gas_day_end_utc(day);
            day = day.next_day().unwrap();
        }

        // Spring: Saturday's Gastag is 23 hours, the transition Sunday's 24.
        let sat = gas_day_end_utc(date!(2026 - 03 - 28)) - gas_day_start_utc(date!(2026 - 03 - 28));
        let sun = gas_day_end_utc(date!(2026 - 03 - 29)) - gas_day_start_utc(date!(2026 - 03 - 29));
        assert_eq!(sat.whole_hours(), 23);
        assert_eq!(sun.whole_hours(), 24);

        // Instants map to the gas day they belong to.
        assert_eq!(
            local_gas_day(datetime!(2026-07-15 3:30 UTC)), // 05:30 local
            date!(2026 - 07 - 14)
        );
        assert_eq!(
            local_gas_day(datetime!(2026-07-15 4:30 UTC)), // 06:30 local
            date!(2026 - 07 - 15)
        );
        // Consistency: every instant lies inside its own gas day's bounds.
        for ts in [
            datetime!(2026-10-25 0:30 UTC),
            datetime!(2026-10-25 4:59 UTC),
            datetime!(2026-10-25 5:00 UTC),
            datetime!(2026-03-29 0:30 UTC),
        ] {
            let day = local_gas_day(ts);
            assert!(
                gas_day_start_utc(day) <= ts && ts < gas_day_end_utc(day),
                "{ts}"
            );
        }
    }
}
