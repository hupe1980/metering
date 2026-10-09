//! Validation, estimation and editing on a validated [`Series`].
//!
//! The pipeline is three calls:
//!
//! 1. [`validate`](validation::validate) — `&Series` + [`Rules`](validation::Rules)
//!    → a [`Report`](validation::Report) of findings and a grade;
//! 2. [`substitute`](substitute::substitute) — the series, its report and a
//!    [`Policy`](substitute::Policy) → a [`Filled`](substitute::Filled) series in
//!    which every gap and every rejected value is an Ersatzwert with its codes;
//! 3. validate the filled series again, then aggregate or resample it.
//!
//! Structural defects — order, duplicates, overlaps, misaligned intervals,
//! mixed channels — are refused by [`Series::new`] and never reach this module.
//!
//! [`classification`] holds the Messtyp code list.

pub mod classification;
pub mod substitute;
pub mod validation;

use rust_decimal::{Decimal, dec};
use time::{Date, OffsetDateTime, Weekday};

use crate::series::Series;
use crate::series::interval::{MeterInterval, QualityFlag};
use crate::time::calendar::{DayBoundary, Period, shift_back_days, to_berlin};
use crate::time::holiday::Bundesland;
use crate::time::resolution::Resolution;

/// The grid buckets of `resolution` on `boundary` that start in `[from, to)`,
/// beginning with the bucket containing `from`.
pub(crate) fn slots(
    boundary: DayBoundary,
    resolution: Resolution,
    from: OffsetDateTime,
    to: OffsetDateTime,
) -> impl Iterator<Item = Period> {
    let mut next = boundary.bucket(from, resolution);
    std::iter::from_fn(move || {
        let current = next.take()?;
        if current.start() >= to {
            return None;
        }
        next = boundary.bucket(current.end(), resolution);
        Some(current)
    })
}

/// A value the market treats as measured, the only kind a substitute may be formed
/// from or compared with (MeteringCode 2006 A8.2.2.1: checked values without an error status).
pub(crate) const fn is_actual(quality: QualityFlag) -> bool {
    matches!(quality, QualityFlag::Measured | QualityFlag::Corrected)
}

/// Load-relevant day class; a statutory holiday of the Land is a Sunday.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DayClass {
    Werktag,
    Samstag,
    SonnFeiertag,
}

fn day_class(date: Date, land: Bundesland) -> DayClass {
    if date.weekday() == Weekday::Sunday || land.is_holiday(date) {
        DayClass::SonnFeiertag
    } else if date.weekday() == Weekday::Saturday {
        DayClass::Samstag
    } else {
        DayClass::Werktag
    }
}

/// The series plus optional prior data, looked up as one.
pub(crate) struct Data<'a> {
    pub(crate) series: &'a Series,
    pub(crate) prior: Option<&'a Series>,
}

impl Data<'_> {
    pub(crate) fn get(&self, from: OffsetDateTime) -> Option<&MeterInterval> {
        self.series
            .get(from)
            .or_else(|| self.prior.and_then(|p| p.get(from)))
    }

    fn earliest(&self) -> Option<OffsetDateTime> {
        let a = self.series.span().map(|(f, _)| f);
        let b = self.prior.and_then(Series::span).map(|(f, _)| f);
        match (a, b) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }
}

/// The same wall-clock slot on up to `n` previous Berlin days of the same day class,
/// keeping intervals that end at or before `before` and that `usable` accepts.
pub(crate) fn references(
    data: &Data<'_>,
    slot: OffsetDateTime,
    before: OffsetDateTime,
    land: Bundesland,
    n: usize,
    usable: impl Fn(&MeterInterval) -> bool,
) -> Vec<(OffsetDateTime, Decimal)> {
    let mut out = Vec::new();
    let boundary = data.series.boundary();
    let (Some(earliest), Some(day)) = (data.earliest(), boundary.day_of(slot)) else {
        return out;
    };
    let class = day_class(day, land);
    let wall = to_berlin(slot).time();
    let mut back = 1;
    while out.len() < n {
        let Some(candidate) = shift_back_days(slot, back) else {
            break;
        };
        back += 1;
        if candidate < earliest {
            break;
        }
        // A wall time the reference day skipped (spring forward) is no match.
        if to_berlin(candidate).time() != wall {
            continue;
        }
        let Some(iv) = data.get(candidate) else {
            continue;
        };
        let same_class = boundary
            .day_of(candidate)
            .is_some_and(|d| day_class(d, land) == class);
        if iv.to() <= before && same_class && usable(iv) {
            out.push((candidate, iv.value()));
        }
    }
    out
}

/// Scales the median absolute deviation to a Gaussian-equivalent standard
/// deviation, `1 / Φ⁻¹(0.75)`.
const K_MAD: Decimal = dec!(1.4826);

/// Scales the mean absolute deviation around the median to a Gaussian-equivalent
/// standard deviation, `√(π/2)` (Iglewicz & Hoaglin's modified z-score).
const K_MEAN_AD: Decimal = dec!(1.253314);

/// Median of `values`, sorting them. `None` when empty or on overflow.
fn median(values: &mut [Decimal]) -> Option<Decimal> {
    values.sort_unstable();
    let mid = values.len() / 2;
    if values.is_empty() {
        None
    } else if values.len().is_multiple_of(2) {
        values[mid - 1]
            .checked_add(values[mid])?
            .checked_div(Decimal::TWO)
    } else {
        Some(values[mid])
    }
}

/// The Hampel test, in exact arithmetic: `Some(median)` when `x` deviates from
/// the median of `window` by more than `t × max(spread, floor)`.
///
/// `spread` is `K_MAD × MAD`, which a spike cannot raise. A zero MAD (more than
/// half the window on one value, e.g. a two-level load) falls back to
/// `K_MEAN_AD × mean absolute deviation`, so the minority level is no outlier.
/// With a zero spread and a zero floor, any deviation counts.
pub(crate) fn hampel(
    x: Decimal,
    window: &mut [Decimal],
    t: Decimal,
    floor: Decimal,
) -> Option<Decimal> {
    let centre = median(window)?;
    let mut deviations: Vec<Decimal> = window
        .iter()
        .map(|v| v.checked_sub(centre).map(|d| d.abs()))
        .collect::<Option<_>>()?;
    let mad = median(&mut deviations)?;
    let spread = if mad > Decimal::ZERO {
        K_MAD.checked_mul(mad)?
    } else {
        let mut sum = Decimal::ZERO;
        for d in &deviations {
            sum = sum.checked_add(*d)?;
        }
        let n = Decimal::from(deviations.len());
        K_MEAN_AD.checked_mul(sum.checked_div(n)?)?
    };
    let sigma = spread.max(floor);
    let deviation = x.checked_sub(centre)?.abs();
    let outlier = if sigma <= Decimal::ZERO {
        deviation > Decimal::ZERO
    } else {
        deviation > t.checked_mul(sigma)?
    };
    outlier.then_some(centre)
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::{date, datetime};

    #[test]
    fn hampel_finds_the_spike_and_a_floor_quiets_a_flat_window() {
        let mut w = [
            dec!(1),
            dec!(1.1),
            dec!(1),
            dec!(50),
            dec!(1),
            dec!(1.1),
            dec!(1),
        ];
        assert_eq!(hampel(dec!(50), &mut w, dec!(3), dec!(0)), Some(dec!(1)));
        let mut flat = [dec!(1), dec!(1), dec!(1), dec!(1.1), dec!(1)];
        assert!(hampel(dec!(1.1), &mut flat, dec!(3), dec!(0)).is_some());
        assert!(hampel(dec!(1.1), &mut flat, dec!(3), dec!(0.1)).is_none());
    }

    #[test]
    fn a_holiday_is_a_sunday() {
        // Fronleichnam 2026 — a holiday in Bavaria only.
        let d = date!(2026 - 06 - 04);
        assert_eq!(day_class(d, Bundesland::By), DayClass::SonnFeiertag);
        assert_eq!(day_class(d, Bundesland::Be), DayClass::Werktag);
        assert_eq!(
            day_class(date!(2026 - 06 - 06), Bundesland::Be),
            DayClass::Samstag
        );
    }

    #[test]
    fn slots_walk_the_grid_across_a_long_day() {
        let day = DayBoundary::Strom.day(date!(2026 - 10 - 25)).unwrap();
        let n = slots(
            DayBoundary::Strom,
            Resolution::QUARTER_HOUR,
            day.start(),
            day.end(),
        )
        .count();
        assert_eq!(n, 100);
        let first = slots(
            DayBoundary::Strom,
            Resolution::Hour,
            datetime!(2026-06-01 0:30 UTC),
            datetime!(2026-06-01 2:00 UTC),
        )
        .next()
        .unwrap();
        assert_eq!(first.start(), datetime!(2026-06-01 0:00 UTC));
    }
}
