//! Jahresprognose — projecting a full year from a partial one, and scoring a
//! forecast against what was metered.
//!
//! - [`project_annual_consumption`]: observed daily rate × target-year length,
//!   optionally corrected by the **prior year's** seasonal shape (RLM, iMSys).
//! - [`project_annual_slp`]: observation weighted by the delivery point's
//!   **Standardlastprofil** (SLP delivery points).
//! - [`wape`], [`mase`], [`annual_energy_error`]: accuracy; MASE after
//!   Hyndman & Koehler (2006), *Another look at measures of forecast
//!   accuracy*, Int. J. Forecasting 22(4).
//!
//! Neither projection fills gaps — a missing slot lowers the coverage, never
//! the rate; gap filling is [`crate::vee::substitute`]. No prediction
//! interval: daily sums are autocorrelated, so one would overstate confidence.

use rust_decimal::Decimal;
use time::{Date, OffsetDateTime};

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use crate::precision::{
    FORECAST_ACCURACY_DP, FORECAST_ACCURACY_STRATEGY, FORECAST_DP, FORECAST_STRATEGY,
    SEASONAL_FACTOR_DP, SEASONAL_FACTOR_STRATEGY,
};
use crate::series::Series;
use crate::series::interval::MeterInterval;
use crate::slp::strom::DynamicSlpProfile;
use crate::time::calendar::DayBoundary;
use crate::time::holiday::SlpCalendar;
use crate::time::resolution::Resolution;

/// The shortest observation window that yields a projection: **7** Berlin
/// calendar days (below a week the weekday mix dominates the mean).
pub const MIN_OBSERVATION_DAYS: u32 = 7;

/// The least share of the window's slots that must carry billable data:
/// **90 %** of the expected slot count.
///
/// The crate's own floor; no rule publishes one. Below it the answer is
/// [`ForecastError::Coverage`]; fill gaps first ([`crate::vee::substitute`]).
pub const MIN_COVERAGE_PERCENT: u32 = 90;

/// Why a projection or an accuracy score was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum ForecastError {
    /// The series holds no billable interval.
    #[error("the series holds no billable interval")]
    Empty,
    /// The resolution cannot be projected: coarser than a day, or (SLP
    /// projection) not a whole number of quarter-hours.
    #[error("resolution {0} cannot be projected")]
    Resolution(Resolution),
    /// The window spans fewer than [`MIN_OBSERVATION_DAYS`].
    #[error("the window spans {days} days; at least {MIN_OBSERVATION_DAYS} are needed")]
    TooShort {
        /// Berlin calendar days the window spans.
        days: u32,
    },
    /// Fewer than [`MIN_COVERAGE_PERCENT`] of the window's slots are billable.
    #[error("{billable} of {expected} slots are billable, below {MIN_COVERAGE_PERCENT} %")]
    Coverage {
        /// Billable intervals in the window.
        billable: u32,
        /// Slots the window holds at the series' resolution.
        expected: u32,
    },
    /// The prior-year series cannot support a seasonal factor: it does not
    /// cover the shifted window, or it holds no energy there or overall.
    #[error("the prior year cannot support a seasonal factor")]
    PriorYear,
    /// The profile has no value for a slot of the observation or the year.
    #[error("the load profile has no value at {0}")]
    ProfileMissing(OffsetDateTime),
    /// The profile's sum over the observed slots is not positive.
    #[error("the load profile's share of the observed slots is not positive")]
    ProfileZero,
    /// An instant lies outside the supported years.
    #[error("an instant lies outside the supported calendar years")]
    Calendar,
    /// An intermediate does not fit a `Decimal`.
    #[error("the arithmetic overflows")]
    Overflow,
}

/// A projected annual consumption, with the figures that produced it.
///
/// ```text
/// daily_average = observed × expected_slots ÷ (observed_days × billable_slots)
/// annual        = daily_average × days_in_target_year ÷ seasonal_factor
/// ```
///
/// The first line corrects for coverage; a factor above 1 (a heavy window)
/// scales the year **down**. The daily average is rounded to [`FORECAST_DP`]
/// before scaling, so the projection re-derives from the stated figures
/// (MessEV § 25 Nr. 7: the method *and the values used*).
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[non_exhaustive]
pub struct AnnualForecast {
    /// First Berlin calendar day of the observation window.
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::iso_date"))]
    pub first_day: Date,
    /// Last Berlin calendar day of the observation window (inclusive).
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::iso_date"))]
    pub last_day: Date,
    /// Billable energy observed (kWh).
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::decimal"))]
    pub observed_kwh: Decimal,
    /// Calendar days the window spans.
    pub observed_days: u32,
    /// Billable intervals in the window.
    pub billable_slots: u32,
    /// Slots the window holds at the series' resolution.
    pub expected_slots: u32,
    /// Coverage-corrected mean daily consumption (kWh/day), rounded to
    /// [`FORECAST_DP`].
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::decimal"))]
    pub daily_average_kwh: Decimal,
    /// Days in the target year — the Berlin year the window ends in.
    pub target_year_days: u16,
    /// The seasonal factor divided out, rounded to [`SEASONAL_FACTOR_DP`];
    /// `None` when no prior year was supplied.
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::decimal_option"))]
    pub seasonal_factor: Option<Decimal>,
    /// Projected annual consumption (kWh), rounded to [`FORECAST_DP`].
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::decimal"))]
    pub projected_annual_kwh: Decimal,
}

/// Project annual consumption from a partial year.
///
/// The window is the run of whole calendar days (on the series'
/// [`DayBoundary`]) from the first billable interval's day to the last one's.
/// Its coverage-corrected rate ([`AnnualForecast`]) is scaled to the **real
/// length of the target year** (366 in a leap year).
///
/// With `prior_year`, the seasonal factor is the prior year's
/// coverage-corrected daily rate over the same window one year earlier,
/// divided by its rate over its whole span.
///
/// # Errors
///
/// [`ForecastError`] when the series is empty, coarser than a day, shorter
/// than [`MIN_OBSERVATION_DAYS`], below [`MIN_COVERAGE_PERCENT`] (the
/// observation or the prior year's shifted window), or when the prior year
/// cannot support a factor.
///
/// ```rust
/// use metering::billing::forecast::project_annual_consumption;
/// use metering::prelude::*;
/// use time::{Duration, macros::date};
///
/// // Fourteen days at 1 kWh per quarter-hour = 96 kWh/day.
/// let base = DayBoundary::Strom.day(date!(2026 - 01 - 01)).unwrap().start();
/// let slots = (0..14 * 96)
///     .map(|i| MeterInterval::quarter_hour(base + Duration::minutes(15 * i), dec!(1), QualityFlag::Measured))
///     .collect::<Result<Vec<_>, _>>()?;
/// let series = Series::new(Resolution::QUARTER_HOUR, DayBoundary::Strom, slots)?;
///
/// let f = project_annual_consumption(&series, None)?;
/// assert_eq!(f.observed_days, 14);
/// assert_eq!(f.projected_annual_kwh, dec!(96) * dec!(365));
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub fn project_annual_consumption(
    series: &Series,
    prior_year: Option<&Series>,
) -> Result<AnnualForecast, ForecastError> {
    let window = Window::of(series)?;
    if window.days < MIN_OBSERVATION_DAYS {
        return Err(ForecastError::TooShort { days: window.days });
    }
    let observed = window.measure(series)?;
    observed.require_coverage()?;

    let daily_average = observed
        .daily_rate()?
        .round_dp_with_strategy(FORECAST_DP, FORECAST_STRATEGY);

    let target_year = window.last_day.year();
    let target_year_days = time::util::days_in_year(target_year);
    let annual = daily_average
        .checked_mul(Decimal::from(target_year_days))
        .ok_or(ForecastError::Overflow)?;

    let seasonal_factor = prior_year
        .map(|prior| seasonal_factor(&window, prior))
        .transpose()?;
    let projected = match seasonal_factor {
        Some(factor) => annual.checked_div(factor).ok_or(ForecastError::PriorYear)?,
        None => annual,
    };

    Ok(AnnualForecast {
        first_day: window.first_day,
        last_day: window.last_day,
        observed_kwh: observed.energy,
        observed_days: window.days,
        billable_slots: observed.billable,
        expected_slots: observed.expected,
        daily_average_kwh: daily_average,
        target_year_days,
        seasonal_factor,
        projected_annual_kwh: projected.round_dp_with_strategy(FORECAST_DP, FORECAST_STRATEGY),
    })
}

/// The prior year's rate over the shifted window ÷ its overall rate, both
/// coverage-corrected, rounded to [`SEASONAL_FACTOR_DP`].
fn seasonal_factor(window: &Window, prior: &Series) -> Result<Decimal, ForecastError> {
    let boundary = prior.boundary();
    let shift = |d: Date| -> Result<Date, ForecastError> {
        let start = boundary.day(d).ok_or(ForecastError::Calendar)?.start();
        let back =
            crate::time::calendar::shift_back_one_year(start).ok_or(ForecastError::Calendar)?;
        boundary.day_of(back).ok_or(ForecastError::Calendar)
    };
    let shifted = Window::new(
        boundary,
        prior.resolution(),
        shift(window.first_day)?,
        shift(window.last_day)?,
    )?;
    let in_window = shifted.measure(prior)?;
    in_window.require_coverage()?;
    let reference = Window::of(prior)?.measure(prior)?;

    let window_rate = in_window.daily_rate()?;
    let reference_rate = reference.daily_rate()?;
    if window_rate <= Decimal::ZERO || reference_rate <= Decimal::ZERO {
        return Err(ForecastError::PriorYear);
    }
    Ok(window_rate
        .checked_div(reference_rate)
        .ok_or(ForecastError::Overflow)?
        .round_dp_with_strategy(SEASONAL_FACTOR_DP, SEASONAL_FACTOR_STRATEGY))
}

/// A run of whole calendar days on one boundary, at one resolution.
struct Window {
    boundary: DayBoundary,
    resolution: Resolution,
    first_day: Date,
    last_day: Date,
    days: u32,
}

/// What a series holds inside a [`Window`].
struct Measured {
    energy: Decimal,
    billable: u32,
    expected: u32,
    days: u32,
}

impl Window {
    fn new(
        boundary: DayBoundary,
        resolution: Resolution,
        first_day: Date,
        last_day: Date,
    ) -> Result<Self, ForecastError> {
        if matches!(resolution, Resolution::Month | Resolution::Year) {
            return Err(ForecastError::Resolution(resolution));
        }
        let days = u32::try_from((last_day - first_day).whole_days() + 1)
            .map_err(|_| ForecastError::Calendar)?;
        Ok(Self {
            boundary,
            resolution,
            first_day,
            last_day,
            days,
        })
    }

    /// The whole days from the series' first billable interval to its last.
    fn of(series: &Series) -> Result<Self, ForecastError> {
        let boundary = series.boundary();
        let mut billable = series.iter().filter(|iv| iv.quality().is_billable());
        let first = billable.next().ok_or(ForecastError::Empty)?;
        let last = billable.next_back().unwrap_or(first);
        let day = |iv: &MeterInterval| boundary.day_of(iv.from()).ok_or(ForecastError::Calendar);
        Self::new(boundary, series.resolution(), day(first)?, day(last)?)
    }

    fn start(&self) -> Result<OffsetDateTime, ForecastError> {
        Ok(self
            .boundary
            .day(self.first_day)
            .ok_or(ForecastError::Calendar)?
            .start())
    }

    fn end(&self) -> Result<OffsetDateTime, ForecastError> {
        Ok(self
            .boundary
            .day(self.last_day)
            .ok_or(ForecastError::Calendar)?
            .end())
    }

    fn measure(&self, series: &Series) -> Result<Measured, ForecastError> {
        let (start, end) = (self.start()?, self.end()?);
        let mut expected = 0u32;
        let mut day = self.first_day;
        loop {
            let slots = self
                .boundary
                .day(day)
                .and_then(|p| p.count(self.resolution))
                .ok_or(ForecastError::Calendar)?;
            expected = expected.checked_add(slots).ok_or(ForecastError::Overflow)?;
            if day == self.last_day {
                break;
            }
            day = day.next_day().ok_or(ForecastError::Calendar)?;
        }
        let mut energy = Decimal::ZERO;
        let mut billable = 0u32;
        for iv in series
            .iter()
            .filter(|iv| iv.from() >= start && iv.to() <= end && iv.quality().is_billable())
        {
            energy = energy
                .checked_add(iv.value())
                .ok_or(ForecastError::Overflow)?;
            billable += 1;
        }
        Ok(Measured {
            energy,
            billable,
            expected,
            days: self.days,
        })
    }
}

impl Measured {
    /// `billable × 100 ≥ expected × MIN_COVERAGE_PERCENT`, in integers.
    fn require_coverage(&self) -> Result<(), ForecastError> {
        let lhs = u64::from(self.billable) * 100;
        let rhs = u64::from(self.expected) * u64::from(MIN_COVERAGE_PERCENT);
        if self.billable == 0 || lhs < rhs {
            return Err(ForecastError::Coverage {
                billable: self.billable,
                expected: self.expected,
            });
        }
        Ok(())
    }

    /// `energy × expected ÷ (days × billable)` — one quotient.
    fn daily_rate(&self) -> Result<Decimal, ForecastError> {
        if self.billable == 0 {
            return Err(ForecastError::Empty);
        }
        let numerator = self
            .energy
            .checked_mul(Decimal::from(self.expected))
            .ok_or(ForecastError::Overflow)?;
        let denominator = Decimal::from(u64::from(self.days) * u64::from(self.billable));
        numerator
            .checked_div(denominator)
            .ok_or(ForecastError::Overflow)
    }
}

/// An SLP-weighted annual projection, with the sums it rests on.
///
/// ```text
/// annual = observed × Σ profile(target year) ÷ Σ profile(observed slots)
/// ```
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[non_exhaustive]
pub struct SlpProjection {
    /// Billable energy observed (kWh).
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::decimal"))]
    pub observed_kwh: Decimal,
    /// The profile summed over the quarter-hours of the billable intervals.
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::decimal"))]
    pub profile_observed: Decimal,
    /// The profile summed over every quarter-hour of the target year.
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::decimal"))]
    pub profile_year: Decimal,
    /// The Berlin calendar year of the last billable interval.
    pub target_year: i32,
    /// Projected annual consumption (kWh), rounded to [`FORECAST_DP`].
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::decimal"))]
    pub projected_annual_kwh: Decimal,
}

/// Project annual consumption for an **SLP delivery point** from its own
/// Standardlastprofil.
///
/// The profile's sum is taken over the **billable** slots only, so a missing
/// slot is missing from both sides. The resolution is a whole number of
/// quarter-hours or a day; each billable interval weighs the profile over the
/// quarter-hours it spans ([`DynamicSlpProfile::value_at`]). The target year is
/// the Berlin year of the last billable interval.
///
/// # Errors
///
/// [`ForecastError`] when the series is empty, its resolution is not a
/// quarter-hour multiple or a day, the profile lacks a value for a needed
/// slot, or its observed sum is not positive.
pub fn project_annual_slp(
    series: &Series,
    profile: &DynamicSlpProfile,
    calendar: &SlpCalendar,
) -> Result<SlpProjection, ForecastError> {
    let resolution = series.resolution();
    let weighable = match resolution {
        Resolution::Day => true,
        r => r
            .fixed_seconds()
            .is_some_and(|s| s.is_multiple_of(QUARTER_HOUR_SECS)),
    };
    if !weighable {
        return Err(ForecastError::Resolution(resolution));
    }

    let mut observed = Decimal::ZERO;
    let mut profile_observed = Decimal::ZERO;
    let mut last: Option<&MeterInterval> = None;
    for iv in series.iter().filter(|iv| iv.quality().is_billable()) {
        observed = observed
            .checked_add(iv.value())
            .ok_or(ForecastError::Overflow)?;
        profile_observed = profile_observed
            .checked_add(profile_sum(profile, calendar, iv.from(), iv.to())?)
            .ok_or(ForecastError::Overflow)?;
        last = Some(iv);
    }
    let last = last.ok_or(ForecastError::Empty)?;
    if profile_observed <= Decimal::ZERO {
        return Err(ForecastError::ProfileZero);
    }

    let target_year = DayBoundary::Strom
        .day_of(last.from())
        .ok_or(ForecastError::Calendar)?
        .year();
    let year = DayBoundary::Strom
        .year(target_year)
        .ok_or(ForecastError::Calendar)?;
    let profile_year = profile_sum(profile, calendar, year.start(), year.end())?;

    let projected = observed
        .checked_mul(profile_year)
        .and_then(|n| n.checked_div(profile_observed))
        .ok_or(ForecastError::Overflow)?
        .round_dp_with_strategy(FORECAST_DP, FORECAST_STRATEGY);
    Ok(SlpProjection {
        observed_kwh: observed,
        profile_observed,
        profile_year,
        target_year,
        projected_annual_kwh: projected,
    })
}

const QUARTER_HOUR_SECS: u32 = 900;

/// The profile summed over the quarter-hours of `[from, to)`.
fn profile_sum(
    profile: &DynamicSlpProfile,
    calendar: &SlpCalendar,
    from: OffsetDateTime,
    to: OffsetDateTime,
) -> Result<Decimal, ForecastError> {
    let mut sum = Decimal::ZERO;
    let mut at = from;
    while at < to {
        let v = profile
            .value_at(at, calendar)
            .ok_or(ForecastError::ProfileMissing(at))?;
        sum = sum.checked_add(v).ok_or(ForecastError::Overflow)?;
        at += time::Duration::seconds(i64::from(QUARTER_HOUR_SECS));
    }
    Ok(sum)
}

// ── accuracy ──────────────────────────────────────────────────────────────────

/// Why an accuracy score could not be formed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum AccuracyError {
    /// No billable actual slot has a forecast value.
    #[error("no billable actual slot has a forecast value")]
    NoOverlap,
    /// The actual values are all zero.
    #[error("the actual values are all zero")]
    ZeroActual,
    /// No history slot has a billable value one week earlier.
    #[error("the history holds no slot with a value one week earlier")]
    NoBaseline,
    /// The seasonal-naive baseline has zero error on the history.
    #[error("the seasonal-naive baseline has zero error")]
    ZeroBaseline,
    /// An intermediate does not fit a `Decimal`.
    #[error("the arithmetic overflows")]
    Overflow,
}

/// Absolute errors of `forecast` against the billable slots of `actual`:
/// `(Σ|A − F|, Σ|A|, n)`.
fn errors(actual: &Series, forecast: &Series) -> Result<(Decimal, Decimal, u32), AccuracyError> {
    let mut abs_error = Decimal::ZERO;
    let mut abs_actual = Decimal::ZERO;
    let mut n = 0u32;
    for a in actual.iter().filter(|iv| iv.quality().is_billable()) {
        let Some(f) = forecast.get(a.from()) else {
            continue;
        };
        let e = a
            .value()
            .checked_sub(f.value())
            .ok_or(AccuracyError::Overflow)?
            .abs();
        abs_error = abs_error.checked_add(e).ok_or(AccuracyError::Overflow)?;
        abs_actual = abs_actual
            .checked_add(a.value().abs())
            .ok_or(AccuracyError::Overflow)?;
        n += 1;
    }
    if n == 0 {
        return Err(AccuracyError::NoOverlap);
    }
    Ok((abs_error, abs_actual, n))
}

/// **WAPE** — weighted absolute percentage error, as a ratio:
/// `Σ|A − F| ÷ Σ|A|`, rounded to [`FORECAST_ACCURACY_DP`].
///
/// Over the billable slots of `actual` that `forecast` has a value for. Unlike
/// MAPE it is defined where single actuals are zero.
///
/// # Errors
///
/// [`AccuracyError::NoOverlap`] when no slot pairs up,
/// [`AccuracyError::ZeroActual`] when every actual is zero.
pub fn wape(actual: &Series, forecast: &Series) -> Result<Decimal, AccuracyError> {
    let (abs_error, abs_actual, _) = errors(actual, forecast)?;
    if abs_actual.is_zero() {
        return Err(AccuracyError::ZeroActual);
    }
    Ok(abs_error
        .checked_div(abs_actual)
        .ok_or(AccuracyError::Overflow)?
        .round_dp_with_strategy(FORECAST_ACCURACY_DP, FORECAST_ACCURACY_STRATEGY))
}

/// **MASE** — mean absolute scaled error against the seasonal-naive forecast
/// (Hyndman & Koehler 2006), rounded to [`FORECAST_ACCURACY_DP`].
///
/// ```text
/// MASE = mean|A_t − F_t|  ÷  mean|Y_t − Y_{t − 1 week}|
/// ```
///
/// The denominator is the in-sample error on `history` of the
/// **seasonal-naive** forecast: the same Berlin wall-clock slot one week
/// earlier (167 or 169 hours across DST). Below 1 the forecast beats it.
///
/// # Errors
///
/// [`AccuracyError`] when no actual slot pairs with the forecast, when no
/// history slot has a value one week earlier, or when the baseline's error is
/// zero.
pub fn mase(
    history: &Series,
    actual: &Series,
    forecast: &Series,
) -> Result<Decimal, AccuracyError> {
    let (abs_error, _, n) = errors(actual, forecast)?;

    let mut naive = Decimal::ZERO;
    let mut m = 0u32;
    for y in history.iter().filter(|iv| iv.quality().is_billable()) {
        let Some(week_ago) = crate::time::calendar::shift_back_days(y.from(), 7)
            .and_then(|t| history.get(t))
            .filter(|iv| iv.quality().is_billable())
        else {
            continue;
        };
        let e = y
            .value()
            .checked_sub(week_ago.value())
            .ok_or(AccuracyError::Overflow)?
            .abs();
        naive = naive.checked_add(e).ok_or(AccuracyError::Overflow)?;
        m += 1;
    }
    if m == 0 {
        return Err(AccuracyError::NoBaseline);
    }
    if naive.is_zero() {
        return Err(AccuracyError::ZeroBaseline);
    }
    // (Σ|e| ÷ n) ÷ (Σ|naive| ÷ m) as one quotient.
    abs_error
        .checked_mul(Decimal::from(m))
        .and_then(|num| {
            naive
                .checked_mul(Decimal::from(n))
                .and_then(|den| num.checked_div(den))
        })
        .map(|q| q.round_dp_with_strategy(FORECAST_ACCURACY_DP, FORECAST_ACCURACY_STRATEGY))
        .ok_or(AccuracyError::Overflow)
}

/// The signed relative error of an annual energy forecast:
/// `(forecast − actual) ÷ actual`, rounded to [`FORECAST_ACCURACY_DP`].
///
/// Positive when the forecast overstated the year. `None` when `actual` is
/// zero or on overflow.
///
/// ```rust
/// use metering::billing::forecast::annual_energy_error;
/// use rust_decimal::dec;
///
/// assert_eq!(annual_energy_error(dec!(3600), dec!(3500)), Some(dec!(0.0286)));
/// assert_eq!(annual_energy_error(dec!(3600), dec!(0)), None);
/// ```
#[must_use]
pub fn annual_energy_error(forecast_kwh: Decimal, actual_kwh: Decimal) -> Option<Decimal> {
    if actual_kwh.is_zero() {
        return None;
    }
    Some(
        forecast_kwh
            .checked_sub(actual_kwh)?
            .checked_div(actual_kwh)?
            .round_dp_with_strategy(FORECAST_ACCURACY_DP, FORECAST_ACCURACY_STRATEGY),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::series::interval::QualityFlag;
    use rust_decimal::dec;
    use time::{Duration, macros::date};

    /// Quarter-hours over `days` Berlin days from `start`, valued by
    /// `kwh(day_index)`.
    fn days_with(start: Date, days: i64, kwh: impl Fn(i64) -> Decimal) -> Vec<MeterInterval> {
        let mut out = Vec::new();
        for d in 0..days {
            let day = DayBoundary::Strom
                .day(start.checked_add(Duration::days(d)).unwrap())
                .unwrap();
            let n = day.count(Resolution::QUARTER_HOUR).unwrap();
            for i in 0..i64::from(n) {
                out.push(
                    MeterInterval::quarter_hour(
                        day.start() + Duration::minutes(15 * i),
                        kwh(d),
                        QualityFlag::Measured,
                    )
                    .unwrap(),
                );
            }
        }
        out
    }

    fn series(intervals: Vec<MeterInterval>) -> Series {
        Series::new(Resolution::QUARTER_HOUR, DayBoundary::Strom, intervals).unwrap()
    }

    fn flat(start: Date, days: i64, kwh: Decimal) -> Series {
        series(days_with(start, days, |_| kwh))
    }

    #[test]
    fn a_flat_fortnight_projects_the_flat_year() {
        let f =
            project_annual_consumption(&flat(date!(2026 - 01 - 01), 14, dec!(1)), None).unwrap();
        assert_eq!(f.observed_days, 14);
        assert_eq!(f.daily_average_kwh, dec!(96));
        assert_eq!(f.projected_annual_kwh, dec!(96) * dec!(365));
        assert_eq!(f.seasonal_factor, None);
    }

    #[test]
    fn short_or_empty_windows_are_refused_with_a_reason() {
        let empty = Series::new(Resolution::QUARTER_HOUR, DayBoundary::Strom, vec![]).unwrap();
        assert_eq!(
            project_annual_consumption(&empty, None),
            Err(ForecastError::Empty)
        );
        assert_eq!(
            project_annual_consumption(&flat(date!(2026 - 01 - 01), 6, dec!(1)), None),
            Err(ForecastError::TooShort { days: 6 })
        );
        assert!(project_annual_consumption(&flat(date!(2026 - 01 - 01), 7, dec!(1)), None).is_ok());
    }

    #[test]
    fn projection_scales_to_the_real_year_length() {
        let common =
            project_annual_consumption(&flat(date!(2026 - 01 - 01), 14, dec!(1)), None).unwrap();
        let leap =
            project_annual_consumption(&flat(date!(2028 - 01 - 01), 14, dec!(1)), None).unwrap();
        assert_eq!(common.target_year_days, 365);
        assert_eq!(leap.target_year_days, 366);
        assert_eq!(
            leap.projected_annual_kwh - common.projected_annual_kwh,
            dec!(96)
        );
    }

    /// A window across the spring transition is fourteen calendar days, and
    /// its 23-hour day holds four fewer slots — expected as well as observed.
    #[test]
    fn a_window_across_dst_counts_calendar_days_and_real_slots() {
        let f =
            project_annual_consumption(&flat(date!(2026 - 03 - 23), 14, dec!(1)), None).unwrap();
        assert_eq!(f.observed_days, 14);
        assert_eq!(f.expected_slots, 14 * 96 - 4);
        assert_eq!(f.billable_slots, f.expected_slots);
        // 1340 kWh over 14 days.
        assert_eq!(f.daily_average_kwh, dec!(95.714));
    }

    /// A prior year whose January runs at twice the rest of the year makes a
    /// January window *heavy*, so the projection must come **down**.
    #[test]
    fn a_heavy_window_is_corrected_down() {
        let prior = series(days_with(date!(2025 - 01 - 01), 365, |d| {
            if d < 31 { dec!(2) } else { dec!(1) }
        }));
        let observed = flat(date!(2026 - 01 - 05), 14, dec!(2));
        let uncorrected = project_annual_consumption(&observed, None).unwrap();
        let corrected = project_annual_consumption(&observed, Some(&prior)).unwrap();

        let factor = corrected.seasonal_factor.unwrap();
        assert!(factor > dec!(1.5) && factor < dec!(2.0), "{factor}");
        assert!(
            corrected.projected_annual_kwh < uncorrected.projected_annual_kwh,
            "a heavy window corrects down: {} vs {}",
            corrected.projected_annual_kwh,
            uncorrected.projected_annual_kwh
        );
        assert_eq!(
            corrected.projected_annual_kwh,
            (corrected.daily_average_kwh * dec!(365) / factor)
                .round_dp_with_strategy(FORECAST_DP, FORECAST_STRATEGY)
        );
    }

    /// Two years with the same seasonal shape: a January observation of the
    /// second projects the second year's real total within a few percent.
    #[test]
    fn a_seasonal_year_is_projected_within_a_few_percent() {
        // Monthly level: heavy winter, light summer.
        let level = |date: Date| -> Decimal {
            [
                dec!(1.6),
                dec!(1.5),
                dec!(1.3),
                dec!(1.1),
                dec!(0.9),
                dec!(0.8),
                dec!(0.7),
                dec!(0.75),
                dec!(0.9),
                dec!(1.1),
                dec!(1.35),
                dec!(1.55),
            ][usize::from(u8::from(date.month())) - 1]
        };
        let year = |y: i32, days: i64| {
            let start = Date::from_calendar_date(y, time::Month::January, 1).unwrap();
            series(days_with(start, days, |d| {
                level(start.checked_add(Duration::days(d)).unwrap())
            }))
        };
        let prior = year(2025, 365);
        let truth = year(2026, 365).billable_total().unwrap();
        let january = year(2026, 31);

        let f = project_annual_consumption(&january, Some(&prior)).unwrap();
        let err = annual_energy_error(f.projected_annual_kwh, truth).unwrap();
        assert!(err.abs() < dec!(0.02), "within 2 %: {err}");

        // Without the correction January overstates the year by half.
        let naive = project_annual_consumption(&january, None).unwrap();
        let naive_err = annual_energy_error(naive.projected_annual_kwh, truth).unwrap();
        assert!(naive_err > dec!(0.3), "{naive_err}");
    }

    /// Missing slots lower the observed days, not the rate.
    #[test]
    fn a_gap_lowers_the_coverage_not_the_rate() {
        let mut slots = days_with(date!(2026 - 01 - 01), 14, |_| dec!(1));
        slots.drain(96..192); // a whole day missing
        let f = project_annual_consumption(&series(slots), None).unwrap();
        assert_eq!(f.observed_days, 14);
        assert_eq!(f.billable_slots, 13 * 96);
        assert_eq!(f.daily_average_kwh, dec!(96), "the rate is unchanged");

        // Non-billable slots are the same as missing ones.
        let mut slots = days_with(date!(2026 - 01 - 01), 14, |_| dec!(1));
        for iv in slots.iter_mut().take(96) {
            *iv = iv.clone().with_quality(QualityFlag::Faulty);
        }
        let f = project_annual_consumption(&series(slots), None).unwrap();
        assert_eq!(f.daily_average_kwh, dec!(96));
    }

    #[test]
    fn coverage_below_the_threshold_is_refused() {
        let mut slots = days_with(date!(2026 - 01 - 01), 10, |_| dec!(1));
        // Drop 97 of 960: 863 left, 89.9 %.
        slots.drain(100..197);
        assert_eq!(
            project_annual_consumption(&series(slots), None),
            Err(ForecastError::Coverage {
                billable: 863,
                expected: 960
            })
        );
        // 864 of 960 is exactly 90 %, and passes.
        let mut slots = days_with(date!(2026 - 01 - 01), 10, |_| dec!(1));
        slots.drain(100..196);
        assert!(project_annual_consumption(&series(slots), None).is_ok());
    }

    /// The prior year's window is coverage-corrected too: a gap in it does
    /// not move a flat reference off 1.
    #[test]
    fn a_gap_in_the_prior_window_does_not_move_the_factor() {
        let mut prior = days_with(date!(2025 - 01 - 01), 60, |_| dec!(1));
        prior.drain(96..192);
        let f = project_annual_consumption(
            &flat(date!(2026 - 01 - 01), 14, dec!(1)),
            Some(&series(prior)),
        )
        .unwrap();
        assert_eq!(f.seasonal_factor, Some(Decimal::ONE));
    }

    #[test]
    fn a_prior_year_that_misses_the_window_is_refused() {
        let prior = flat(date!(2025 - 08 - 01), 30, dec!(5));
        let observed = flat(date!(2026 - 01 - 05), 14, dec!(1));
        assert!(matches!(
            project_annual_consumption(&observed, Some(&prior)),
            Err(ForecastError::Coverage { billable: 0, .. })
        ));
    }

    fn g25() -> (DynamicSlpProfile, SlpCalendar) {
        use crate::slp::strom::{LoadProfile, SlpDayType};
        let mut p = DynamicSlpProfile::new(LoadProfile::G25);
        for month in 1u8..=12 {
            // Winter months weigh twice the summer ones.
            let v = if (4..=9).contains(&month) {
                dec!(1)
            } else {
                dec!(2)
            };
            for dt in SlpDayType::ALL {
                p.insert(month, dt, vec![v; 96]).unwrap();
            }
        }
        (p, SlpCalendar::new(crate::time::holiday::Bundesland::Be))
    }

    /// A delivery point that follows its profile exactly projects to the
    /// profile's year, whatever part of the year was observed.
    #[test]
    fn the_slp_projection_scales_by_the_profile_share() {
        let (profile, cal) = g25();
        // 2 kWh per winter quarter-hour: exactly the profile.
        let january = flat(date!(2026 - 01 - 01), 31, dec!(2));
        let p = project_annual_slp(&january, &profile, &cal).unwrap();
        // Year: 182 winter days × 96 × 2 + 183 summer days × 96 × 1, minus/plus
        // the DST quarter-hours (March −4 at 2, October +4 at 2): equals the
        // profile's own year sum.
        assert_eq!(p.projected_annual_kwh, p.profile_year);
        assert_eq!(p.target_year, 2026);

        // A gap is missing from both sides, so the projection does not move.
        let mut slots = days_with(date!(2026 - 01 - 01), 31, |_| dec!(2));
        slots.drain(0..500);
        let gappy = project_annual_slp(&series(slots), &profile, &cal).unwrap();
        assert_eq!(gappy.projected_annual_kwh, p.projected_annual_kwh);
    }

    #[test]
    fn the_slp_projection_refuses_a_missing_profile_value() {
        use crate::slp::strom::LoadProfile;
        let empty = DynamicSlpProfile::new(LoadProfile::G25);
        let cal = SlpCalendar::new(crate::time::holiday::Bundesland::Be);
        assert!(matches!(
            project_annual_slp(&flat(date!(2026 - 01 - 01), 1, dec!(1)), &empty, &cal),
            Err(ForecastError::ProfileMissing(_))
        ));
    }

    #[test]
    fn wape_weights_absolute_errors_by_energy() {
        let actual = series(days_with(date!(2026 - 01 - 01), 1, |_| dec!(2)));
        let forecast = series(days_with(date!(2026 - 01 - 01), 1, |_| dec!(1.5)));
        // Σ|2 − 1,5| ÷ Σ2 = 0,25.
        assert_eq!(wape(&actual, &forecast), Ok(dec!(0.25)));
        let zero = flat(date!(2026 - 01 - 01), 1, dec!(0));
        assert_eq!(wape(&zero, &forecast), Err(AccuracyError::ZeroActual));
        let elsewhere = flat(date!(2026 - 02 - 01), 1, dec!(1));
        assert_eq!(wape(&actual, &elsewhere), Err(AccuracyError::NoOverlap));
    }

    /// MASE below 1 beats repeating last week; the scale is the in-sample
    /// seasonal-naive error.
    #[test]
    fn mase_scales_by_the_seasonal_naive_error() {
        // History: alternate weeks at 1 and 2 kWh per quarter-hour, so the
        // naive "one week earlier" is off by 1 on every slot.
        let history = series(days_with(date!(2026 - 01 - 05), 21, |d| {
            if (d / 7) % 2 == 0 { dec!(1) } else { dec!(2) }
        }));
        let actual = flat(date!(2026 - 01 - 26), 1, dec!(1));
        let forecast = flat(date!(2026 - 01 - 26), 1, dec!(1.25));
        // mean|e| = 0,25, mean|naive| = 1.
        assert_eq!(mase(&history, &actual, &forecast), Ok(dec!(0.25)));

        let constant = flat(date!(2026 - 01 - 05), 14, dec!(1));
        assert_eq!(
            mase(&constant, &actual, &forecast),
            Err(AccuracyError::ZeroBaseline)
        );
        let short = flat(date!(2026 - 01 - 05), 3, dec!(1));
        assert_eq!(
            mase(&short, &actual, &forecast),
            Err(AccuracyError::NoBaseline)
        );
    }

    #[test]
    fn the_annual_energy_error_is_signed() {
        assert_eq!(annual_energy_error(dec!(90), dec!(100)), Some(dec!(-0.1)));
        assert_eq!(annual_energy_error(dec!(110), dec!(100)), Some(dec!(0.1)));
        assert_eq!(annual_energy_error(dec!(1), Decimal::ZERO), None);
    }
}
