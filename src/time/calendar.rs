//! Europe/Berlin calendar arithmetic — the German market's day, month and year.
//!
//! Metering periods are local calendar periods: a Bilanzierungstag starts at
//! 00:00 Europe/Berlin (23:00 or 22:00 UTC the previous day), and a day is
//! 23, 24 or 25 hours long (92, 96 or 100 quarter-hours). EDI@Energy
//! *Allgemeine Festlegungen* 6.1d, Kap. 3: *"Die Angabe von Zeiten in einer EDIFACT
//! Nachricht erfolgt in koordinierter Weltzeit (Coordinated Universal Time,
//! UTC). In Deutschland gilt die Mitteleuropäische Zeit (MEZ) bzw. die
//! Mitteleuropäische Sommerzeit (MESZ) als gesetzliche deutsche Zeit. Alle in
//! den Prozessen genannten Zeitpunkte … nutzen die gesetzliche deutsche
//! Zeit."* Transitions come from the IANA tz database via `time-tz`
//! ([`berlin`]).
//!
//! Day starts per Kap. 3.1 (asserted in `tests/it/published_examples.rs`):
//!
//! | Sparte | MEZ | MESZ |
//! |---|---|---|
//! | Strom (00:00 local) | `2300` — 23:00 UTC | `2200` — 22:00 UTC |
//! | Gas (06:00 local) | `0500` — 05:00 UTC | `0400` — 04:00 UTC |
//!
//! Leap seconds (Kap. 3.9 `23:59:60`) cannot be represented by
//! [`time::OffsetDateTime`]; such a timestamp fails at the parse.
//!
//! Start here: a [`DayBoundary`] names a day, month or year as a half-open UTC
//! [`Period`]. Supported years are [`FIRST_YEAR`]..=[`LAST_YEAR`]; outside
//! them every constructor answers `None`.
//!
//! ```rust
//! use metering::{DayBoundary, Resolution};
//! use time::Month;
//! use time::macros::date;
//!
//! // The spring-forward day is 23 hours long — 92 quarter-hours, not 96.
//! let day = DayBoundary::Strom.day(date!(2026 - 03 - 29)).unwrap();
//! assert_eq!(day.count(Resolution::QUARTER_HOUR), Some(92));
//! let march = DayBoundary::Strom.month(2026, Month::March).unwrap();
//! assert_eq!(march.count(Resolution::QUARTER_HOUR), Some(2_972));
//! ```

use time::{Date, Duration, Month, OffsetDateTime, PrimitiveDateTime, Time, UtcOffset};
use time_tz::{
    Offset as _, OffsetDateTimeExt as _, OffsetResult, PrimitiveDateTimeExt as _, TimeZone as _,
    Tz, timezones,
};

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use crate::time::resolution::Resolution;

/// The first year the calendar answers for.
pub const FIRST_YEAR: i32 = 1900;
/// The last year the calendar answers for.
pub const LAST_YEAR: i32 = 9998;

/// The Europe/Berlin timezone, from the IANA tz database.
#[must_use]
pub fn berlin() -> &'static Tz {
    timezones::db::europe::BERLIN
}

/// Convert a UTC instant to Europe/Berlin local time.
#[must_use]
pub fn to_berlin(instant: OffsetDateTime) -> OffsetDateTime {
    instant.to_timezone(berlin())
}

fn supported(year: i32) -> bool {
    (FIRST_YEAR..=LAST_YEAR).contains(&year)
}

/// Which daily boundary a period is cut on: the calendar day (Strom) or the
/// **Gastag**, 06:00 to 06:00 local (GaBi Gas, following Art. 3 Nr. 6
/// VO (EU) 312/2014).
///
/// The boundary carries up to the month. EDI@Energy *Allgemeine
/// Festlegungen* 6.1d, Kap. 3.1: *"Die Angabe des Bilanzierungsmonats erfolgt
/// unter Angabe von Jahr und Monat (z. B. Juni 2021), sodass damit der
/// Zeitraum vom 01.06.2021 00:00 Uhr bis 01.07.2021 00:00 Uhr gesetzlicher
/// deutscher Zeit abgedeckt ist, wenn es sich um den Bilanzierungsmonat in der
/// Sparte Strom handelt, in der Sparte Gas ist damit der Zeitraum vom
/// 01.06.2021 06:00 Uhr bis 01.07.2021 06:00 Uhr gesetzlicher deutscher Zeit
/// abgedeckt."*
///
/// The clocks move before 06:00, so the long or short Gastag is the one
/// named after the Saturday. The SLP-Gas Leitfaden: *"Daher
/// ist die Zeitumstellung in den Werten für den Samstag vor der Umstellung zu
/// berücksichtigen."*
///
/// ```rust
/// use metering::DayBoundary;
/// use time::macros::{date, datetime};
///
/// let day = date!(2026 - 01 - 15);
/// assert_eq!(DayBoundary::Strom.day(day).unwrap().start(), datetime!(2026-01-14 23:00 UTC));
/// assert_eq!(DayBoundary::Gas.day(day).unwrap().start(), datetime!(2026-01-15 5:00 UTC));
///
/// // The 25-hour Gastag is Saturday's.
/// let saturday = DayBoundary::Gas.day(date!(2026 - 10 - 24)).unwrap();
/// assert_eq!(saturday.duration().whole_hours(), 25);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DayBoundary {
    /// 00:00 Europe/Berlin — the Liefertag of the electricity market.
    Strom,
    /// 06:00 Europe/Berlin — the Gastag of the gas market.
    Gas,
}

impl DayBoundary {
    /// Every variant, in declaration order.
    pub const ALL: [Self; 2] = [Self::Strom, Self::Gas];

    /// Stable DB/wire label. Matches the `serde` tag and
    /// [`FromStr`](std::str::FromStr) input.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Strom => "STROM",
            Self::Gas => "GAS",
        }
    }

    /// The Berlin wall-clock time a day starts at under this boundary.
    #[must_use]
    pub const fn local_start(self) -> Time {
        match self {
            Self::Strom => Time::MIDNIGHT,
            Self::Gas => time::macros::time!(6:00),
        }
    }

    fn start_of(self, date: Date) -> Option<OffsetDateTime> {
        if !supported(date.year()) {
            return None;
        }
        Some(resolve_local(PrimitiveDateTime::new(
            date,
            self.local_start(),
        )))
    }

    fn period(self, first: Date, next: Date) -> Option<Period> {
        let start = self.start_of(first)?;
        // Unchecked: the exclusive end may lie in `LAST_YEAR + 1`.
        let end = resolve_local(PrimitiveDateTime::new(next, self.local_start()));
        Some(Period {
            start,
            end,
            boundary: self,
        })
    }

    /// The day named `date` as a half-open UTC [`Period`]: 23, 24 or 25 hours;
    /// `None` outside the supported years.
    #[must_use]
    pub fn day(self, date: Date) -> Option<Period> {
        self.period(date, date.next_day()?)
    }

    /// The month `month` of `year` — the **Bilanzierungsmonat**.
    ///
    /// ```rust
    /// use metering::DayBoundary;
    /// use time::Month;
    /// use time::macros::datetime;
    ///
    /// let march = DayBoundary::Strom.month(2026, Month::March).unwrap();
    /// assert_eq!(march.start(), datetime!(2026-02-28 23:00 UTC));
    /// assert_eq!(march.end(), datetime!(2026-03-31 22:00 UTC));
    /// assert_eq!(march.duration().whole_hours(), 31 * 24 - 1);
    /// ```
    #[must_use]
    pub fn month(self, year: i32, month: Month) -> Option<Period> {
        let first = Date::from_calendar_date(year, month, 1).ok()?;
        let next = if month == Month::December {
            Date::from_calendar_date(year.checked_add(1)?, Month::January, 1).ok()?
        } else {
            Date::from_calendar_date(year, month.next(), 1).ok()?
        };
        self.period(first, next)
    }

    /// The year `year`.
    #[must_use]
    pub fn year(self, year: i32) -> Option<Period> {
        let first = Date::from_calendar_date(year, Month::January, 1).ok()?;
        let next = Date::from_calendar_date(year.checked_add(1)?, Month::January, 1).ok()?;
        self.period(first, next)
    }

    /// The day an instant belongs to under this boundary (not
    /// `instant.date()`, which is the UTC day); `None` outside the supported
    /// years.
    ///
    /// ```rust
    /// use metering::DayBoundary;
    /// use time::macros::{date, datetime};
    ///
    /// // 23:30 UTC on 14 July is already 01:30 on 15 July in Berlin.
    /// assert_eq!(DayBoundary::Strom.day_of(datetime!(2026-07-14 23:30 UTC)), Some(date!(2026 - 07 - 15)));
    /// // 03:30 UTC on 15 July is 05:30 local — still the Gastag of the 14th.
    /// assert_eq!(DayBoundary::Gas.day_of(datetime!(2026-07-15 3:30 UTC)), Some(date!(2026 - 07 - 14)));
    /// ```
    #[must_use]
    pub fn day_of(self, instant: OffsetDateTime) -> Option<Date> {
        let local = to_berlin(instant);
        let date = if local.time() < self.local_start() {
            local.date().previous_day()?
        } else {
            local.date()
        };
        supported(date.year()).then_some(date)
    }

    /// Whole calendar days between the days two instants belong to — unlike
    /// `(to - from).whole_days()`, which truncates across a 23-hour day.
    /// `None` outside the supported years.
    #[must_use]
    pub fn days_between(self, from: OffsetDateTime, to: OffsetDateTime) -> Option<i64> {
        Some((self.day_of(to)? - self.day_of(from)?).whole_days())
    }

    /// The bucket of `resolution` containing `instant`, as a [`Period`].
    ///
    /// Day, month and year buckets are calendar periods on this boundary.
    /// Fixed buckets are laid out from the start of the local day (not the
    /// Unix epoch), so no bucket straddles a day.
    ///
    /// ```rust
    /// use metering::{DayBoundary, Resolution};
    /// use time::macros::datetime;
    ///
    /// let b = DayBoundary::Strom
    ///     .bucket(datetime!(2026-06-01 12:07 UTC), Resolution::QUARTER_HOUR)
    ///     .unwrap();
    /// assert_eq!(b.start(), datetime!(2026-06-01 12:00 UTC));
    /// assert_eq!(b.end(), datetime!(2026-06-01 12:15 UTC));
    /// ```
    #[must_use]
    pub fn bucket(self, instant: OffsetDateTime, resolution: Resolution) -> Option<Period> {
        let date = self.day_of(instant)?;
        match resolution {
            Resolution::Day => self.day(date),
            Resolution::Month => self.month(date.year(), date.month()),
            Resolution::Year => self.year(date.year()),
            Resolution::Minutes(_) | Resolution::Hour => {
                let day = self.day(date)?;
                let step = i64::from(resolution.fixed_seconds()?);
                let offset = (instant - day.start).whole_seconds();
                let start = day.start + Duration::seconds(offset.div_euclid(step) * step);
                Some(Period {
                    start,
                    end: start + Duration::seconds(step),
                    boundary: self,
                })
            }
        }
    }
}

crate::ids::codes::string_codes! {
    DayKind;
    DayBoundary;
}

/// A half-open UTC span `[start, end)` cut on a [`DayBoundary`], built by
/// [`DayBoundary::day`], [`month`](DayBoundary::month),
/// [`year`](DayBoundary::year) and [`bucket`](DayBoundary::bucket).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct Period {
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::rfc3339"))]
    start: OffsetDateTime,
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::rfc3339"))]
    end: OffsetDateTime,
    boundary: DayBoundary,
}

impl Period {
    /// The first instant (inclusive), UTC.
    #[must_use]
    pub const fn start(&self) -> OffsetDateTime {
        self.start
    }

    /// The end (exclusive), UTC.
    #[must_use]
    pub const fn end(&self) -> OffsetDateTime {
        self.end
    }

    /// The boundary the period was cut on.
    #[must_use]
    pub const fn boundary(&self) -> DayBoundary {
        self.boundary
    }

    /// The two ends as a pair, for APIs taking `(from, to)`.
    #[must_use]
    pub const fn range(&self) -> (OffsetDateTime, OffsetDateTime) {
        (self.start, self.end)
    }

    /// The elapsed time — never a whole number of days across a transition.
    #[must_use]
    pub fn duration(&self) -> Duration {
        self.end - self.start
    }

    /// `true` when `instant` lies in `[start, end)`.
    #[must_use]
    pub fn contains(&self, instant: OffsetDateTime) -> bool {
        self.start <= instant && instant < self.end
    }

    /// How many intervals of `resolution` the period holds.
    ///
    /// Fixed resolutions divide the elapsed time (`None` when not exact);
    /// `Day`, `Month` and `Year` count calendar units on the period's own
    /// boundary (`None` when the period does not start and end on one).
    #[must_use]
    pub fn count(&self, resolution: Resolution) -> Option<u32> {
        if let Some(step) = resolution.fixed_seconds() {
            let secs = self.duration().whole_seconds();
            let step = i64::from(step);
            return (secs >= 0 && secs % step == 0)
                .then(|| u32::try_from(secs / step).ok())
                .flatten();
        }
        let b = self.boundary;
        let first = b.day_of(self.start)?;
        // The last day is the one the final instant belongs to.
        let last = b.day_of(self.end - Duration::nanoseconds(1))?;
        let n = match resolution {
            Resolution::Day => (b.day(first)?.start == self.start && b.day(last)?.end == self.end)
                .then(|| (last - first).whole_days() + 1)?,
            Resolution::Month => {
                let aligned = b.month(first.year(), first.month())?.start == self.start
                    && b.month(last.year(), last.month())?.end == self.end;
                aligned.then(|| {
                    i64::from(last.year() - first.year()) * 12 + i64::from(last.month() as u8)
                        - i64::from(first.month() as u8)
                        + 1
                })?
            }
            Resolution::Year => {
                let aligned = b.year(first.year())?.start == self.start
                    && b.year(last.year())?.end == self.end;
                aligned.then(|| i64::from(last.year() - first.year()) + 1)?
            }
            Resolution::Minutes(_) | Resolution::Hour => return None,
        };
        u32::try_from(n).ok()
    }

    /// Whether the period is shorter or longer than its wall-clock span; a
    /// year contains both transitions and is [`Normal`](DayKind::Normal).
    #[must_use]
    pub fn kind(&self) -> DayKind {
        let wall = |t: OffsetDateTime| {
            let l = to_berlin(t);
            PrimitiveDateTime::new(l.date(), l.time())
        };
        let wall_span = wall(self.end) - wall(self.start);
        match self.duration().cmp(&wall_span) {
            std::cmp::Ordering::Less => DayKind::ShortDay,
            std::cmp::Ordering::Greater => DayKind::LongDay,
            std::cmp::Ordering::Equal => DayKind::Normal,
        }
    }

    /// The first instant inside the period at which the UTC offset changes.
    ///
    /// On the fall-back day the repeated local hour 02:00–03:00 occupies
    /// `[t − 1 h, t + 1 h)` in UTC.
    ///
    /// ```rust
    /// use metering::DayBoundary;
    /// use time::macros::{date, datetime};
    ///
    /// let autumn = DayBoundary::Strom.day(date!(2026 - 10 - 25)).unwrap();
    /// assert_eq!(autumn.transition(), Some(datetime!(2026-10-25 1:00 UTC)));
    /// assert_eq!(DayBoundary::Strom.day(date!(2026 - 07 - 20)).unwrap().transition(), None);
    /// ```
    #[must_use]
    pub fn transition(&self) -> Option<OffsetDateTime> {
        let offset = |t: OffsetDateTime| to_berlin(t).offset();
        let first = offset(self.start);
        // Berlin transitions and period starts are whole hours.
        let mut cursor = self.start;
        while cursor < self.end {
            let next = cursor + Duration::hours(1);
            if next < self.end && offset(next) != first {
                return Some(next);
            }
            cursor = next;
        }
        None
    }
}

/// A period's length relative to its wall-clock span.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DayKind {
    /// No net DST change — an ordinary 24-hour day.
    Normal,
    /// Spring forward — one hour short (CET → CEST, last Sunday in March).
    ShortDay,
    /// Fall back — one hour long (CEST → CET, last Sunday in October).
    LongDay,
}

impl DayKind {
    /// Every kind, in declaration order.
    pub const ALL: [Self; 3] = [Self::Normal, Self::ShortDay, Self::LongDay];

    /// Stable DB/wire label.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Normal => "NORMAL",
            Self::ShortDay => "SHORT_DAY",
            Self::LongDay => "LONG_DAY",
        }
    }

    /// `true` for the two DST transition kinds.
    #[must_use]
    pub const fn is_dst_transition(self) -> bool {
        !matches!(self, Self::Normal)
    }
}

/// The same Berlin wall-clock time `days` calendar days earlier (unlike
/// `Duration::days`, which is a fixed 24 h per day).
///
/// An ambiguous local time resolves to the earlier instant; a skipped one is
/// pushed forward by the gap. `None` when the result leaves the supported
/// years.
///
/// ```rust
/// use metering::time::calendar;
/// use time::macros::datetime;
///
/// let back = calendar::shift_back_days(datetime!(2026-10-28 11:00 UTC), 7).unwrap();
/// assert_eq!(back, datetime!(2026-10-21 10:00 UTC));
/// assert_eq!((datetime!(2026-10-28 11:00 UTC) - back).whole_hours(), 169);
/// ```
#[must_use]
pub fn shift_back_days(instant: OffsetDateTime, days: i64) -> Option<OffsetDateTime> {
    let local = to_berlin(instant);
    let date = local.date().checked_sub(Duration::days(days))?;
    supported(date.year()).then(|| resolve_local(PrimitiveDateTime::new(date, local.time())))
}

/// The same Berlin wall-clock time one year earlier; 29 February maps to
/// 28 February. `None` when the result leaves the supported years.
#[must_use]
pub fn shift_back_one_year(instant: OffsetDateTime) -> Option<OffsetDateTime> {
    let local = to_berlin(instant);
    let date = local.date();
    let target_year = date.year().checked_sub(1)?;
    if !supported(target_year) {
        return None;
    }
    let day = date
        .day()
        .min(time::util::days_in_month(date.month(), target_year));
    let shifted = Date::from_calendar_date(target_year, date.month(), day).ok()?;
    Some(resolve_local(PrimitiveDateTime::new(shifted, local.time())))
}

/// Resolve a Berlin wall-clock time to its UTC instant: an ambiguous time
/// takes the earlier instant, a skipped one is pushed forward by the gap
/// (02:30 → 03:30, as `java.time` and `zoneinfo` do).
fn resolve_local(naive: PrimitiveDateTime) -> OffsetDateTime {
    let resolved = match naive.assume_timezone(berlin()) {
        OffsetResult::Some(t) => t,
        OffsetResult::Ambiguous(first, _) => first,
        OffsetResult::None => {
            // The offset in force a day earlier lands past the jump.
            let before = naive.assume_utc() - Duration::days(1);
            naive.assume_offset(berlin().get_offset_utc(&before).to_utc())
        }
    };
    resolved.to_offset(UtcOffset::UTC)
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::{date, datetime};

    const STROM: DayBoundary = DayBoundary::Strom;
    const GAS: DayBoundary = DayBoundary::Gas;

    fn hours(p: Option<Period>) -> i64 {
        p.unwrap().duration().whole_hours()
    }

    #[test]
    fn dst_transition_days_are_23_and_25_hours() {
        assert_eq!(hours(STROM.day(date!(2026 - 03 - 29))), 23);
        assert_eq!(hours(STROM.day(date!(2026 - 07 - 20))), 24);
        assert_eq!(hours(STROM.day(date!(2026 - 10 - 25))), 25);
        let q = Resolution::QUARTER_HOUR;
        assert_eq!(STROM.day(date!(2026 - 03 - 29)).unwrap().count(q), Some(92));
        assert_eq!(
            STROM.day(date!(2026 - 10 - 25)).unwrap().count(q),
            Some(100)
        );
        assert_eq!(
            STROM.day(date!(2026 - 03 - 29)).unwrap().kind(),
            DayKind::ShortDay
        );
        assert_eq!(
            STROM.day(date!(2026 - 10 - 25)).unwrap().kind(),
            DayKind::LongDay
        );
        assert_eq!(
            STROM.day(date!(2026 - 07 - 20)).unwrap().kind(),
            DayKind::Normal
        );
    }

    #[test]
    fn months_and_years_count_quarter_hours_and_calendar_units() {
        let q = Resolution::QUARTER_HOUR;
        assert_eq!(
            STROM.month(2026, Month::March).unwrap().count(q),
            Some(2_972)
        );
        assert_eq!(
            STROM.month(2026, Month::October).unwrap().count(q),
            Some(2_980)
        );
        assert_eq!(
            STROM
                .month(2026, Month::March)
                .unwrap()
                .count(Resolution::Day),
            Some(31)
        );
        let y = STROM.year(2028).unwrap();
        assert_eq!(y.count(Resolution::Day), Some(366));
        assert_eq!(y.count(Resolution::Month), Some(12));
        assert_eq!(y.count(Resolution::Year), Some(1));
        assert_eq!(y.kind(), DayKind::Normal);
        assert_eq!(
            STROM.year(2026).unwrap().count(Resolution::Hour),
            Some(365 * 24)
        );
        // A day does not hold a whole number of months.
        assert_eq!(
            STROM
                .day(date!(2026 - 01 - 01))
                .unwrap()
                .count(Resolution::Month),
            None
        );
    }

    #[test]
    fn day_boundaries_are_local_not_utc() {
        assert_eq!(
            STROM.day(date!(2026 - 01 - 15)).unwrap().start(),
            datetime!(2026-01-14 23:00 UTC)
        );
        assert_eq!(
            STROM.day(date!(2026 - 07 - 15)).unwrap().start(),
            datetime!(2026-07-14 22:00 UTC)
        );
        assert_eq!(
            GAS.day(date!(2026 - 07 - 15)).unwrap().start(),
            datetime!(2026-07-15 4:00 UTC)
        );
    }

    #[test]
    fn days_tile_the_year_without_gaps() {
        for b in DayBoundary::ALL {
            let mut d = date!(2026 - 01 - 01);
            let mut prev_end = b.day(d).unwrap().start();
            let mut total = Duration::ZERO;
            while d.year() == 2026 {
                let p = b.day(d).unwrap();
                assert_eq!(p.start(), prev_end, "{b} {d}");
                prev_end = p.end();
                total += p.duration();
                d = d.next_day().unwrap();
            }
            assert_eq!(total, b.year(2026).unwrap().duration());
        }
    }

    #[test]
    fn the_gastag_shifts_the_transition_to_saturday() {
        assert_eq!(hours(GAS.day(date!(2026 - 10 - 24))), 25);
        assert_eq!(hours(GAS.day(date!(2026 - 10 - 25))), 24);
        assert_eq!(hours(GAS.day(date!(2026 - 03 - 28))), 23);
        let m = GAS.month(2026, Month::March).unwrap();
        assert_eq!(m.start(), datetime!(2026-03-01 5:00 UTC));
        assert_eq!(m.end(), datetime!(2026-04-01 4:00 UTC));
    }

    #[test]
    fn day_of_follows_the_boundary() {
        assert_eq!(
            STROM.day_of(datetime!(2026-07-14 23:30 UTC)),
            Some(date!(2026 - 07 - 15))
        );
        assert_eq!(
            GAS.day_of(datetime!(2026-07-15 4:30 UTC)),
            Some(date!(2026 - 07 - 15))
        );
        assert_eq!(
            GAS.day_of(datetime!(2026-07-15 3:30 UTC)),
            Some(date!(2026 - 07 - 14))
        );
    }

    #[test]
    fn unsupported_years_answer_none_rather_than_a_substitute() {
        assert!(STROM.day(date!(1899 - 12 - 31)).is_none());
        assert!(STROM.day(date!(9999 - 01 - 01)).is_none());
        assert!(STROM.year(9998).is_some());
        assert!(STROM.year(9999).is_none());
        assert!(STROM.month(1899, Month::December).is_none());
        assert!(STROM.day_of(datetime!(1850-06-01 0:00 UTC)).is_none());
        assert!(shift_back_days(datetime!(1900-01-02 12:00 UTC), 5).is_none());
        assert!(shift_back_one_year(datetime!(1900-06-01 12:00 UTC)).is_none());
    }

    #[test]
    fn buckets_snap_to_the_local_day_start_not_the_epoch() {
        let b = GAS
            .bucket(datetime!(2026-01-15 5:30 UTC), Resolution::Hour)
            .unwrap();
        assert_eq!(b.start(), datetime!(2026-01-15 5:00 UTC));
        let day = STROM.day(date!(2026 - 10 - 25)).unwrap();
        let mut t = day.start();
        let mut n = 0;
        while t < day.end() {
            let b = STROM.bucket(t, Resolution::QUARTER_HOUR).unwrap();
            assert_eq!(b.start(), t);
            t = b.end();
            n += 1;
        }
        assert_eq!(n, 100);
        assert_eq!(
            STROM.bucket(datetime!(2026-03-15 12:00 UTC), Resolution::Month),
            STROM.month(2026, Month::March)
        );
    }

    #[test]
    fn transitions_are_located_to_the_instant() {
        assert_eq!(
            STROM.day(date!(2026 - 03 - 29)).unwrap().transition(),
            Some(datetime!(2026-03-29 1:00 UTC))
        );
        assert_eq!(
            STROM.day(date!(2026 - 10 - 25)).unwrap().transition(),
            Some(datetime!(2026-10-25 1:00 UTC))
        );
        assert_eq!(STROM.day(date!(2026 - 07 - 20)).unwrap().transition(), None);
    }

    #[test]
    fn shifting_back_keeps_the_local_clock_time() {
        assert_eq!(
            shift_back_one_year(datetime!(2026-07-15 10:00 UTC)),
            Some(datetime!(2025-07-15 10:00 UTC))
        );
        let leap = STROM.day(date!(2028 - 02 - 29)).unwrap().start();
        let back = shift_back_one_year(leap).unwrap();
        assert_eq!(STROM.day_of(back), Some(date!(2027 - 02 - 28)));
        assert_eq!(
            shift_back_days(datetime!(2026-10-28 11:00 UTC), 7),
            Some(datetime!(2026-10-21 10:00 UTC))
        );
    }

    #[test]
    fn a_skipped_local_time_is_pushed_forward_by_the_gap() {
        let back = shift_back_days(datetime!(2026-03-30 0:30 UTC), 1).unwrap();
        assert_eq!(back, datetime!(2026-03-29 1:30 UTC));
        assert_eq!(to_berlin(back).hour(), 3);
    }

    #[test]
    fn an_ambiguous_local_time_resolves_to_the_earlier_pass() {
        let back = shift_back_days(datetime!(2026-10-26 1:30 UTC), 1).unwrap();
        assert_eq!(back, datetime!(2026-10-25 0:30 UTC));
    }
}
