//! EEG § 51 and § 51a — quarter-hours with a negative spot price, and the
//! extension of the Vergütungszeitraum they earn.
//!
//! **§ 51 Abs. 1.** *"Für Zeiträume, in denen der Spotmarktpreis negativ ist,
//! verringert sich der anzulegende Wert auf null."* Input: the set of
//! negative-price quarter-hours ([`NegativePrices`]).
//!
//! **§ 51a Abs. 1.** The Vergütungszeitraum is extended by the number of
//! those quarter-hours *"im Jahr der Inbetriebnahme und in den
//! darauffolgenden 19 Kalenderjahren"* ([`zero_value_quarter_hours`]); the
//! count *"wird aufgerundet auf den nächsten vollen Kalendertag"*
//! ([`extension_days`]).
//!
//! **§ 51a Abs. 2.** For a Solaranlage the count is multiplied by 0,5 and
//! *"auf die nächste volle Viertelstunde aufgerundet
//! (Volllastviertelstunden)"* ([`volllastviertelstunden`]), then spent month
//! by month against [`VOLLLASTVIERTELSTUNDEN`] ([`solar_extension`]).
//!
//! **§ 51 Abs. 3.** The energy fed in while the price was negative without
//! interruption, per span ([`negative_span_energy`]); the 5 % reduction is
//! money and stays out.
//!
//! Whether § 51 applies to a plant (§ 51 Abs. 2, § 51b, § 100 Abs. 46/47) is
//! the caller's fact. Only the quarter-hour measure of the current § 51
//! Abs. 1 is implemented, not the hour-based measures of the versions of
//! 24.02.2025 and 31.12.2022 named in § 51a Abs. 3 and 4. A returned end
//! date ends an entitlement period; it is not a Frist.

use std::collections::BTreeSet;

use rust_decimal::Decimal;
use time::{Date, Duration, Month, OffsetDateTime};

use super::{EegError, sum};
use crate::series::Series;
use crate::time::calendar::DayBoundary;
use crate::time::resolution::Resolution;

/// The Volllastviertelstunden of each month, January first — § 51a Abs. 2
/// Satz 3 Nr. 1 to 12 EEG: *"87 für den Monat Januar"*, …, *"73 für den
/// Monat Dezember"*.
pub const VOLLLASTVIERTELSTUNDEN: [u32; 12] =
    [87, 189, 340, 442, 490, 508, 498, 453, 371, 231, 118, 73];

/// The Volllastviertelstunden of `month` ([`VOLLLASTVIERTELSTUNDEN`]).
#[must_use]
pub const fn volllastviertelstunden_of(month: Month) -> u32 {
    VOLLLASTVIERTELSTUNDEN[month as usize - 1]
}

/// The quarter-hours in which the spot price was negative (§ 51 Abs. 1 EEG).
///
/// Entries are Berlin-grid quarter-hour starts; the repeated autumn hour
/// counts once per UTC occurrence.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct NegativePrices {
    starts: BTreeSet<OffsetDateTime>,
}

impl NegativePrices {
    /// Collect the quarter-hour starts with a negative spot price.
    ///
    /// # Errors
    ///
    /// [`EegError::NotQuarterHour`] for an instant that does not start a
    /// quarter-hour; [`EegError::Calendar`] outside the supported years.
    pub fn new(starts: impl IntoIterator<Item = OffsetDateTime>) -> Result<Self, EegError> {
        let mut set = BTreeSet::new();
        for at in starts {
            let bucket = DayBoundary::Strom
                .bucket(at, Resolution::QUARTER_HOUR)
                .ok_or(EegError::Calendar)?;
            if bucket.start() != at {
                return Err(EegError::NotQuarterHour { at });
            }
            set.insert(at);
        }
        Ok(Self { starts: set })
    }

    /// `true` when the quarter-hour starting at `at` had a negative price.
    #[must_use]
    pub fn contains(&self, at: OffsetDateTime) -> bool {
        self.starts.contains(&at)
    }

    /// The negative quarter-hours starting in `[from, to)`.
    #[must_use]
    pub fn count(&self, from: OffsetDateTime, to: OffsetDateTime) -> u32 {
        if from >= to {
            return 0;
        }
        u32::try_from(self.starts.range(from..to).count()).unwrap_or(u32::MAX)
    }

    /// The negative quarter-hours of a Berlin calendar year (§ 51a Abs. 3
    /// Nr. 1 / Abs. 4 Nr. 1 Buchst. a); `None` outside the supported years.
    #[must_use]
    pub fn count_in_year(&self, year: i32) -> Option<u32> {
        let y = DayBoundary::Strom.year(year)?;
        Some(self.count(y.start(), y.end()))
    }
}

/// The § 51a Abs. 1 count for one plant, with the window it ran over.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct ZeroValueCount {
    /// Window start: commissioning, or the end of the exempt years.
    pub from: OffsetDateTime,
    /// Window end: the end of the 19th calendar year after commissioning.
    pub to: OffsetDateTime,
    /// The quarter-hours in the window whose anzulegender Wert was zero.
    pub quarter_hours: u32,
}

/// § 51a Abs. 1 Satz 1 EEG: a plant's zero-value quarter-hours from
/// commissioning to the end of the 19th calendar year after it.
///
/// `exempt_through_year` is the last calendar year a § 51 Abs. 2 exemption
/// covers (*"vor dem Ablauf des Kalenderjahres"*, Nr. 1 or 2); no
/// quarter-hour of that year or earlier counts.
///
/// # Errors
///
/// [`EegError::Calendar`] when the window leaves the supported years.
pub fn zero_value_quarter_hours(
    negative: &NegativePrices,
    commissioning: OffsetDateTime,
    exempt_through_year: Option<i32>,
) -> Result<ZeroValueCount, EegError> {
    let first_year = DayBoundary::Strom
        .day_of(commissioning)
        .ok_or(EegError::Calendar)?
        .year();
    let last = first_year.checked_add(19).ok_or(EegError::Calendar)?;
    let to = DayBoundary::Strom
        .year(last)
        .ok_or(EegError::Calendar)?
        .end();
    let from = match exempt_through_year {
        Some(y) => {
            let end = DayBoundary::Strom.year(y).ok_or(EegError::Calendar)?.end();
            end.max(commissioning)
        }
        None => commissioning,
    };
    Ok(ZeroValueCount {
        from,
        to,
        quarter_hours: negative.count(from, to),
    })
}

/// The quarter-hours of a 24-hour Kalendertag.
const QUARTER_HOURS_PER_DAY: u32 = 96;

/// § 51a Abs. 1 Satz 2 EEG: the count in Kalendertage, rounded up —
/// ⌈quarter-hours ÷ 96⌉.
///
/// 96 per day regardless of DST: the statute converts a count, not a span.
///
/// ```rust
/// use metering::eeg::negative_prices::extension_days;
///
/// assert_eq!(extension_days(96), 1);
/// assert_eq!(extension_days(97), 2); // up, never to nearest
/// assert_eq!(extension_days(0), 0);
/// ```
#[must_use]
pub const fn extension_days(quarter_hours: u32) -> u32 {
    quarter_hours.div_ceil(QUARTER_HOURS_PER_DAY)
}

/// § 51a Abs. 2 Satz 1 EEG: the count multiplied by 0,5 and rounded up to
/// the next full quarter-hour — ⌈quarter-hours ÷ 2⌉.
///
/// ```rust
/// use metering::eeg::negative_prices::volllastviertelstunden;
///
/// assert_eq!(volllastviertelstunden(1001), 501); // 500,5 → 501
/// assert_eq!(volllastviertelstunden(1000), 500);
/// ```
#[must_use]
pub const fn volllastviertelstunden(quarter_hours: u32) -> u32 {
    quarter_hours.div_ceil(2)
}

/// One month of a [`SolarExtension`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct MonthDraw {
    /// Calendar year.
    pub year: i32,
    /// Calendar month.
    pub month: Month,
    /// The month's Volllastviertelstunden, prorated in the month the original
    /// period ends.
    pub available: Decimal,
    /// What the budget spends in this month.
    pub drawn: Decimal,
}

/// The § 51a Abs. 2 extension of a Solaranlage's Vergütungszeitraum.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct SolarExtension {
    /// The budget of Volllastviertelstunden spent.
    pub budget: u32,
    /// The months it is spent over, in order.
    pub months: Vec<MonthDraw>,
    /// Last day of the extended Vergütungszeitraum (a month end); the original
    /// end for a zero budget.
    pub end: Date,
}

/// § 51a Abs. 2 Satz 3 to 6 EEG: spend `budget` Volllastviertelstunden from
/// the day after `original_end` (the last day of the original period).
///
/// A part month offers its table value × remaining days ÷ days of the month
/// (Satz 4, unrounded). The extension runs
/// *"bis zum Ende des Monats, auf den die letzte auszugleichende
/// Volllastviertelstunde entfällt"* (Satz 6).
///
/// `None` past the end of the supported dates.
///
/// ```rust
/// use metering::eeg::negative_prices::{solar_extension, volllastviertelstunden};
/// use time::macros::date;
///
/// // 500 Volllastviertelstunden from 15 April: 15/30 × 442 = 221 in April,
/// // the remaining 279 in May.
/// let e = solar_extension(date!(2045 - 04 - 15), volllastviertelstunden(1000)).unwrap();
/// assert_eq!(e.end, date!(2045 - 05 - 31));
/// ```
#[must_use]
pub fn solar_extension(original_end: Date, budget: u32) -> Option<SolarExtension> {
    let mut left = Decimal::from(budget);
    let mut months = Vec::new();
    if left.is_zero() {
        return Some(SolarExtension {
            budget,
            months,
            end: original_end,
        });
    }
    let (mut year, mut month) = (original_end.year(), original_end.month());
    let days = Decimal::from(month.length(year));
    let remaining = days.checked_sub(Decimal::from(original_end.day()))?;
    let mut available = Decimal::from(volllastviertelstunden_of(month))
        .checked_mul(remaining)?
        .checked_div(days)?;
    loop {
        if available > Decimal::ZERO {
            let drawn = available.min(left);
            left = left.checked_sub(drawn)?;
            months.push(MonthDraw {
                year,
                month,
                available,
                drawn,
            });
            if left.is_zero() {
                let end = Date::from_calendar_date(year, month, month.length(year)).ok()?;
                return Some(SolarExtension {
                    budget,
                    months,
                    end,
                });
            }
        }
        if month == Month::December {
            year = year.checked_add(1)?;
        }
        month = month.next();
        available = Decimal::from(volllastviertelstunden_of(month));
    }
}

/// One uninterrupted run of negative-price quarter-hours and its feed-in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct NegativeSpan {
    /// First quarter-hour start (UTC).
    pub from: OffsetDateTime,
    /// End of the last quarter-hour (UTC, exclusive).
    pub to: OffsetDateTime,
    /// Quarter-hours in the span.
    pub quarter_hours: u32,
    /// Energy fed in, kWh.
    pub kwh: Decimal,
}

/// The § 51 Abs. 3 quantity of one calendar month.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct SpanEnergy {
    /// Each uninterrupted negative-price span inside the month, in order.
    pub spans: Vec<NegativeSpan>,
    /// `Σ kwh` of the spans.
    pub total: Decimal,
}

/// § 51 Abs. 3 EEG: the energy fed in *"in dem Zeitraum …, in dem der
/// Spotmarktpreis ohne Unterbrechung negativ gewesen ist"*, for one Berlin
/// calendar month, span by span.
///
/// A span crossing the month boundary is cut at it.
///
/// # Errors
///
/// [`EegError::Resolution`] unless `feed_in` is quarter-hourly;
/// [`EegError::Missing`] or [`EegError::NotBillable`] for a negative-price
/// quarter-hour without a usable feed-in value; [`EegError::Calendar`]
/// outside the supported years; [`EegError::Overflow`].
pub fn negative_span_energy(
    feed_in: &Series,
    negative: &NegativePrices,
    year: i32,
    month: Month,
) -> Result<SpanEnergy, EegError> {
    if feed_in.resolution() != Resolution::QUARTER_HOUR {
        return Err(EegError::Resolution {
            series: "feed-in",
            found: feed_in.resolution(),
        });
    }
    let period = DayBoundary::Strom
        .month(year, month)
        .ok_or(EegError::Calendar)?;
    let step = Duration::minutes(15);
    let mut spans: Vec<NegativeSpan> = Vec::new();
    for &at in negative.starts.range(period.start()..period.end()) {
        let iv = feed_in.get(at).ok_or(EegError::Missing {
            series: "feed-in",
            at,
        })?;
        if !iv.quality().is_billable() {
            return Err(EegError::NotBillable {
                series: "feed-in",
                at,
            });
        }
        match spans.last_mut() {
            Some(span) if span.to == at => {
                span.to = at + step;
                span.quarter_hours += 1;
                span.kwh = span.kwh.checked_add(iv.value()).ok_or(EegError::Overflow)?;
            }
            _ => spans.push(NegativeSpan {
                from: at,
                to: at + step,
                quarter_hours: 1,
                kwh: iv.value(),
            }),
        }
    }
    let total = sum(spans.iter().map(|s| s.kwh))?;
    Ok(SpanEnergy { spans, total })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::series::interval::{MeterInterval, QualityFlag};
    use rust_decimal::dec;
    use time::macros::{date, datetime};

    #[test]
    fn the_monthly_table_is_the_statutes() {
        assert_eq!(
            VOLLLASTVIERTELSTUNDEN,
            [87, 189, 340, 442, 490, 508, 498, 453, 371, 231, 118, 73]
        );
        assert_eq!(volllastviertelstunden_of(Month::January), 87);
        assert_eq!(volllastviertelstunden_of(Month::December), 73);
    }

    #[test]
    fn both_roundings_go_up() {
        assert_eq!(extension_days(1), 1);
        assert_eq!(extension_days(95), 1);
        assert_eq!(extension_days(96), 1);
        assert_eq!(extension_days(145), 2); // 1,51 days — nearest would also be 2
        assert_eq!(extension_days(100), 2); // 1,04 days — nearest would be 1
        assert_eq!(volllastviertelstunden(1), 1);
        assert_eq!(volllastviertelstunden(3), 2);
        assert_eq!(volllastviertelstunden(0), 0);
    }

    #[test]
    fn solar_extension_prorates_the_first_month_and_ends_at_month_end() {
        let e = solar_extension(date!(2045 - 04 - 15), volllastviertelstunden(1000)).unwrap();
        assert_eq!(e.budget, 500);
        assert_eq!(e.months.len(), 2);
        assert_eq!(e.months[0].available, dec!(221));
        assert_eq!(e.months[0].drawn, dec!(221));
        assert_eq!(e.months[1].month, Month::May);
        assert_eq!(e.months[1].drawn, dec!(279));
        assert_eq!(e.end, date!(2045 - 05 - 31));
    }

    #[test]
    fn a_budget_exhausted_exactly_at_a_month_end_ends_that_month() {
        // Ends 31 December: no proration; January 87 + February 189 = 276.
        let e = solar_extension(date!(2044 - 12 - 31), 276).unwrap();
        assert_eq!(e.end, date!(2045 - 02 - 28));
        let e = solar_extension(date!(2044 - 12 - 31), 277).unwrap();
        assert_eq!(e.end, date!(2045 - 03 - 31));
        let e = solar_extension(date!(2044 - 12 - 31), 0).unwrap();
        assert_eq!(e.end, date!(2044 - 12 - 31));
        assert!(e.months.is_empty());
    }

    #[test]
    fn a_budget_used_inside_the_prorated_month_ends_that_month() {
        // 10 June: 20/30 × 508 = 338,66…; a budget of 300 stays in June.
        let e = solar_extension(date!(2045 - 06 - 10), 300).unwrap();
        assert_eq!(e.end, date!(2045 - 06 - 30));
        assert_eq!(e.months.len(), 1);
        assert_eq!(e.months[0].drawn, dec!(300));
    }

    fn year_set(starts: &[OffsetDateTime]) -> NegativePrices {
        NegativePrices::new(starts.iter().copied()).unwrap()
    }

    #[test]
    fn the_set_accepts_only_quarter_hour_starts() {
        assert_eq!(
            NegativePrices::new([datetime!(2026-06-01 10:05 UTC)]),
            Err(EegError::NotQuarterHour {
                at: datetime!(2026-06-01 10:05 UTC)
            })
        );
    }

    #[test]
    fn the_repeated_hour_counts_each_utc_quarter_hour() {
        // 2026-10-25: local 02:00–03:00 runs twice, 00:00–02:00 UTC.
        let starts: Vec<_> = (0..8)
            .map(|i| datetime!(2026-10-25 0:00 UTC) + Duration::minutes(15 * i))
            .collect();
        let set = year_set(&starts);
        assert_eq!(set.count_in_year(2026), Some(8));
    }

    #[test]
    fn the_window_runs_from_commissioning_through_the_nineteenth_year() {
        let set = year_set(&[
            datetime!(2026-03-01 11:00 UTC), // before commissioning
            datetime!(2026-07-01 11:00 UTC),
            datetime!(2045-12-31 12:00 UTC), // last year of the window
            datetime!(2046-01-01 12:00 UTC), // after it
        ]);
        let c = zero_value_quarter_hours(&set, datetime!(2026-05-01 10:00 UTC), None).unwrap();
        assert_eq!(c.quarter_hours, 2);
        assert_eq!(c.to, datetime!(2045-12-31 23:00 UTC));
        // Exempt through 2026: the 2026 quarter-hour does not count.
        let c =
            zero_value_quarter_hours(&set, datetime!(2026-05-01 10:00 UTC), Some(2026)).unwrap();
        assert_eq!(c.quarter_hours, 1);
        assert_eq!(c.from, datetime!(2026-12-31 23:00 UTC));
    }

    fn feed_in(start: OffsetDateTime, values: &[Decimal]) -> Series {
        let ivs = values
            .iter()
            .enumerate()
            .map(|(i, &v)| {
                MeterInterval::quarter_hour(
                    start + Duration::minutes(15 * i as i64),
                    v,
                    QualityFlag::Measured,
                )
                .unwrap()
            })
            .collect();
        Series::new(Resolution::QUARTER_HOUR, DayBoundary::Strom, ivs).unwrap()
    }

    #[test]
    fn span_energy_groups_uninterrupted_runs_and_cuts_at_the_month() {
        // 31 May 21:30 UTC = 23:30 local; the June month starts 31 May 22:00 UTC.
        let t = datetime!(2026-05-31 21:30 UTC);
        let series = feed_in(
            t,
            &[
                dec!(1),
                dec!(2),
                dec!(3),
                dec!(4),
                dec!(5),
                dec!(6),
                dec!(7),
            ],
        );
        let q = |i: i64| t + Duration::minutes(15 * i);
        let set = year_set(&[q(0), q(1), q(2), q(3), q(5), q(6)]);
        let june = negative_span_energy(&series, &set, 2026, Month::June).unwrap();
        assert_eq!(june.spans.len(), 2);
        assert_eq!(june.spans[0].from, q(2));
        assert_eq!(june.spans[0].quarter_hours, 2);
        assert_eq!(june.spans[0].kwh, dec!(7));
        assert_eq!(june.spans[1].kwh, dec!(13));
        assert_eq!(june.total, dec!(20));
        let may = negative_span_energy(&series, &set, 2026, Month::May).unwrap();
        assert_eq!(may.total, dec!(3));
        // A negative quarter-hour without a feed-in value is refused.
        let gap = year_set(&[q(9)]);
        assert!(matches!(
            negative_span_energy(&series, &gap, 2026, Month::June),
            Err(EegError::Missing { .. })
        ));
    }
}
