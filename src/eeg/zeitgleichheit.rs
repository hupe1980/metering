//! EnFG § 46 — Zeitgleichheit and the statutory worst-case estimate.
//!
//! **Abs. 5 Satz 1.** Withdrawn and self-consumed electricity may count
//! *"höchstens bis zu der Höhe der tatsächlichen Netzentnahme, bezogen auf
//! jedes 15-Minuten-Intervall (Zeitgleichheit von Netzentnahme und
//! Verbrauch)"*. [`zeitgleichheit`] takes the per-quarter-hour minimum, then
//! sums: `Σ min(a, b) ≤ min(Σ a, Σ b)`.
//!
//! **Abs. 3 Satz 4.** Where the quantities are estimated, the requirement
//! that no less Umlage is paid than under measurement *"ist insbesondere
//! erfüllt, wenn"* the maximum power draw of the device *"mit der Summe der
//! vollen Zeitstunden des jeweiligen Kalenderjahres multipliziert wird"* —
//! [`worst_case_estimate`], over the Berlin calendar year's own hours.
//!
//! Umlage amounts are money and stay out.
//!
//! ```rust
//! use metering::eeg::zeitgleichheit::zeitgleichheit;
//! use metering::prelude::*;
//! use time::{Duration, macros::datetime};
//!
//! let t = datetime!(2026-06-01 0:00 UTC);
//! let series = |values: [Decimal; 2]| -> Result<Series, Box<dyn std::error::Error>> {
//!     let slots = values.iter().enumerate().map(|(i, v)| {
//!         MeterInterval::quarter_hour(t + Duration::minutes(15 * i as i64), *v, QualityFlag::Measured)
//!     });
//!     let slots = slots.collect::<Result<Vec<_>, _>>()?;
//!     Ok(Series::new(Resolution::QUARTER_HOUR, DayBoundary::Strom, slots)?)
//! };
//! let consumption = series([dec!(5), dec!(1)])?;
//! let withdrawal = series([dec!(2), dec!(4)])?;
//! let z = zeitgleichheit(&consumption, &withdrawal)?;
//! // min(5, 2) + min(1, 4) = 3, not min(6, 6) = 6.
//! assert_eq!(z.total, dec!(3));
//! assert_eq!(z.intervals[0].counted, dec!(2));
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

use rust_decimal::Decimal;
use time::OffsetDateTime;

use super::{EegError, align, coincident, sum};
use crate::series::Series;
use crate::series::interval::QualityFlag;
use crate::time::calendar::DayBoundary;

/// One quarter-hour of [`Zeitgleichheit`]: both inputs and the counted share.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct CoincidentInterval {
    /// Quarter-hour start (UTC, inclusive).
    pub from: OffsetDateTime,
    /// Quarter-hour end (UTC, exclusive).
    pub to: OffsetDateTime,
    /// The consumption claimed for the privilege, kWh.
    pub consumption: Decimal,
    /// The grid withdrawal, kWh.
    pub withdrawal: Decimal,
    /// `min(consumption, withdrawal)` — what counts, kWh.
    pub counted: Decimal,
    /// The worse of the two input qualities; non-measured values are counted
    /// and reported, not hidden.
    pub quality: QualityFlag,
}

/// EnFG § 46 Abs. 5: the per-quarter-hour coincident quantity and its sum.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Zeitgleichheit {
    /// Every quarter-hour, ascending.
    pub intervals: Vec<CoincidentInterval>,
    /// `Σ counted`, kWh.
    pub total: Decimal,
}

impl Zeitgleichheit {
    /// The quarter-hours whose inputs were not both measured.
    pub fn not_measured(&self) -> impl Iterator<Item = &CoincidentInterval> {
        self.intervals
            .iter()
            .filter(|iv| iv.quality != QualityFlag::Measured)
    }
}

/// EnFG § 46 Abs. 5 Satz 1: consumption counted at most up to the grid
/// withdrawal, per 15-minute interval, then summed.
///
/// # Errors
///
/// [`EegError`] when a series is not quarter-hourly, the two grids differ
/// (named by instant), a value is negative or non-billable, or on overflow.
pub fn zeitgleichheit(
    consumption: &Series,
    withdrawal: &Series,
) -> Result<Zeitgleichheit, EegError> {
    let rows = align(&[("consumption", consumption), ("withdrawal", withdrawal)])?;
    let intervals: Vec<CoincidentInterval> = rows
        .into_iter()
        .map(|r| CoincidentInterval {
            from: r.from,
            to: r.to,
            consumption: r.values[0],
            withdrawal: r.values[1],
            counted: coincident(r.values[0], r.values[1]),
            quality: r.quality,
        })
        .collect();
    let total = sum(intervals.iter().map(|iv| iv.counted))?;
    Ok(Zeitgleichheit { intervals, total })
}

/// The EnFG § 46 Abs. 3 Satz 4 estimate, with the hours it used.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct WorstCaseEstimate {
    /// The full hours of the Berlin calendar year (8 760; 8 784 in a leap
    /// year — the DST hours cancel).
    pub hours: u32,
    /// `maximum power × hours`, kWh.
    pub kwh: Decimal,
}

/// EnFG § 46 Abs. 3 Satz 4: the maximum power draw of a device times the
/// full hours of the calendar year, in kWh.
///
/// The estimate for the quantity at the highest Umlage rate; always a whole
/// calendar year. `None` outside the supported years, for a negative power,
/// and on overflow.
///
/// ```rust
/// use metering::eeg::zeitgleichheit::worst_case_estimate;
/// use rust_decimal::dec;
///
/// let e = worst_case_estimate(dec!(2.5), 2028).unwrap();
/// assert_eq!(e.hours, 8_784);
/// assert_eq!(e.kwh, dec!(21960.0));
/// ```
#[must_use]
pub fn worst_case_estimate(max_power_kw: Decimal, year: i32) -> Option<WorstCaseEstimate> {
    if max_power_kw < Decimal::ZERO {
        return None;
    }
    let period = DayBoundary::Strom.year(year)?;
    let hours = u32::try_from(period.duration().whole_hours()).ok()?;
    let kwh = max_power_kw.checked_mul(Decimal::from(hours))?;
    Some(WorstCaseEstimate { hours, kwh })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::series::interval::MeterInterval;
    use crate::time::resolution::Resolution;
    use rust_decimal::dec;
    use time::Duration;
    use time::macros::datetime;

    fn series(values: &[Decimal]) -> Series {
        let t = datetime!(2026-03-29 0:00 UTC);
        let ivs = values
            .iter()
            .enumerate()
            .map(|(i, &v)| {
                MeterInterval::quarter_hour(
                    t + Duration::minutes(15 * i as i64),
                    v,
                    QualityFlag::Measured,
                )
                .unwrap()
            })
            .collect();
        Series::new(Resolution::QUARTER_HOUR, DayBoundary::Strom, ivs).unwrap()
    }

    #[test]
    fn minimum_per_interval_before_the_sum() {
        let c = series(&[dec!(5), dec!(0), dec!(3)]);
        let w = series(&[dec!(1), dec!(4), dec!(3)]);
        let z = zeitgleichheit(&c, &w).unwrap();
        assert_eq!(z.total, dec!(4));
        let min_of_sums = dec!(8).min(dec!(8));
        assert_ne!(z.total, min_of_sums);
        assert_eq!(
            z.intervals.iter().map(|i| i.counted).collect::<Vec<_>>(),
            [dec!(1), dec!(0), dec!(3)]
        );
    }

    #[test]
    fn substituted_inputs_are_reported() {
        let c = series(&[dec!(1), dec!(1)]);
        let t = datetime!(2026-03-29 0:00 UTC);
        let w = Series::new(
            Resolution::QUARTER_HOUR,
            DayBoundary::Strom,
            vec![
                MeterInterval::quarter_hour(t, dec!(1), QualityFlag::Measured).unwrap(),
                MeterInterval::quarter_hour(
                    t + Duration::minutes(15),
                    dec!(1),
                    QualityFlag::Substituted,
                )
                .unwrap(),
            ],
        )
        .unwrap();
        let z = zeitgleichheit(&c, &w).unwrap();
        assert_eq!(z.total, dec!(2));
        let flagged: Vec<_> = z.not_measured().map(|i| i.from).collect();
        assert_eq!(flagged, [t + Duration::minutes(15)]);
    }

    #[test]
    fn the_estimate_uses_the_calendar_years_hours() {
        assert_eq!(worst_case_estimate(dec!(1), 2026).unwrap().hours, 8_760);
        assert_eq!(worst_case_estimate(dec!(1), 2028).unwrap().hours, 8_784);
        assert_eq!(
            worst_case_estimate(dec!(10), 2027).unwrap().kwh,
            dec!(87600)
        );
        assert!(worst_case_estimate(dec!(-1), 2026).is_none());
        assert!(worst_case_estimate(dec!(1), 1800).is_none());
    }
}
