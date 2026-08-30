//! Jahresprognose — projecting a full year from a partial one.
//!
//! ## Scope
//!
//! One calculation: scale an observed period's consumption to a whole calendar
//! year, optionally corrected for where in the year the observation sits.
//! It is used for Abschlag sizing, for the SLP Jahresprognose, and for
//! estimating year-end Mehr-/Mindermengen.
//!
//! **Gap filling lives in [`crate::substitute`].** Fill the series first, then
//! project from it.
//!
//! ## What this does not do
//!
//! No machine learning. An ML forecaster needs a runtime — PyTorch, ONNX — and
//! this crate performs no I/O and links nothing that does. Extrapolating a
//! daily mean is not a model, and calling it one would be worse than the
//! honest arithmetic below.

use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive as _;
use time::OffsetDateTime;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use crate::interval::MeterInterval;

// ── AnnualForecast ────────────────────────────────────────────────────────────

/// A projected annual consumption, with the inputs that produced it.
///
/// ## Method
///
/// ```text
/// annual_kwh = observed / observed_days × days_in_target_year × seasonal_factor
/// ```
///
/// `observed_days` counts **Europe/Berlin calendar days** and the year factor
/// is the target year's real length, so neither a DST transition inside the
/// observation window nor a leap day skews the result.
///
/// The result is cut to [`FORECAST_DP`] places; the division that produces it
/// does not generally terminate.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct AnnualForecast {
    /// Start of the observation window (earliest interval start).
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::rfc3339"))]
    pub observation_from: OffsetDateTime,
    /// End of the observation window (latest interval end).
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::rfc3339"))]
    pub observation_to: OffsetDateTime,
    /// Billable energy observed in the window (kWh).
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::decimal"))]
    pub observed: Decimal,
    /// Berlin calendar days the window spans.
    pub observed_days: u32,
    /// Days in the target year — 366 in a leap year.
    pub target_year_days: u16,
    /// Projected annual consumption (kWh), cut to [`FORECAST_DP`] places.
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::decimal"))]
    pub projected_annual: Decimal,
    /// Seasonal correction factor; `1` when none was applied.
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::decimal"))]
    pub seasonal_factor: Decimal,
    /// Whether a seasonal factor could be derived from prior-year data.
    ///
    /// An uncorrected projection from a January window overstates the year for
    /// a heating-dominated load and understates it for a cooling-dominated one.
    /// The flag exists so a caller can refuse to bill on one.
    ///
    /// It records whether the **correction ran**, not whether it moved the
    /// number: a window whose prior-year rate equals the prior year's overall
    /// rate produces a factor of exactly `1`, and that is a corrected
    /// projection rather than an uncorrected one.
    pub seasonal_correction_applied: bool,
    /// Lower bound of the 95 % prediction interval (kWh), clamped at zero and
    /// cut to [`FORECAST_DP`] places.
    ///
    /// See [`AnnualForecast::prediction_interval_note`] for what it does and
    /// does not claim. `None` when fewer than two whole days were observed.
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::decimal_option"))]
    pub confidence_lower: Option<Decimal>,
    /// Upper bound of the 95 % prediction interval (kWh), cut to
    /// [`FORECAST_DP`] places.
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::decimal_option"))]
    pub confidence_upper: Option<Decimal>,
}

impl AnnualForecast {
    /// What the prediction interval is and is not.
    ///
    /// It treats the observed **daily** sums as independent draws from one
    /// distribution and asks how far the year's total could plausibly land from
    /// the projection. Two sources of error contribute, and both must:
    ///
    /// ```text
    /// Var(total) = Y² · σ²/n   (the daily mean is estimated from n days)
    ///            + Y  · σ²     (the remaining days vary around it)
    /// ```
    ///
    /// The first term dominates for a short window: with `Y = 365` and
    /// `n = 14` it is twenty-six times the second, so omitting it would report
    /// an interval roughly five times too narrow.
    ///
    /// What it still does not model: daily sums from a load profile are
    /// **not** independent and not identically distributed. Consumption is
    /// autocorrelated and strongly seasonal, so a January window's spread
    /// understates a whole year's. Treat the interval as a lower bound on the
    /// uncertainty, not a confidence statement about the year.
    #[must_use]
    pub const fn prediction_interval_note() -> &'static str {
        "95 % interval from daily-sum variability, assuming independent days; \
         real load profiles are autocorrelated and seasonal, so the true spread is wider"
    }

    /// Mean daily consumption over the observation window (kWh/day).
    #[must_use]
    pub fn daily_average_kwh(&self) -> Decimal {
        if self.observed_days == 0 {
            return Decimal::ZERO;
        }
        self.observed / Decimal::from(self.observed_days)
    }
}

/// Decimal places a projected quantity is cut to: **3**, a milliwatt-hour.
///
/// The projection divides — `observed ÷ observed_days` — so its quotient does
/// not generally terminate, and an annual figure carrying twenty-eight
/// significant digits is not a quantity anyone can put on an Abschlag. Cutting
/// it makes every reported number representable; the granularity is four orders
/// of magnitude below anything the market settles.
///
/// [`AnnualForecast::projected_annual`] is therefore homogeneous only to its
/// last reported place: doubling every reading doubles the projection to within
/// `2 × 10⁻³` kWh rather than exactly, because `round(2x) ≠ 2·round(x)` at a
/// rounding boundary. See the crate-level **What "exact" means here**.
pub const FORECAST_DP: u32 = 3;

/// The minimum observation window that yields a projection.
///
/// Below a week the daily mean is dominated by the weekday mix of whichever
/// days happened to be observed, and no correction here can repair that.
pub const MIN_OBSERVATION_DAYS: i64 = 7;

// ── project_annual_consumption ────────────────────────────────────────────────

/// Project annual consumption from a partial year of interval data.
///
/// Aggregates the billable energy, divides by the Berlin calendar days the
/// window spans, and scales to the **real length of the target year** — the
/// Berlin calendar year the observation ends in. A flat 365 would understate a
/// leap-year Jahresprognose by one day, 0.27 %, which is real money on an
/// industrial Abschlag.
///
/// When `prior_year_intervals` are supplied, a seasonal factor corrects for
/// where in the year the window sits: the prior year's daily rate over the
/// matching window, divided by the prior year's daily rate overall.
///
/// ## Returns
///
/// `None` when `intervals` is empty or spans fewer than
/// [`MIN_OBSERVATION_DAYS`].
///
/// ## Example
///
/// ```rust
/// use metering::{project_annual_consumption, MeterInterval, QualityFlag, calendar};
/// use rust_decimal::dec;
/// use time::{Duration, macros::date};
///
/// // Fourteen days at 1 kWh per quarter-hour = 96 kWh/day.
/// let base = calendar::day_start_utc(date!(2026 - 01 - 01));
/// let intervals: Vec<MeterInterval> = (0..14 * 96).map(|i| MeterInterval {
///     from: base + Duration::minutes(15 * i),
///     to:   base + Duration::minutes(15 * i + 15),
///     value: dec!(1),
///     quality: QualityFlag::Measured,
///     obis_code: None,
/// }).collect();
///
/// let f = project_annual_consumption(&intervals, None).unwrap();
/// assert_eq!(f.observed_days, 14);
/// assert_eq!(f.projected_annual, dec!(96) * dec!(365));
/// ```
#[must_use]
pub fn project_annual_consumption(
    intervals: &[MeterInterval],
    prior_year_intervals: Option<&[MeterInterval]>,
) -> Option<AnnualForecast> {
    let first_from = intervals.iter().map(|iv| iv.from).min()?;
    let last_to = intervals.iter().map(|iv| iv.to).max()?;

    // Berlin calendar days, not `(last_to - first_from).whole_days()`: an
    // observation window spanning the spring transition is 24n − 1 hours long,
    // and integer division reports one day fewer than it covers. That inflates
    // the daily average — and the projection built on it — by 1/n.
    let observed_days_i64 = crate::calendar::days_between(first_from, last_to);
    if observed_days_i64 < MIN_OBSERVATION_DAYS {
        return None;
    }
    let observed_days = u32::try_from(observed_days_i64).ok()?;

    let observed: Decimal = intervals
        .iter()
        .filter(|iv| iv.quality.is_billable())
        .map(|iv| iv.value)
        .sum();

    let target_year = crate::calendar::local_year(last_to);
    let target_year_days = crate::calendar::days_in_year(target_year);
    let daily_avg = observed / Decimal::from(observed_days);

    // The flag says whether the correction ran, not whether the factor differs
    // from 1 — a legitimately neutral factor is still a correction.
    let derived_factor =
        prior_year_intervals.and_then(|prior| seasonal_factor(first_from, last_to, prior));
    let seasonal_correction_applied = derived_factor.is_some();
    let seasonal_factor = derived_factor.unwrap_or(Decimal::ONE);

    let projected = daily_avg * Decimal::from(target_year_days) * seasonal_factor;

    let half_width = prediction_half_width(intervals, seasonal_factor, target_year_days);

    Some(AnnualForecast {
        observation_from: first_from,
        observation_to: last_to,
        observed,
        observed_days,
        target_year_days,
        projected_annual: projected.round_dp(FORECAST_DP),
        seasonal_factor,
        seasonal_correction_applied,
        confidence_lower: half_width
            .map(|h| (projected - h).max(Decimal::ZERO).round_dp(FORECAST_DP)),
        confidence_upper: half_width.map(|h| (projected + h).round_dp(FORECAST_DP)),
    })
}

// ── statistics ────────────────────────────────────────────────────────────────

/// Half-width of the 95 % prediction interval for the annual total.
///
/// `Var(total) = Y²σ²/n + Yσ²`, the estimation error of the daily mean plus the
/// residual variation of the remaining days — see
/// [`AnnualForecast::prediction_interval_note`].
///
/// Computed in `f64`: the bounds are diagnostics, not billed quantities, and
/// the projection itself stays exact `Decimal`. `None` with fewer than two
/// observed days or when the statistics degenerate.
fn prediction_half_width(
    intervals: &[MeterInterval],
    seasonal_factor: Decimal,
    year_days: u16,
) -> Option<Decimal> {
    use std::collections::BTreeMap;

    // Group by Berlin calendar day, matching how the daily sums this variance
    // describes are actually settled.
    let mut daily: BTreeMap<time::Date, Decimal> = BTreeMap::new();
    for iv in intervals.iter().filter(|iv| iv.quality.is_billable()) {
        *daily.entry(iv.berlin_day()).or_insert(Decimal::ZERO) += iv.value;
    }
    if daily.len() < 2 {
        return None;
    }

    let values: Vec<f64> = daily.values().filter_map(Decimal::to_f64).collect();
    if values.len() != daily.len() {
        return None;
    }
    let n = values.len() as f64;
    let mean = values.iter().sum::<f64>() / n;
    let variance = values.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / (n - 1.0);

    let y = f64::from(year_days);
    // Y²σ²/n + Yσ² — the estimation term first, which dominates for short
    // windows.
    let total_variance = variance * (y * y / n + y);
    let factor = seasonal_factor.to_f64()?;
    let half = 1.96 * total_variance.sqrt() * factor.abs();
    if !half.is_finite() {
        return None;
    }
    Decimal::try_from(half)
        .ok()
        .map(|d| d.round_dp(FORECAST_DP))
}

/// The prior year's daily rate over the matching window, relative to its
/// overall daily rate.
///
/// A factor above 1 means the observation window is a heavier-than-average part
/// of the year, so the naive projection would overstate the year without it.
///
/// `None` when the prior-year data cannot support the comparison — no overlap
/// with the shifted window, or a zero total on either side.
fn seasonal_factor(
    obs_from: OffsetDateTime,
    obs_to: OffsetDateTime,
    prior_year: &[MeterInterval],
) -> Option<Decimal> {
    if prior_year.is_empty() {
        return None;
    }

    // Shift the observation window back one calendar year. A fixed 365-day
    // subtraction drifts by a day across a leap year and lands an hour off
    // across a DST transition, so a "same two weeks of March" comparison would
    // silently compare different windows.
    let prior_from = crate::calendar::shift_back_one_year(obs_from);
    let prior_to = crate::calendar::shift_back_one_year(obs_to);

    let billable = |iv: &&MeterInterval| iv.quality.is_billable();

    let prior_window_kwh: Decimal = prior_year
        .iter()
        .filter(billable)
        .filter(|iv| iv.from >= prior_from && iv.to <= prior_to)
        .map(|iv| iv.value)
        .sum();
    let prior_total_kwh: Decimal = prior_year.iter().filter(billable).map(|iv| iv.value).sum();

    if prior_window_kwh.is_zero() || prior_total_kwh.is_zero() {
        return None;
    }

    // Both rates are per Berlin calendar day: the window over its own span, the
    // reference over the span the prior-year data actually covers. Dividing the
    // total by a flat 365 would inflate the factor whenever the caller supplies
    // less than a full year — the failure mode of assuming "prior year" means
    // "a whole year".
    let window_days = crate::calendar::days_between(prior_from, prior_to).max(1);
    let prior_first = prior_year.iter().map(|iv| iv.from).min()?;
    let prior_last = prior_year.iter().map(|iv| iv.to).max()?;
    let reference_days = crate::calendar::days_between(prior_first, prior_last).max(1);

    let window_daily = prior_window_kwh / Decimal::from(window_days);
    let reference_daily = prior_total_kwh / Decimal::from(reference_days);
    if reference_daily.is_zero() {
        return None;
    }

    Some((window_daily / reference_daily).round_dp(4))
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interval::QualityFlag;
    use rust_decimal::dec;
    use time::{
        Duration,
        macros::{date, datetime},
    };

    fn make_iv(from: OffsetDateTime, kwh: Decimal) -> MeterInterval {
        MeterInterval {
            from,
            to: from + Duration::minutes(15),
            value: kwh,
            quality: QualityFlag::Measured,
            obis_code: None,
        }
    }

    /// `n` days of quarter-hours from local midnight on `start`, `kwh` each.
    fn aligned_days(start: time::Date, days: i64, kwh: Decimal) -> Vec<MeterInterval> {
        let base = crate::calendar::day_start_utc(start);
        (0..days * 96)
            .map(|i| make_iv(base + Duration::minutes(15 * i), kwh))
            .collect()
    }

    #[test]
    fn a_flat_fortnight_projects_the_flat_year() {
        let intervals = aligned_days(date!(2026 - 01 - 01), 14, dec!(1));
        let f = project_annual_consumption(&intervals, None).unwrap();
        assert_eq!(f.observed_days, 14);
        assert_eq!(f.daily_average_kwh(), dec!(96));
        assert_eq!(f.projected_annual, dec!(96) * dec!(365));
        assert!(!f.seasonal_correction_applied);
        assert_eq!(f.seasonal_factor, Decimal::ONE);
    }

    #[test]
    fn insufficient_data_returns_none() {
        assert!(project_annual_consumption(&[], None).is_none());
        let one = vec![make_iv(datetime!(2026-01-01 00:00 UTC), dec!(1))];
        assert!(project_annual_consumption(&one, None).is_none());
        // Six days is below the floor; seven is not.
        assert!(
            project_annual_consumption(&aligned_days(date!(2026 - 01 - 01), 6, dec!(1)), None)
                .is_none()
        );
        assert!(
            project_annual_consumption(&aligned_days(date!(2026 - 01 - 01), 7, dec!(1)), None)
                .is_some()
        );
    }

    /// A leap year is 366 days of consumption, not 365.
    #[test]
    fn projection_scales_to_the_real_year_length() {
        let common =
            project_annual_consumption(&aligned_days(date!(2026 - 01 - 01), 14, dec!(1)), None)
                .unwrap();
        let leap =
            project_annual_consumption(&aligned_days(date!(2028 - 01 - 01), 14, dec!(1)), None)
                .unwrap();

        assert_eq!(common.target_year_days, 365);
        assert_eq!(leap.target_year_days, 366);
        assert_eq!(
            leap.projected_annual - common.projected_annual,
            dec!(96),
            "the difference is exactly one day"
        );
    }

    /// An observation window spanning the spring transition is 24n − 1 hours,
    /// so `whole_days()` counted 13 days for 14 and inflated the projection by
    /// 7.7 %.
    #[test]
    fn observation_window_spanning_dst_counts_calendar_days() {
        // Fourteen *calendar* days of quarter-hours. The March window holds
        // four fewer intervals than the June one, because one of its days is
        // 23 hours long — which is the whole point.
        let calendar_days = |start: time::Date, days: i64| {
            let base = crate::calendar::day_start_utc(start);
            let count: u32 = (0..days)
                .map(|d| {
                    crate::calendar::intervals_in_day(
                        start.checked_add(Duration::days(d)).unwrap(),
                        crate::IntervalResolution::QuarterHour,
                    )
                    .unwrap()
                })
                .sum();
            (0..i64::from(count))
                .map(|i| make_iv(base + Duration::minutes(15 * i), dec!(1)))
                .collect::<Vec<_>>()
        };

        let across_dst = calendar_days(date!(2026 - 03 - 23), 14); // holds the 23-hour day
        let ordinary = calendar_days(date!(2026 - 06 - 01), 14);
        assert_eq!(across_dst.len() + 4, ordinary.len());

        let a = project_annual_consumption(&across_dst, None).unwrap();
        let b = project_annual_consumption(&ordinary, None).unwrap();

        assert_eq!(a.observed_days, 14, "fourteen calendar days, not thirteen");
        assert_eq!(b.observed_days, 14);

        // The DST window holds four fewer intervals (the lost hour), so its
        // projection is legitimately a touch lower — but only by that hour, not
        // by the 7.7 % a truncated day count would have added.
        let ratio = a.projected_annual / b.projected_annual;
        assert!(
            ratio > dec!(0.997) && ratio < dec!(1.0),
            "projections must differ only by the lost hour, got {ratio}"
        );
    }

    // ── prediction interval ──────────────────────────────────────────────────

    #[test]
    fn bounds_bracket_the_projection() {
        let base = crate::calendar::day_start_utc(date!(2026 - 01 - 01));
        let intervals: Vec<_> = (0..14 * 96)
            .map(|i| {
                let kwh = if (i / 96) % 2 == 0 {
                    dec!(1.0)
                } else {
                    dec!(1.4)
                };
                make_iv(base + Duration::minutes(15 * i), kwh)
            })
            .collect();
        let f = project_annual_consumption(&intervals, None).unwrap();
        let lower = f.confidence_lower.unwrap();
        let upper = f.confidence_upper.unwrap();
        assert!(lower < f.projected_annual && f.projected_annual < upper);
        assert!(lower >= Decimal::ZERO, "lower bound clamped at zero");
    }

    /// Zero day-to-day variance collapses the interval onto the projection.
    #[test]
    fn constant_consumption_has_a_zero_width_interval() {
        let f =
            project_annual_consumption(&aligned_days(date!(2026 - 01 - 01), 14, dec!(1.0)), None)
                .unwrap();
        assert_eq!(f.confidence_lower, Some(f.projected_annual));
        assert_eq!(f.confidence_upper, Some(f.projected_annual));
    }

    /// The estimation term dominates a short window. Omitting it — as the
    /// previous `1.96 · σ · √Y` did — reported an interval about five times
    /// too narrow at n = 14.
    #[test]
    fn the_interval_includes_the_estimation_error() {
        let base = crate::calendar::day_start_utc(date!(2026 - 01 - 01));
        let intervals: Vec<_> = (0..14 * 96)
            .map(|i| {
                let kwh = if (i / 96) % 2 == 0 {
                    dec!(1.0)
                } else {
                    dec!(2.0)
                };
                make_iv(base + Duration::minutes(15 * i), kwh)
            })
            .collect();
        let f = project_annual_consumption(&intervals, None).unwrap();
        let half = (f.confidence_upper.unwrap() - f.projected_annual)
            .to_f64()
            .unwrap();

        // Daily sums alternate 96 and 192 kWh: σ ≈ 49.9 over n = 14.
        let sigma = 49.9_f64;
        let y = 365.0_f64;
        let old = 1.96 * sigma * y.sqrt(); // the formula that was there
        let new = 1.96 * (sigma * sigma * (y * y / 14.0 + y)).sqrt();

        assert!(
            (half - new).abs() / new < 0.05,
            "half-width {half:.0} should be near {new:.0}"
        );
        assert!(
            half > old * 4.0,
            "the corrected interval must be several times the old one: {half:.0} vs {old:.0}"
        );
    }

    /// A longer window shrinks the interval, which is the whole point of the
    /// estimation term.
    #[test]
    fn a_longer_window_narrows_the_interval() {
        let alternating = |days: i64| {
            let base = crate::calendar::day_start_utc(date!(2026 - 01 - 01));
            (0..days * 96)
                .map(|i| {
                    let kwh = if (i / 96) % 2 == 0 {
                        dec!(1.0)
                    } else {
                        dec!(2.0)
                    };
                    make_iv(base + Duration::minutes(15 * i), kwh)
                })
                .collect::<Vec<_>>()
        };
        let short = project_annual_consumption(&alternating(14), None).unwrap();
        let long = project_annual_consumption(&alternating(120), None).unwrap();

        let width = |f: &AnnualForecast| f.confidence_upper.unwrap() - f.confidence_lower.unwrap();
        assert!(
            width(&long) < width(&short),
            "120 days must give a tighter interval than 14: {} vs {}",
            width(&long),
            width(&short)
        );
    }

    // ── seasonality ──────────────────────────────────────────────────────────

    /// A January window on a heating load: the prior year says January runs at
    /// twice the annual daily rate, so the naive projection halves.
    #[test]
    fn seasonal_correction_scales_a_winter_window_down() {
        // Prior year: January at 2 kWh per quarter-hour, the rest at 1.
        let base = crate::calendar::day_start_utc(date!(2025 - 01 - 01));
        let prior: Vec<_> = (0..365 * 96)
            .map(|i| {
                let kwh = if i < 31 * 96 { dec!(2) } else { dec!(1) };
                make_iv(base + Duration::minutes(15 * i), kwh)
            })
            .collect();

        let observed = aligned_days(date!(2026 - 01 - 05), 14, dec!(2));
        let uncorrected = project_annual_consumption(&observed, None).unwrap();
        let corrected = project_annual_consumption(&observed, Some(&prior)).unwrap();

        assert!(corrected.seasonal_correction_applied);
        assert!(
            corrected.seasonal_factor > dec!(1.5) && corrected.seasonal_factor < dec!(2.0),
            "January runs hot relative to the year: {}",
            corrected.seasonal_factor
        );
        assert!(
            corrected.projected_annual > uncorrected.projected_annual,
            "the factor scales the projection"
        );
    }

    /// A caller passing six months of "prior year" data must not have it
    /// treated as a full year — that would double the reference daily rate and
    /// halve the factor.
    #[test]
    fn a_partial_prior_year_is_measured_over_its_own_span() {
        // Six months of perfectly flat prior data. A flat reference means the
        // window rate equals the overall rate, so the factor is 1 whatever the
        // span — unless the span is assumed to be 365 days, in which case it
        // comes out near 2.
        let prior = aligned_days(date!(2025 - 01 - 01), 180, dec!(1));
        let observed = aligned_days(date!(2026 - 02 - 01), 14, dec!(1));
        let f = project_annual_consumption(&observed, Some(&prior)).unwrap();
        assert_eq!(
            f.seasonal_factor,
            Decimal::ONE,
            "a flat reference is seasonally neutral over any span"
        );
        assert!(
            f.seasonal_correction_applied,
            "the correction ran and found the window neutral — which is not the \
             same as having no reference to run it against"
        );
    }

    #[test]
    fn no_prior_overlap_leaves_the_projection_uncorrected() {
        // Prior data from a completely different part of the year.
        let prior = aligned_days(date!(2025 - 08 - 01), 30, dec!(5));
        let observed = aligned_days(date!(2026 - 01 - 05), 14, dec!(1));
        let f = project_annual_consumption(&observed, Some(&prior)).unwrap();
        assert!(!f.seasonal_correction_applied);
        assert_eq!(f.seasonal_factor, Decimal::ONE);
    }

    #[test]
    fn non_billable_intervals_do_not_contribute() {
        let mut intervals = aligned_days(date!(2026 - 01 - 01), 14, dec!(1));
        for iv in intervals.iter_mut().take(96) {
            iv.quality = QualityFlag::Faulty;
        }
        let f = project_annual_consumption(&intervals, None).unwrap();
        assert_eq!(
            f.observed,
            dec!(1) * Decimal::from(13 * 96),
            "the faulty day is excluded from the sum"
        );
        assert_eq!(f.observed_days, 14, "but the window is still fourteen days");
    }

    #[test]
    fn the_note_says_what_the_interval_does_not_model() {
        let note = AnnualForecast::prediction_interval_note();
        assert!(note.contains("autocorrelated"), "{note}");
    }
}
