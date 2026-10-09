//! Resampling — a [`Series`] onto a coarser grid of the same calendar.
//!
//! [`resample`] only **sums**; it never interpolates or splits an interval.
//! Day, month and year buckets are Europe/Berlin calendar periods on the
//! series' own [`DayBoundary`](crate::DayBoundary) (the 06:00 Gastag for gas),
//! so a DST day holds 92 or 100 quarter-hours.
//!
//! **A bucket missing any source slot is left out**, so it surfaces as a gap
//! for [`validate`](crate::vee::validation::validate) rather than a short
//! total: substitute first, then resample. A bucket's quality is the worst of
//! its slots ([`QualityFlag::worst_of`](crate::QualityFlag::worst_of)); the
//! channel is kept.
//!
//! ## Regulatory basis
//!
//! - **§ 2 MsbG** — RLM, the 15-minute interval metering these buckets start from.
//! - **GPKE Kap. 8.4** (BNetzA **BK6-24-174**) — Jahresmehr- und
//!   Jahresmindermengen, settled **annually**.
//!
//! ```rust
//! use metering::prelude::*;
//! use time::{Duration, macros::date};
//!
//! let day = DayBoundary::Strom.day(date!(2026 - 10 - 25)).unwrap();
//! let slots = (0..100)
//!     .map(|i| MeterInterval::quarter_hour(day.start() + Duration::minutes(15 * i), dec!(0.25), QualityFlag::Measured))
//!     .collect::<Result<Vec<_>, _>>()?;
//! let series = Series::new(Resolution::QUARTER_HOUR, DayBoundary::Strom, slots)?;
//! let daily = resample(&series, Resolution::Day)?;
//! assert_eq!(daily.len(), 1);
//! assert_eq!(daily.as_slice()[0].value(), dec!(25));
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

use rust_decimal::Decimal;

use crate::series::interval::{MeterInterval, QualityFlag};
use crate::series::{Series, SeriesError};
use crate::time::resolution::Resolution;

/// Why [`resample`] refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum ResampleError {
    /// The target grid is finer than the source, or does not nest it.
    #[error("{from} does not nest into {to}")]
    NotCoarser {
        /// The series' resolution.
        from: Resolution,
        /// The requested resolution.
        to: Resolution,
    },
    /// A bucket total left the `Decimal` range.
    #[error("a bucket total overflows")]
    Overflow,
    /// The buckets did not form a series.
    #[error(transparent)]
    Series(#[from] SeriesError),
}

/// Whether every `to` bucket is a whole number of `from` buckets.
const fn nests(from: Resolution, to: Resolution) -> bool {
    match (from.fixed_seconds(), to.fixed_seconds()) {
        // Fixed steps divide the hour and every day is whole hours.
        (Some(f), Some(t)) => t >= f && t % f == 0,
        (Some(_), None) => true,
        (None, Some(_)) => false,
        (None, None) => matches!(
            (from, to),
            (
                Resolution::Day,
                Resolution::Day | Resolution::Month | Resolution::Year
            ) | (Resolution::Month, Resolution::Month | Resolution::Year)
                | (Resolution::Year, Resolution::Year)
        ),
    }
}

/// Sum `series` into buckets of `target` on the series' calendar.
///
/// # Errors
///
/// [`ResampleError::NotCoarser`] when `target` does not nest the series'
/// grid; [`ResampleError::Overflow`] when a total leaves the `Decimal` range.
pub fn resample(series: &Series, target: Resolution) -> Result<Series, ResampleError> {
    let (boundary, source) = (series.boundary(), series.resolution());
    if !nests(source, target) {
        return Err(ResampleError::NotCoarser {
            from: source,
            to: target,
        });
    }
    let mut out: Vec<MeterInterval> = Vec::new();
    let mut slots = series.iter().peekable();
    while let Some(first) = slots.next() {
        // Every interval is inside the supported years, so it has a bucket.
        let Some(bucket) = boundary.bucket(first.from(), target) else {
            continue;
        };
        let mut members = vec![first];
        while let Some(next) = slots.next_if(|iv| iv.from() < bucket.end()) {
            members.push(next);
        }
        let complete = members.first().map(|iv| iv.from()) == Some(bucket.start())
            && members.windows(2).all(|w| w[0].to() == w[1].from())
            && members.last().map(|iv| iv.to()) == Some(bucket.end());
        if !complete {
            continue;
        }
        let total = members
            .iter()
            .try_fold(Decimal::ZERO, |sum, iv| sum.checked_add(iv.value()))
            .ok_or(ResampleError::Overflow)?;
        let quality = QualityFlag::worst_of(members.iter().map(|iv| iv.quality()));
        if let Ok(iv) = MeterInterval::build(
            bucket.start(),
            bucket.end(),
            total,
            quality,
            series.channel(),
        ) {
            out.push(iv);
        }
    }
    Ok(Series::new(target, boundary, out)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::time::calendar::DayBoundary;
    use rust_decimal::dec;
    use time::macros::{date, datetime};
    use time::{Duration, Month, OffsetDateTime};

    fn qh(start: OffsetDateTime, n: i64, v: Decimal) -> Series {
        let ivs = (0..n)
            .map(|i| {
                MeterInterval::quarter_hour(
                    start + Duration::minutes(15 * i),
                    v,
                    QualityFlag::Measured,
                )
                .unwrap()
            })
            .collect();
        Series::new(Resolution::QUARTER_HOUR, DayBoundary::Strom, ivs).unwrap()
    }

    #[test]
    fn four_quarters_make_an_hour() {
        let s = qh(datetime!(2026-01-01 0:00 UTC), 8, dec!(2.5));
        let h = resample(&s, Resolution::Hour).unwrap();
        assert_eq!(h.len(), 2);
        assert_eq!(h.as_slice()[0].value(), dec!(10));
        assert_eq!(h.resolution(), Resolution::Hour);
    }

    #[test]
    fn the_spring_day_is_92_quarter_hours_and_a_short_one_is_left_out() {
        let day = DayBoundary::Strom.day(date!(2026 - 03 - 29)).unwrap();
        let full = qh(day.start(), 92, dec!(1));
        assert_eq!(
            resample(&full, Resolution::Day).unwrap().as_slice()[0].value(),
            dec!(92)
        );
        let short = qh(day.start(), 91, dec!(1));
        assert!(resample(&short, Resolution::Day).unwrap().is_empty());
    }

    #[test]
    fn a_gas_series_buckets_on_the_gastag() {
        let gastag = DayBoundary::Gas.day(date!(2026 - 01 - 15)).unwrap();
        let ivs = (0..24)
            .map(|i| {
                MeterInterval::hour(
                    gastag.start() + Duration::hours(i),
                    dec!(1),
                    QualityFlag::Measured,
                )
                .unwrap()
            })
            .collect();
        let s = Series::new(Resolution::Hour, DayBoundary::Gas, ivs).unwrap();
        let d = resample(&s, Resolution::Day).unwrap();
        assert_eq!(d.as_slice()[0].from(), gastag.start());
        assert_eq!(d.as_slice()[0].value(), dec!(24));
    }

    #[test]
    fn days_nest_into_months_and_the_worst_quality_wins() {
        let march = DayBoundary::Strom.month(2026, Month::March).unwrap();
        let ivs: Vec<_> = (0..31)
            .map(|d| {
                let p = DayBoundary::Strom
                    .day(date!(2026 - 03 - 01) + Duration::days(d))
                    .unwrap();
                let q = if d == 3 {
                    QualityFlag::Substituted
                } else {
                    QualityFlag::Measured
                };
                MeterInterval::new(p.start(), p.end(), dec!(2), q).unwrap()
            })
            .collect();
        let s = Series::new(Resolution::Day, DayBoundary::Strom, ivs).unwrap();
        let m = resample(&s, Resolution::Month).unwrap();
        assert_eq!(m.as_slice()[0].from(), march.start());
        assert_eq!(m.as_slice()[0].value(), dec!(62));
        assert_eq!(m.as_slice()[0].quality(), QualityFlag::Substituted);
    }

    #[test]
    fn a_finer_or_non_nesting_target_is_refused() {
        let s = qh(datetime!(2026-01-01 0:00 UTC), 4, dec!(1));
        assert!(matches!(
            resample(&s, Resolution::minutes(5).unwrap()),
            Err(ResampleError::NotCoarser { .. })
        ));
        let six = Series::new(Resolution::minutes(6).unwrap(), DayBoundary::Strom, vec![]).unwrap();
        assert!(resample(&six, Resolution::minutes(4).unwrap()).is_err());
        assert!(resample(&six, Resolution::minutes(10).unwrap()).is_err());
        assert!(resample(&six, Resolution::minutes(12).unwrap()).is_ok());
    }
}
