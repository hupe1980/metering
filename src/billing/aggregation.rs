//! Billing period aggregation: Arbeitsmenge, Spitzenleistung, Benutzungsdauer.
//!
//! | Quantity | Source |
//! |---|---|
//! | Jahreshöchstleistung ([`BillingPeriod::spitzenleistung_kw`]) | § 17 Abs. 2 StromNEV — *"Das Jahresleistungsentgelt ist das Produkt aus dem jeweiligen Jahresleistungspreis und der **Jahreshöchstleistung** in Kilowatt der jeweiligen Entnahme im Abrechnungsjahr."* |
//! | Viertelstundenleistung | § 2 MsbG — registrierende Lastgangmessung |
//!
//! Start at [`aggregate`]: one [`Series`] (one grid, one channel) and the
//! [`Period`] billed. Only **billable** intervals inside the period count; the
//! Spitzenleistung is `max(kWh × 3600 ÷ s)` over them, on a grid of an hour or
//! finer and an energy channel only. Import and export are separate series;
//! [`sum_by_direction`] balances them. Tariff registers are
//! [`Zaehlzeitdefinition::split_energy`](crate::billing::zaehlzeit::Zaehlzeitdefinition::split_energy)'s.

use rust_decimal::Decimal;
use time::OffsetDateTime;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use crate::precision::{BENUTZUNGSDAUER_DP, BENUTZUNGSDAUER_STRATEGY};
use crate::series::Series;
use crate::series::interval::MeterInterval;
use crate::time::calendar::Period;
use crate::vee::validation::{Coverage, coverage};

/// A billing period's quantities.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[non_exhaustive]
pub struct BillingPeriod {
    /// The period billed.
    pub period: Period,
    /// Arbeitsmenge — the sum of the billable intervals inside the period, in
    /// the channel's unit.
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::decimal"))]
    pub arbeitsmenge: Decimal,
    /// Spitzenleistung in kW: the highest average power over one billable
    /// interval. `None` on a grid coarser than an hour, on a non-energy
    /// channel, or when nothing billable arrived.
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::decimal_option"))]
    pub spitzenleistung_kw: Option<Decimal>,
    /// The first interval the Spitzenleistung was reached in.
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::rfc3339_option"))]
    pub spitzenleistung_at: Option<OffsetDateTime>,
    /// Intervals inside the period that contributed.
    pub billable_count: usize,
    /// Intervals inside the period excluded as non-billable.
    pub excluded_count: usize,
    /// Intervals not wholly inside the period, ignored.
    pub outside_count: usize,
    /// Billable intervals against the period (unlike the
    /// [`Report`](crate::vee::validation::Report)'s coverage, which counts
    /// every delivered interval).
    pub coverage: Coverage,
}

/// The Arbeitsmenge left the `Decimal` range.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("the Arbeitsmenge overflows")]
#[non_exhaustive]
pub struct Overflow;

/// The day StromNEV ceases to be in force: **31.12.2028**.
///
/// *"Die V tritt gem. Art. 15 Abs. 3 G v. 22.12.2023 I Nr. 405 mit Ablauf des
/// 31.12.2028 außer Kraft"* (consolidated StromNEV).
///
/// The quantities stay correct; what lapses is the **statutory basis for
/// billing on them** (§ 17 Abs. 1, § 17 Abs. 2, Anlage 4). From 01.01.2029 the
/// BNetzA's *Allgemeine Netzentgeltsystematik Strom* applies. No function
/// gates on this date.
pub const STROMNEV_AUSSERKRAFT: time::Date =
    match time::Date::from_calendar_date(2028, time::Month::December, 31) {
        Ok(d) => d,
        Err(_) => unreachable!(),
    };

/// Aggregate the billable intervals of `series` inside `period`.
///
/// A tied peak reports the earliest interval.
///
/// # Errors
///
/// [`Overflow`] when the sum leaves the `Decimal` range.
///
/// ```rust
/// use metering::prelude::*;
/// use time::{Duration, macros::date};
///
/// let day = DayBoundary::Strom.day(date!(2026 - 06 - 01)).unwrap();
/// // The evening's last six hours never arrived.
/// let slots = (0..72)
///     .map(|i| MeterInterval::quarter_hour(day.start() + Duration::minutes(15 * i), dec!(0.5), QualityFlag::Measured))
///     .collect::<Result<Vec<_>, _>>()?;
/// let series = Series::new(Resolution::QUARTER_HOUR, DayBoundary::Strom, slots)?;
/// let period = aggregate(&series, day)?;
/// assert_eq!(period.arbeitsmenge, dec!(36));
/// assert_eq!(period.spitzenleistung_kw, Some(dec!(2)));
/// assert_eq!(period.coverage.pct(), Some(dec!(75)));
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub fn aggregate(series: &Series, period: Period) -> Result<BillingPeriod, Overflow> {
    let peaks = series
        .resolution()
        .fixed_seconds()
        .is_some_and(|s| s <= 3600);
    let inside = |iv: &MeterInterval| period.start() <= iv.from() && iv.to() <= period.end();
    let mut out = BillingPeriod {
        period,
        arbeitsmenge: Decimal::ZERO,
        spitzenleistung_kw: None,
        spitzenleistung_at: None,
        billable_count: 0,
        excluded_count: 0,
        outside_count: 0,
        coverage: coverage(series, &period, |iv| {
            iv.quality().is_billable() && inside(iv)
        }),
    };
    for iv in series {
        if !inside(iv) {
            out.outside_count += 1;
            continue;
        }
        if !iv.quality().is_billable() {
            out.excluded_count += 1;
            continue;
        }
        out.billable_count += 1;
        out.arbeitsmenge = out.arbeitsmenge.checked_add(iv.value()).ok_or(Overflow)?;
        // Ascending order: a strictly higher power wins, so a tie keeps the earliest.
        if peaks
            && let Some(kw) = iv.demand_kw()
            && out.spitzenleistung_kw.is_none_or(|best| kw > best)
        {
            out.spitzenleistung_kw = Some(kw);
            out.spitzenleistung_at = Some(iv.from());
        }
    }
    Ok(out)
}

// ── directional balance ───────────────────────────────────────────────────────

/// The energy that crossed a measurement point, split by
/// [`Direction`](crate::Direction).
///
/// An interval without a direction (no OBIS code, a reactive register, a gas
/// volume) lands in [`undirected`](Self::undirected), so
/// `import + export + undirected` is always the plain sum of the input.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct DirectionalEnergy {
    /// Bezug — the sum over intervals whose code counts C = 1.
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::decimal"))]
    pub import: Decimal,
    /// Einspeisung — the sum over intervals whose code counts C = 2.
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::decimal"))]
    pub export: Decimal,
    /// Everything with no direction to read.
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::decimal"))]
    pub undirected: Decimal,
}

impl BillingPeriod {
    /// Benutzungsstundenzahl — `Arbeitsmenge ÷ Spitzenleistung`, in hours.
    ///
    /// § 17 Abs. 1 StromNEV makes the Netzentgelt depend on *"der jeweiligen
    /// **Benutzungsstundenzahl** der Entnahmestelle"*; Anlage 4 zu § 17 Abs. 2
    /// builds the Gleichzeitigkeitsgrad on two lines meeting *"durch die
    /// Jahresbenutzungsdauer 2 500 Stunden"*, so the threshold applies to a
    /// **year**. Basis lapses with
    /// [`STROMNEV_AUSSERKRAFT`]. Rounded to [`BENUTZUNGSDAUER_DP`] places.
    ///
    /// `None` when there is no positive Spitzenleistung (grid coarser than an
    /// hour, non-energy channel, nothing billable, or a zero peak).
    ///
    /// ```rust
    /// use metering::prelude::*;
    /// use time::{Duration, macros::date};
    ///
    /// // A flat 4 kW draw for a day: 96 kWh against a 4 kW peak is 24 hours.
    /// let day = DayBoundary::Strom.day(date!(2026 - 06 - 01)).unwrap();
    /// let slots = (0..96)
    ///     .map(|i| MeterInterval::quarter_hour(day.start() + Duration::minutes(15 * i), dec!(1), QualityFlag::Measured))
    ///     .collect::<Result<Vec<_>, _>>()?;
    /// let period = aggregate(&Series::new(Resolution::QUARTER_HOUR, DayBoundary::Strom, slots)?, day)?;
    /// assert_eq!(period.spitzenleistung_kw, Some(dec!(4)));
    /// assert_eq!(period.benutzungsdauer_h(), Some(dec!(24)));
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    #[must_use]
    pub fn benutzungsdauer_h(&self) -> Option<Decimal> {
        let peak = self.spitzenleistung_kw?;
        if peak <= Decimal::ZERO {
            return None;
        }
        Some(
            self.arbeitsmenge
                .checked_div(peak)?
                .round_dp_with_strategy(BENUTZUNGSDAUER_DP, BENUTZUNGSDAUER_STRATEGY),
        )
    }
}

impl DirectionalEnergy {
    /// `import − export`: positive when the point drew more than it fed back.
    /// Excludes [`undirected`](Self::undirected).
    #[must_use]
    pub fn net(&self) -> Decimal {
        self.import - self.export
    }

    /// `import + export + undirected` — the plain sum of everything counted.
    #[must_use]
    pub fn total(&self) -> Decimal {
        self.import + self.export + self.undirected
    }
}

/// Sum a series by flow direction.
///
/// The conservation check for a bidirectional point (V2G, battery, PV behind
/// the grid meter): an allocation is correct only if both sides balance.
///
/// ```rust
/// use metering::{MeterInterval, billing::aggregation::sum_by_direction};
/// use rust_decimal::dec;
/// use time::macros::datetime;
///
/// let iv = |code: &str, kwh| MeterInterval::quarter_hour(datetime!(2026-06-01 12:00 UTC), kwh, metering::QualityFlag::Measured).unwrap()
///     .with_obis(code.parse().unwrap());
///
/// let grid = [iv("1-0:1.8.0", dec!(9)), iv("1-0:2.8.0", dec!(4))];
/// let allocated = [
///     iv("1-0:1.8.0", dec!(5)), iv("1-0:1.8.0", dec!(4)),   // two sessions
///     iv("1-0:2.8.0", dec!(4)),                             // one discharge
/// ];
///
/// let measured = sum_by_direction(&grid);
/// let split = sum_by_direction(&allocated);
///
/// assert_eq!(measured.import - split.import, dec!(0));
/// assert_eq!(measured.export - split.export, dec!(0));
/// assert_eq!(measured.net(), dec!(5));
/// ```
///
/// A **physical** balance: unlike [`aggregate`], every interval counts,
/// billable or not.
#[must_use]
pub fn sum_by_direction(intervals: &[MeterInterval]) -> DirectionalEnergy {
    let mut out = DirectionalEnergy::default();
    for iv in intervals {
        match iv.direction() {
            Some(crate::series::interval::Direction::Import) => out.import += iv.value(),
            Some(crate::series::interval::Direction::Export) => out.export += iv.value(),
            None => out.undirected += iv.value(),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::series::interval::QualityFlag;
    use crate::time::calendar::DayBoundary;
    use crate::time::resolution::Resolution;
    use rust_decimal::dec;
    use time::Duration;
    use time::macros::date;

    fn day() -> Period {
        DayBoundary::Strom.day(date!(2026 - 06 - 01)).unwrap()
    }

    fn series(values: &[(i64, Decimal, QualityFlag)]) -> Series {
        let ivs = values
            .iter()
            .map(|(i, v, q)| {
                MeterInterval::quarter_hour(day().start() + Duration::minutes(15 * i), *v, *q)
                    .unwrap()
            })
            .collect();
        Series::new(Resolution::QUARTER_HOUR, DayBoundary::Strom, ivs).unwrap()
    }

    #[test]
    fn the_stromnev_basis_has_a_stated_end_date() {
        assert_eq!(STROMNEV_AUSSERKRAFT, date!(2028 - 12 - 31));
    }

    #[test]
    fn only_billable_values_inside_the_period_count_and_the_first_peak_wins() {
        let m = QualityFlag::Measured;
        let s = series(&[
            (0, dec!(5), m),
            (1, dec!(9), QualityFlag::Faulty),
            (2, dec!(5), m),
            (96, dec!(7), m), // the next day
        ]);
        let p = aggregate(&s, day()).unwrap();
        assert_eq!(p.arbeitsmenge, dec!(10));
        assert_eq!(
            (p.billable_count, p.excluded_count, p.outside_count),
            (2, 1, 1)
        );
        assert_eq!(p.spitzenleistung_kw, Some(dec!(20)));
        assert_eq!(p.spitzenleistung_at, Some(day().start()));
        assert_eq!(p.benutzungsdauer_h(), Some(dec!(0.5)));
    }

    /// Coverage counts billable time inside the period only, so it never
    /// exceeds 100 % — out-of-period data cannot push it over.
    #[test]
    fn coverage_is_clipped_to_the_period() {
        let m = QualityFlag::Measured;
        let s = series(&(0..200).map(|i| (i, dec!(1), m)).collect::<Vec<_>>());
        let p = aggregate(&s, day()).unwrap();
        assert_eq!(p.coverage.pct(), Some(dec!(100)));
        assert!(p.coverage.is_complete());
        assert_eq!(p.outside_count, 104);
    }

    #[test]
    fn a_daily_series_has_no_spitzenleistung() {
        let iv = MeterInterval::new(day().start(), day().end(), dec!(24), QualityFlag::Measured)
            .unwrap();
        let s = Series::new(Resolution::Day, DayBoundary::Strom, vec![iv]).unwrap();
        let p = aggregate(&s, day()).unwrap();
        assert_eq!(p.arbeitsmenge, dec!(24));
        assert_eq!(p.spitzenleistung_kw, None);
        assert_eq!(p.benutzungsdauer_h(), None);
    }

    #[test]
    fn an_empty_series_is_zero_with_zero_coverage() {
        let s = series(&[]);
        let p = aggregate(&s, day()).unwrap();
        assert_eq!(p.arbeitsmenge, Decimal::ZERO);
        assert_eq!(p.coverage.pct(), Some(dec!(0)));
    }

    #[test]
    fn overflow_is_an_error() {
        let m = QualityFlag::Measured;
        let s = series(&[(0, Decimal::MAX, m), (1, Decimal::MAX, m)]);
        assert_eq!(aggregate(&s, day()), Err(Overflow));
    }
}
