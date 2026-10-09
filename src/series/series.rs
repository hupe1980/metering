//! [`Series`] — a validated, ordered run of [`MeterInterval`]s on one grid.
//!
//! [`Series::new`] checks once, so nothing downstream repeats it, that the
//! intervals are **sorted**, free of **duplicates** and **overlaps**, on the
//! **same channel** (OBIS code), and each exactly the bucket of the series'
//! [`Resolution`] cut on its [`DayBoundary`] — so a quarter-hour series holds
//! 92 slots on the spring-forward day and 100 on the autumn one, and a 25-hour
//! [`Day`](Resolution::Day) is one interval.
//!
//! **Gaps are allowed**: a missing slot is a finding for
//! [`crate::vee::validation`], not a structural error.
//!
//! ```rust
//! use metering::{DayBoundary, MeterInterval, QualityFlag, Resolution, Series};
//! use rust_decimal::dec;
//! use time::{Duration, macros::datetime};
//!
//! let start = datetime!(2026-06-01 0:00 UTC);
//! let slots = (0..4).map(|i| {
//!     MeterInterval::quarter_hour(start + Duration::minutes(15 * i), dec!(1.5), QualityFlag::Measured)
//! });
//! let series = Series::new(
//!     Resolution::QUARTER_HOUR,
//!     DayBoundary::Strom,
//!     slots.collect::<Result<Vec<_>, _>>()?,
//! )?;
//! assert_eq!(series.billable_total(), Some(dec!(6.0)));
//! assert!(series.get(start).is_some());
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

use rust_decimal::Decimal;
use time::OffsetDateTime;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use crate::ids::obis::ObisCode;
use crate::series::interval::{MeterInterval, Unit};
use crate::time::calendar::DayBoundary;
use crate::time::resolution::Resolution;

/// Why a [`Series`] could not be built. Every variant names the instant(s).
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum SeriesError {
    /// Two intervals start at the same instant.
    #[error("two intervals start at {from}")]
    Duplicate {
        /// The shared start.
        from: OffsetDateTime,
    },
    /// An interval starts before the previous one ends.
    #[error("the interval starting {later} overlaps the one starting {earlier}")]
    Overlap {
        /// Start of the earlier interval.
        earlier: OffsetDateTime,
        /// Start of the later, overlapping interval.
        later: OffsetDateTime,
    },
    /// An interval is not the bucket of the series' resolution at its start.
    #[error(
        "interval [{from}, {to}) is not a bucket of the series' grid (expected [{expected_from}, {expected_to}))"
    )]
    Misaligned {
        /// The interval's start.
        from: OffsetDateTime,
        /// The interval's end.
        to: OffsetDateTime,
        /// Start of the bucket containing `from`.
        expected_from: OffsetDateTime,
        /// End of the bucket containing `from`.
        expected_to: OffsetDateTime,
    },
    /// An interval lies outside the calendar's supported years.
    #[error("interval starting {from} lies outside the supported calendar years")]
    OutOfRange {
        /// The interval's start.
        from: OffsetDateTime,
    },
    /// An interval is on a different OBIS channel from the first.
    #[error("the interval starting {from} is on a different channel")]
    MixedChannels {
        /// The interval's start.
        from: OffsetDateTime,
    },
}

/// A validated, ordered run of intervals on one grid and one channel — see
/// the [module docs](crate::series).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(try_from = "RawSeries"))]
pub struct Series {
    resolution: Resolution,
    boundary: DayBoundary,
    intervals: Vec<MeterInterval>,
}

/// The unvalidated wire shape; deserialisation goes through [`Series::new`].
#[cfg(feature = "serde")]
#[derive(Deserialize)]
struct RawSeries {
    resolution: Resolution,
    boundary: DayBoundary,
    intervals: Vec<MeterInterval>,
}

#[cfg(feature = "serde")]
impl TryFrom<RawSeries> for Series {
    type Error = SeriesError;

    fn try_from(raw: RawSeries) -> Result<Self, Self::Error> {
        Self::new(raw.resolution, raw.boundary, raw.intervals)
    }
}

impl Series {
    /// Validate `intervals` as a series of `resolution` cut on `boundary`,
    /// sorting them by start.
    ///
    /// # Errors
    ///
    /// A [`SeriesError`] naming the first offending instant.
    pub fn new(
        resolution: Resolution,
        boundary: DayBoundary,
        mut intervals: Vec<MeterInterval>,
    ) -> Result<Self, SeriesError> {
        intervals.sort_by_key(MeterInterval::from);
        let channel = intervals.first().and_then(MeterInterval::obis);
        let mut previous: Option<&MeterInterval> = None;
        for iv in &intervals {
            if iv.obis() != channel {
                return Err(SeriesError::MixedChannels { from: iv.from() });
            }
            if let Some(prev) = previous {
                if prev.from() == iv.from() {
                    return Err(SeriesError::Duplicate { from: iv.from() });
                }
                if prev.to() > iv.from() {
                    return Err(SeriesError::Overlap {
                        earlier: prev.from(),
                        later: iv.from(),
                    });
                }
            }
            let bucket = boundary
                .bucket(iv.from(), resolution)
                .ok_or(SeriesError::OutOfRange { from: iv.from() })?;
            if bucket.range() != (iv.from(), iv.to()) {
                return Err(SeriesError::Misaligned {
                    from: iv.from(),
                    to: iv.to(),
                    expected_from: bucket.start(),
                    expected_to: bucket.end(),
                });
            }
            previous = Some(iv);
        }
        Ok(Self {
            resolution,
            boundary,
            intervals,
        })
    }

    /// The intervals, ascending.
    pub fn iter(&self) -> std::slice::Iter<'_, MeterInterval> {
        self.intervals.iter()
    }

    /// The intervals as a slice, ascending.
    #[must_use]
    pub fn as_slice(&self) -> &[MeterInterval] {
        &self.intervals
    }

    /// The intervals, ascending, by value.
    #[must_use]
    pub fn into_intervals(self) -> Vec<MeterInterval> {
        self.intervals
    }

    /// The number of intervals present (gaps not counted).
    #[must_use]
    pub fn len(&self) -> usize {
        self.intervals.len()
    }

    /// `true` when the series holds no interval.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.intervals.is_empty()
    }

    /// The grid.
    #[must_use]
    pub const fn resolution(&self) -> Resolution {
        self.resolution
    }

    /// The day boundary the grid is cut on.
    #[must_use]
    pub const fn boundary(&self) -> DayBoundary {
        self.boundary
    }

    /// The OBIS channel every interval shares, if they name one.
    #[must_use]
    pub fn channel(&self) -> Option<ObisCode> {
        self.intervals.first().and_then(MeterInterval::obis)
    }

    /// The unit of the channel, when it names one with a known unit.
    #[must_use]
    pub fn unit(&self) -> Option<Unit> {
        self.channel()?.unit()
    }

    /// The interval starting at `from`, if present.
    #[must_use]
    pub fn get(&self, from: OffsetDateTime) -> Option<&MeterInterval> {
        self.intervals
            .binary_search_by_key(&from, MeterInterval::from)
            .ok()
            .and_then(|i| self.intervals.get(i))
    }

    /// The sum of the **billable** intervals
    /// ([`QualityFlag::is_billable`](crate::QualityFlag::is_billable)); `None`
    /// on overflow.
    #[must_use]
    pub fn billable_total(&self) -> Option<Decimal> {
        self.intervals
            .iter()
            .filter(|iv| iv.quality().is_billable())
            .try_fold(Decimal::ZERO, |sum, iv| sum.checked_add(iv.value()))
    }

    /// `[first start, last end)`, or `None` for an empty series.
    #[must_use]
    pub fn span(&self) -> Option<(OffsetDateTime, OffsetDateTime)> {
        Some((self.intervals.first()?.from(), self.intervals.last()?.to()))
    }
}

impl<'a> IntoIterator for &'a Series {
    type Item = &'a MeterInterval;
    type IntoIter = std::slice::Iter<'a, MeterInterval>;

    fn into_iter(self) -> Self::IntoIter {
        self.intervals.iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::series::interval::QualityFlag;
    use rust_decimal::dec;
    use time::Duration;
    use time::macros::{date, datetime};

    fn qh(from: OffsetDateTime) -> MeterInterval {
        MeterInterval::quarter_hour(from, dec!(1), QualityFlag::Measured).unwrap()
    }

    fn series(ivs: Vec<MeterInterval>) -> Result<Series, SeriesError> {
        Series::new(Resolution::QUARTER_HOUR, DayBoundary::Strom, ivs)
    }

    /// Reported as an overlap before the misalignment it also is.
    #[test]
    fn rejects_an_overlap_naming_both_intervals() {
        let t = datetime!(2026-06-01 0:00 UTC);
        let inside = MeterInterval::new(
            t + Duration::minutes(5),
            t + Duration::minutes(20),
            dec!(1),
            QualityFlag::Measured,
        )
        .unwrap();
        assert_eq!(
            series(vec![qh(t), inside]),
            Err(SeriesError::Overlap {
                earlier: t,
                later: t + Duration::minutes(5),
            })
        );
    }

    #[test]
    fn rejects_an_interval_outside_the_supported_years() {
        let t = datetime!(9999-06-01 0:00 UTC);
        assert_eq!(
            series(vec![qh(t)]),
            Err(SeriesError::OutOfRange { from: t })
        );
    }

    #[test]
    fn sorts_and_allows_gaps() {
        let t = datetime!(2026-06-01 0:00 UTC);
        let s = series(vec![qh(t + Duration::hours(1)), qh(t)]).unwrap();
        assert_eq!(s.as_slice()[0].from(), t);
        assert_eq!(s.len(), 2);
        assert_eq!(s.span(), Some((t, t + Duration::minutes(75))));
    }

    #[test]
    fn rejects_duplicates_overlaps_and_misalignment_with_the_instant() {
        let t = datetime!(2026-06-01 0:00 UTC);
        assert_eq!(
            series(vec![qh(t), qh(t)]),
            Err(SeriesError::Duplicate { from: t })
        );
        let odd = qh(t + Duration::minutes(5));
        assert!(matches!(
            series(vec![odd]),
            Err(SeriesError::Misaligned { from, .. }) if from == t + Duration::minutes(5)
        ));
        let hour = MeterInterval::hour(t, dec!(1), QualityFlag::Measured).unwrap();
        assert!(matches!(
            series(vec![hour]),
            Err(SeriesError::Misaligned { .. })
        ));
        let long = MeterInterval::new(t, t + Duration::minutes(30), dec!(1), QualityFlag::Measured)
            .unwrap();
        let inner = qh(t + Duration::minutes(15));
        assert!(Series::new(Resolution::HALF_HOUR, DayBoundary::Strom, vec![long, inner]).is_err());
    }

    #[test]
    fn rejects_mixed_channels() {
        let t = datetime!(2026-06-01 0:00 UTC);
        let a = qh(t).with_obis(ObisCode::STROM_BEZUG_LASTGANG);
        let b = qh(t + Duration::minutes(15)).with_obis(ObisCode::STROM_EINSPEISUNG_LASTGANG);
        assert_eq!(
            series(vec![a.clone(), b]),
            Err(SeriesError::MixedChannels {
                from: t + Duration::minutes(15)
            })
        );
        let s = series(vec![a]).unwrap();
        assert_eq!(s.channel(), Some(ObisCode::STROM_BEZUG_LASTGANG));
        assert_eq!(s.unit(), Some(Unit::KiloWattHour));
    }

    #[test]
    fn a_long_day_is_one_daily_interval() {
        let day = DayBoundary::Strom.day(date!(2026 - 10 - 25)).unwrap();
        let iv =
            MeterInterval::new(day.start(), day.end(), dec!(24), QualityFlag::Measured).unwrap();
        let s = Series::new(Resolution::Day, DayBoundary::Strom, vec![iv]).unwrap();
        assert_eq!(s.len(), 1);
        // The same span is not a Gastag.
        assert!(Series::new(Resolution::Day, DayBoundary::Gas, s.into_intervals()).is_err());
    }

    #[test]
    fn billable_total_excludes_faulty_values() {
        let t = datetime!(2026-06-01 0:00 UTC);
        let s = series(vec![
            qh(t),
            qh(t + Duration::minutes(15)).with_quality(QualityFlag::Faulty),
        ])
        .unwrap();
        assert_eq!(s.billable_total(), Some(dec!(1)));
        assert_eq!(
            s.get(t + Duration::minutes(15)).map(MeterInterval::quality),
            Some(QualityFlag::Faulty)
        );
        assert!(s.get(t + Duration::minutes(30)).is_none());
    }
}
