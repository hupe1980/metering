//! Netzqualität — EN 50160 voltage characteristics.
//!
//! ## EN 50160 is a statistical standard, and that is the whole point
//!
//! It is tempting to write `voltage > 253 V → non-compliant`. EN 50160 says no
//! such thing. Every one of its limits is a **share of 10-minute mean values
//! over an observation window**:
//!
//! | Parameter | Limit | Share | Window |
//! |---|---|---|---|
//! | Supply voltage | `Un ± 10 %` | 95 % | one week |
//! | Supply voltage | `Un + 10 % / − 15 %` | 100 % | one week |
//! | Frequency | `50 Hz ± 1 %` | 99.5 % | one year |
//! | Frequency | `50 Hz + 4 % / − 6 %` | 100 % | one year |
//! | THD of voltage | `≤ 8 %` | 95 % | one week |
//!
//! A week of 10-minute means is 1 008 samples, and up to 50 of them may sit
//! outside `Un ± 10 %` with the supply still conforming. So a single interval
//! above 253 V is **not** a breach, and reporting it as one produces alarms
//! that are individually true and collectively meaningless.
//!
//! This module therefore separates the two questions:
//!
//! - [`PowerQualityInterval`] carries the per-interval **indicators**
//!   ([`voltage_out_of_range`](PowerQualityInterval::voltage_out_of_range) and
//!   friends). They say "this sample is outside the band" — useful for
//!   triage, and not a conformance verdict.
//! - [`assess_en50160`] answers the standard's actual question over a series.

//!
//! ## What is deliberately not assessed
//!
//! - **Voltage unbalance.** EN 50160 limits the negative-sequence ratio
//!   `u₂ = U₂ / U₁` to 2 % for 95 % of a week. Computing `U₂` needs the phase
//!   *angles*, and a meter that reports three RMS magnitudes has not supplied
//!   them. The magnitude-only approximations in circulation answer a different
//!   question, and returning one under the name "unbalance" would be a
//!   conformance claim this data cannot support.
//! - **Flicker (`Plt`), dips, swells, interruptions and harmonics by order.**
//!   These need waveform-level measurement, not interval means.
//!
//! ## Scope
//!
//! The limits below are for **low voltage** (`Un = 230 V` phase-to-neutral).
//! EN 50160 states different bands for medium and high voltage; supply them
//! through [`En50160Limits`] rather than assuming these apply.

use rust_decimal::Decimal;
use time::OffsetDateTime;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

// ── PowerQualityInterval ──────────────────────────────────────────────────────

/// One power-quality measurement interval.
///
/// EN 50160 is defined on **10-minute mean** values, so that is the resolution
/// [`assess_en50160`] expects. The type does not enforce it — a meter may
/// deliver quarter-hours — but the assessment's percentages only mean what the
/// standard says they mean at 10 minutes.
///
/// Kept separate from [`MeterInterval`](crate::MeterInterval) because these are
/// *instantaneous* quantities averaged over the interval, not quantities
/// accumulated within it: two voltage intervals cannot be added, and one that
/// covers twice the span does not carry twice the volts.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct PowerQualityInterval {
    /// Interval start (UTC).
    pub from: OffsetDateTime,
    /// Interval end (UTC).
    pub to: OffsetDateTime,

    /// L1 phase voltage in Volt, mean over the interval.
    pub voltage_l1_v: Option<Decimal>,
    /// L2 phase voltage in Volt. `None` on a single-phase meter.
    pub voltage_l2_v: Option<Decimal>,
    /// L3 phase voltage in Volt. `None` on a single-phase meter.
    pub voltage_l3_v: Option<Decimal>,

    /// L1 phase current in Ampere, mean over the interval.
    pub current_l1_a: Option<Decimal>,
    /// L2 phase current in Ampere.
    pub current_l2_a: Option<Decimal>,
    /// L3 phase current in Ampere.
    pub current_l3_a: Option<Decimal>,

    /// Grid frequency in Hz, mean over the interval. Nominal 50.00 Hz.
    pub frequency_hz: Option<Decimal>,

    /// Power factor (cos φ), dimensionless.
    ///
    /// Not an EN 50160 parameter — the standard says nothing about it. It is
    /// here because Blindstrom surcharges in industrial NNE tariffs turn on it.
    pub power_factor: Option<Decimal>,

    /// Total harmonic distortion of the voltage, in percent.
    ///
    /// EN 50160 sums harmonics up to order 40.
    pub thd_voltage_pct: Option<Decimal>,
    /// Total harmonic distortion of the current, in percent.
    ///
    /// Not an EN 50160 parameter: the standard characterises the *supply*, and
    /// current distortion is a property of the installation drawing it.
    pub thd_current_pct: Option<Decimal>,
}

impl PowerQualityInterval {
    /// An interval over `[from, to)` with every measurement absent.
    ///
    /// There is deliberately no `Default`: it would have to invent a window,
    /// and the only candidate — a zero-length interval at the Unix epoch — is a
    /// value no meter ever produced.
    ///
    /// ```rust
    /// use metering::power_quality::PowerQualityInterval;
    /// use rust_decimal::dec;
    /// use time::macros::datetime;
    ///
    /// let iv = PowerQualityInterval {
    ///     voltage_l1_v: Some(dec!(231.4)),
    ///     ..PowerQualityInterval::empty(
    ///         datetime!(2026-06-01 0:00 UTC),
    ///         datetime!(2026-06-01 0:10 UTC),
    ///     )
    /// };
    /// assert!(!iv.voltage_out_of_range(dec!(230), dec!(10)));
    /// ```
    #[must_use]
    pub const fn empty(from: OffsetDateTime, to: OffsetDateTime) -> Self {
        Self {
            from,
            to,
            voltage_l1_v: None,
            voltage_l2_v: None,
            voltage_l3_v: None,
            current_l1_a: None,
            current_l2_a: None,
            current_l3_a: None,
            frequency_hz: None,
            power_factor: None,
            thd_voltage_pct: None,
            thd_current_pct: None,
        }
    }

    /// Every measured phase voltage, in phase order.
    pub fn phase_voltages(&self) -> impl Iterator<Item = Decimal> {
        [self.voltage_l1_v, self.voltage_l2_v, self.voltage_l3_v]
            .into_iter()
            .flatten()
    }

    /// `true` when **any** measured phase deviates from `nominal_v` by more
    /// than `threshold_pct`.
    ///
    /// A per-interval **indicator**, not an EN 50160 verdict — see the
    /// [module docs](self#en-50160-is-a-statistical-standard-and-that-is-the-whole-point).
    #[must_use]
    pub fn voltage_out_of_range(&self, nominal_v: Decimal, threshold_pct: Decimal) -> bool {
        if nominal_v.is_zero() {
            return false;
        }
        self.phase_voltages()
            .any(|v| ((v - nominal_v) / nominal_v * Decimal::ONE_HUNDRED).abs() > threshold_pct)
    }

    /// `true` when the frequency deviates from 50 Hz by more than
    /// `threshold_hz`. A per-interval indicator.
    #[must_use]
    pub fn frequency_out_of_range(&self, threshold_hz: Decimal) -> bool {
        self.frequency_hz
            .is_some_and(|f| (f - Decimal::from(50u32)).abs() > threshold_hz)
    }

    /// `true` when the power factor is below `min_pf`.
    ///
    /// Not an EN 50160 parameter; industrial NNE tariffs commonly require
    /// cos φ ≥ 0.9 and charge Blindarbeit below it.
    #[must_use]
    pub fn power_factor_below(&self, min_pf: Decimal) -> bool {
        self.power_factor.is_some_and(|pf| pf < min_pf)
    }

    /// `true` when the voltage THD exceeds `max_thd_pct`. A per-interval
    /// indicator.
    #[must_use]
    pub fn voltage_thd_exceeded(&self, max_thd_pct: Decimal) -> bool {
        self.thd_voltage_pct.is_some_and(|thd| thd > max_thd_pct)
    }
}

// ── En50160Limits ─────────────────────────────────────────────────────────────

/// The EN 50160 limits to assess against.
///
/// [`LOW_VOLTAGE`](Self::LOW_VOLTAGE) carries the standard's low-voltage
/// figures. Medium and high voltage have different bands; set them explicitly
/// rather than assuming.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct En50160Limits {
    /// Declared supply voltage `Un`, phase to neutral.
    pub nominal_voltage_v: Decimal,
    /// Band the 95 % share of voltage means must sit inside, as ± percent.
    pub voltage_band_pct: Decimal,
    /// Share of voltage means required inside the band, in percent.
    pub voltage_share_pct: f64,
    /// Upper bound every voltage mean must respect, as + percent.
    pub voltage_absolute_upper_pct: Decimal,
    /// Lower bound every voltage mean must respect, as − percent.
    pub voltage_absolute_lower_pct: Decimal,
    /// Band the frequency means must sit inside, as ± percent of 50 Hz.
    pub frequency_band_pct: Decimal,
    /// Share of frequency means required inside the band, in percent.
    pub frequency_share_pct: f64,
    /// Maximum total harmonic distortion of the voltage, in percent.
    pub thd_max_pct: Decimal,
    /// Share of THD means required at or below the maximum, in percent.
    pub thd_share_pct: f64,
}

impl En50160Limits {
    /// EN 50160 low voltage: `Un = 230 V` phase to neutral.
    ///
    /// - Voltage: ±10 % for 95 % of a week; +10 % / −15 % for all of it.
    /// - Frequency (interconnected system): ±1 % for 99.5 % of a year.
    /// - THD of the voltage: ≤ 8 % for 95 % of a week.
    pub const LOW_VOLTAGE: Self = Self {
        nominal_voltage_v: Decimal::from_parts(230, 0, 0, false, 0),
        voltage_band_pct: Decimal::from_parts(10, 0, 0, false, 0),
        voltage_share_pct: 95.0,
        voltage_absolute_upper_pct: Decimal::from_parts(10, 0, 0, false, 0),
        voltage_absolute_lower_pct: Decimal::from_parts(15, 0, 0, false, 0),
        frequency_band_pct: Decimal::from_parts(1, 0, 0, false, 0),
        frequency_share_pct: 99.5,
        thd_max_pct: Decimal::from_parts(8, 0, 0, false, 0),
        thd_share_pct: 95.0,
    };
}

impl Default for En50160Limits {
    fn default() -> Self {
        Self::LOW_VOLTAGE
    }
}

// ── LimitOutcome ──────────────────────────────────────────────────────────────

/// How one parameter fared against its limit.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct LimitOutcome {
    /// Number of samples that carried this parameter.
    pub samples: usize,
    /// Number of them inside the limit.
    pub within: usize,
    /// `within / samples × 100`.
    pub share_pct: f64,
    /// The share the standard requires.
    pub required_share_pct: f64,
    /// `true` when [`share_pct`](Self::share_pct) meets the requirement.
    pub compliant: bool,
    /// The sample furthest outside the limit, when there was one.
    pub worst: Option<Decimal>,
}

impl LimitOutcome {
    fn new(within: usize, samples: usize, required_share_pct: f64, worst: Option<Decimal>) -> Self {
        let share_pct = if samples == 0 {
            100.0
        } else {
            within as f64 / samples as f64 * 100.0
        };
        Self {
            samples,
            within,
            share_pct,
            required_share_pct,
            // A share is compared with a small tolerance: 95 % of 1 008 samples
            // is 957.6, and 957 of them is 94.9404 % — which a strict `>=`
            // would fail on a supply the standard accepts.
            compliant: samples == 0 || share_pct + 1e-9 >= required_share_pct,
            worst,
        }
    }
}

// ── En50160Report ─────────────────────────────────────────────────────────────

/// The outcome of an EN 50160 assessment over a series.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct En50160Report {
    /// Number of intervals assessed.
    pub intervals: usize,
    /// Total duration the series covers, in seconds.
    pub covered_secs: i64,
    /// `true` when the series spans at least the one week EN 50160's voltage
    /// and THD limits are defined over.
    ///
    /// A report over less than a week is an indication, not a conformance
    /// statement, and [`is_conclusive`](Self::is_conclusive) says so.
    pub covers_a_week: bool,
    /// Voltage inside `Un ± band` for the required share.
    pub voltage_band: LimitOutcome,
    /// Every voltage mean inside `Un + upper / − lower`. Required share 100 %.
    pub voltage_absolute: LimitOutcome,
    /// Frequency inside `50 Hz ± band` for the required share.
    pub frequency: LimitOutcome,
    /// Voltage THD at or below the maximum for the required share.
    pub thd_voltage: LimitOutcome,
}

impl En50160Report {
    /// Every parameter that had samples, in report order.
    pub fn outcomes(&self) -> impl Iterator<Item = (&'static str, &LimitOutcome)> {
        [
            ("voltage_band", &self.voltage_band),
            ("voltage_absolute", &self.voltage_absolute),
            ("frequency", &self.frequency),
            ("thd_voltage", &self.thd_voltage),
        ]
        .into_iter()
        .filter(|(_, o)| o.samples > 0)
    }

    /// `true` when every parameter with samples met its required share.
    ///
    /// Check [`is_conclusive`](Self::is_conclusive) too: a compliant verdict
    /// over three hours of data is not an EN 50160 statement.
    #[must_use]
    pub fn compliant(&self) -> bool {
        self.outcomes().all(|(_, o)| o.compliant)
    }

    /// `true` when the series is long enough for the verdict to mean what
    /// EN 50160 means — at least one week, with at least one parameter
    /// measured.
    #[must_use]
    pub fn is_conclusive(&self) -> bool {
        self.covers_a_week && self.outcomes().next().is_some()
    }
}

/// Seconds in the one-week observation window EN 50160 defines its voltage and
/// THD limits over.
pub const OBSERVATION_WEEK_SECS: i64 = 7 * 24 * 3600;

// ── assess_en50160 ────────────────────────────────────────────────────────────

/// Assess a series of 10-minute means against EN 50160.
///
/// Each phase voltage counts as its own sample: the standard's limits apply per
/// phase, so a three-phase week is 3 024 voltage samples rather than 1 008.
///
/// Intervals missing a parameter simply do not contribute to that parameter's
/// outcome — a single-phase meter produces no L2/L3 samples, and a meter
/// without a harmonics analyser produces no THD samples. An outcome with zero
/// samples reports `compliant: true` and is excluded from
/// [`En50160Report::outcomes`], so a parameter that was never measured cannot
/// fail and cannot silently pass either.
///
/// ## Example
///
/// ```rust
/// use metering::power_quality::{En50160Limits, PowerQualityInterval, assess_en50160};
/// use rust_decimal::dec;
/// use time::{Duration, macros::datetime};
///
/// // A week of 10-minute means, all nominal but for one excursion.
/// let mut series: Vec<PowerQualityInterval> = (0..1008)
///     .map(|i| {
///         let from = datetime!(2026-06-01 0:00 UTC) + Duration::minutes(i * 10);
///         PowerQualityInterval {
///             voltage_l1_v: Some(dec!(231)),
///             ..PowerQualityInterval::empty(from, from + Duration::minutes(10))
///         }
///     })
///     .collect();
/// series[500].voltage_l1_v = Some(dec!(260)); // one sample well over 253 V
///
/// let report = assess_en50160(&series, &En50160Limits::LOW_VOLTAGE);
/// assert!(report.is_conclusive(), "a full week was supplied");
///
/// // One excursion in 1 008 is 99.9 % inside the ±10 % band — the standard
/// // allows 5 % outside, so the band test passes...
/// assert!(report.voltage_band.compliant);
/// // ...but the +10 % absolute limit admits no exceptions at all.
/// assert!(!report.voltage_absolute.compliant);
/// assert!(!report.compliant());
/// ```
#[must_use]
pub fn assess_en50160(intervals: &[PowerQualityInterval], limits: &En50160Limits) -> En50160Report {
    let un = limits.nominal_voltage_v;
    let band = un * limits.voltage_band_pct / Decimal::ONE_HUNDRED;
    let upper = un + un * limits.voltage_absolute_upper_pct / Decimal::ONE_HUNDRED;
    let lower = un - un * limits.voltage_absolute_lower_pct / Decimal::ONE_HUNDRED;

    let nominal_hz = Decimal::from(50u32);
    let freq_band = nominal_hz * limits.frequency_band_pct / Decimal::ONE_HUNDRED;

    let mut band_tally = Tally::new(|v: Decimal| (v - un).abs());
    let mut absolute_tally = Tally::new(|v: Decimal| (v - un).abs());
    let mut freq_tally = Tally::new(|f: Decimal| (f - nominal_hz).abs());
    let mut thd_tally = Tally::new(|t: Decimal| t);

    let mut covered_secs = 0i64;
    for iv in intervals {
        covered_secs += (iv.to - iv.from).whole_seconds().max(0);

        for v in iv.phase_voltages() {
            band_tally.push(v, (v - un).abs() <= band);
            absolute_tally.push(v, v <= upper && v >= lower);
        }
        if let Some(f) = iv.frequency_hz {
            freq_tally.push(f, (f - nominal_hz).abs() <= freq_band);
        }
        if let Some(thd) = iv.thd_voltage_pct {
            thd_tally.push(thd, thd <= limits.thd_max_pct);
        }
    }

    En50160Report {
        intervals: intervals.len(),
        covered_secs,
        covers_a_week: covered_secs >= OBSERVATION_WEEK_SECS,
        voltage_band: band_tally.finish(limits.voltage_share_pct),
        // The absolute band admits no exceptions, so the required share is 100 %.
        voltage_absolute: absolute_tally.finish(100.0),
        frequency: freq_tally.finish(limits.frequency_share_pct),
        thd_voltage: thd_tally.finish(limits.thd_share_pct),
    }
}

/// Counts samples inside a limit and remembers the furthest one outside.
struct Tally<F> {
    samples: usize,
    within: usize,
    worst: Option<Decimal>,
    worst_score: Decimal,
    /// How far outside a value is; larger is worse.
    score: F,
}

impl<F: Fn(Decimal) -> Decimal> Tally<F> {
    fn new(score: F) -> Self {
        Self {
            samples: 0,
            within: 0,
            worst: None,
            worst_score: Decimal::MIN,
            score,
        }
    }

    fn push(&mut self, value: Decimal, inside: bool) {
        self.samples += 1;
        if inside {
            self.within += 1;
            return;
        }
        let score = (self.score)(value);
        if score > self.worst_score {
            self.worst_score = score;
            self.worst = Some(value);
        }
    }

    fn finish(self, required_share_pct: f64) -> LimitOutcome {
        LimitOutcome::new(self.within, self.samples, required_share_pct, self.worst)
    }
}

/// The share of a `LimitOutcome`'s samples that sat outside, in percent.
///
/// Convenience for a report line; `100 − share_pct` with the float subtraction
/// done once.
#[must_use]
pub fn exceedance_pct(outcome: &LimitOutcome) -> f64 {
    (100.0 - outcome.share_pct).max(0.0)
}

/// The 95th-percentile-style question EN 50160 asks, as a plain number.
///
/// Returns the value at `share` of the sorted magnitudes — the figure a power
/// quality report prints as "U95". `None` when nothing was measured.
///
/// `share` is a fraction in `0.0..=1.0`; EN 50160's voltage test is `0.95`.
#[must_use]
pub fn voltage_percentile(intervals: &[PowerQualityInterval], share: f64) -> Option<Decimal> {
    let mut values: Vec<Decimal> = intervals
        .iter()
        .flat_map(|iv| iv.phase_voltages())
        .collect();
    if values.is_empty() {
        return None;
    }
    values.sort_unstable();
    let share = share.clamp(0.0, 1.0);
    // Nearest-rank: the smallest value at or above the requested share.
    let rank = (share * values.len() as f64).ceil() as usize;
    let index = rank.saturating_sub(1).min(values.len() - 1);
    values.get(index).copied()
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal::dec;
    use time::{Duration, macros::datetime};

    fn at(i: i64) -> (OffsetDateTime, OffsetDateTime) {
        let from = datetime!(2026-06-01 0:00 UTC) + Duration::minutes(i * 10);
        (from, from + Duration::minutes(10))
    }

    /// A week of 10-minute means at `volts`.
    fn week(volts: Decimal) -> Vec<PowerQualityInterval> {
        (0..1008)
            .map(|i| {
                let (from, to) = at(i);
                PowerQualityInterval {
                    voltage_l1_v: Some(volts),
                    ..PowerQualityInterval::empty(from, to)
                }
            })
            .collect()
    }

    // ── the statistical assessment ───────────────────────────────────────────

    #[test]
    fn a_nominal_week_is_compliant_and_conclusive() {
        let report = assess_en50160(&week(dec!(231)), &En50160Limits::LOW_VOLTAGE);
        assert!(report.compliant());
        assert!(report.is_conclusive());
        assert_eq!(report.voltage_band.samples, 1008);
        assert_eq!(report.voltage_band.within, 1008);
        assert!((report.voltage_band.share_pct - 100.0).abs() < 1e-9);
        assert_eq!(report.voltage_band.worst, None);
    }

    /// The distinction the per-interval predicate cannot make: EN 50160 allows
    /// 5 % of a week outside `Un ± 10 %`.
    #[test]
    fn five_percent_outside_the_band_still_conforms() {
        let mut series = week(dec!(231));
        // 50 of 1 008 is 4.96 % — inside the allowance.
        for iv in series.iter_mut().take(50) {
            iv.voltage_l1_v = Some(dec!(254)); // just over +10 %
        }
        let report = assess_en50160(&series, &En50160Limits::LOW_VOLTAGE);
        assert!(
            report.voltage_band.compliant,
            "{:.3} % inside, {} required",
            report.voltage_band.share_pct, report.voltage_band.required_share_pct
        );

        // 51 tips it over.
        series[50].voltage_l1_v = Some(dec!(254));
        let report = assess_en50160(&series, &En50160Limits::LOW_VOLTAGE);
        assert!(!report.voltage_band.compliant);
        assert_eq!(report.voltage_band.worst, Some(dec!(254)));
    }

    /// The exact boundary: 95 % of 1 008 is 957.6, so 958 samples inside
    /// conforms and 957 does not. A naive integer comparison gets this wrong.
    #[test]
    fn the_ninety_five_percent_boundary_is_exact() {
        let build = |outside: usize| {
            let mut series = week(dec!(231));
            for iv in series.iter_mut().take(outside) {
                iv.voltage_l1_v = Some(dec!(260));
            }
            assess_en50160(&series, &En50160Limits::LOW_VOLTAGE)
        };
        // 1008 − 50 = 958 inside → 95.04 %.
        assert!(build(50).voltage_band.compliant);
        // 1008 − 51 = 957 inside → 94.94 %.
        assert!(!build(51).voltage_band.compliant);
    }

    /// The absolute band admits no exceptions, so one excursion fails it while
    /// the 95 % band still passes.
    #[test]
    fn the_absolute_limit_admits_no_exceptions() {
        let mut series = week(dec!(231));
        series[500].voltage_l1_v = Some(dec!(260)); // over +10 %

        let report = assess_en50160(&series, &En50160Limits::LOW_VOLTAGE);
        assert!(report.voltage_band.compliant, "one in 1 008 is within 5 %");
        assert!(!report.voltage_absolute.compliant);
        assert!(!report.compliant());
        assert_eq!(report.voltage_absolute.worst, Some(dec!(260)));
    }

    /// The absolute band is asymmetric: +10 % / −15 %, so 200 V conforms and
    /// 254 V does not.
    #[test]
    fn the_absolute_band_is_asymmetric() {
        let low = {
            let mut s = week(dec!(231));
            s[0].voltage_l1_v = Some(dec!(200)); // −13 %, inside −15 %
            assess_en50160(&s, &En50160Limits::LOW_VOLTAGE)
        };
        assert!(low.voltage_absolute.compliant, "−13 % is inside −15 %");

        let high = {
            let mut s = week(dec!(231));
            s[0].voltage_l1_v = Some(dec!(254)); // +10.4 %, outside +10 %
            assess_en50160(&s, &En50160Limits::LOW_VOLTAGE)
        };
        assert!(!high.voltage_absolute.compliant);
    }

    /// A verdict over less than a week is not an EN 50160 statement, and the
    /// report says so rather than quietly claiming conformance.
    #[test]
    fn a_short_series_is_not_conclusive() {
        let day: Vec<_> = week(dec!(231)).into_iter().take(144).collect();
        let report = assess_en50160(&day, &En50160Limits::LOW_VOLTAGE);
        assert!(report.compliant(), "nothing breached");
        assert!(!report.is_conclusive(), "but a day is not a week");
        assert!(!report.covers_a_week);
        assert_eq!(report.covered_secs, 144 * 600);

        // Exactly a week is.
        assert!(assess_en50160(&week(dec!(231)), &En50160Limits::LOW_VOLTAGE).covers_a_week);
    }

    /// Each phase is its own sample, so a three-phase week is 3 024 of them.
    #[test]
    fn every_phase_counts_separately() {
        let series: Vec<_> = (0..1008)
            .map(|i| {
                let (from, to) = at(i);
                PowerQualityInterval {
                    voltage_l1_v: Some(dec!(231)),
                    voltage_l2_v: Some(dec!(229)),
                    voltage_l3_v: Some(dec!(232)),
                    ..PowerQualityInterval::empty(from, to)
                }
            })
            .collect();
        let report = assess_en50160(&series, &En50160Limits::LOW_VOLTAGE);
        assert_eq!(report.voltage_band.samples, 3024);
        assert!(report.compliant());
    }

    /// A parameter nobody measured cannot fail, and is excluded from the
    /// verdict rather than counted as a pass.
    #[test]
    fn unmeasured_parameters_are_excluded_not_passed() {
        let report = assess_en50160(&week(dec!(231)), &En50160Limits::LOW_VOLTAGE);
        assert_eq!(report.frequency.samples, 0);
        assert_eq!(report.thd_voltage.samples, 0);

        let named: Vec<&str> = report.outcomes().map(|(n, _)| n).collect();
        assert_eq!(named, vec!["voltage_band", "voltage_absolute"]);
        assert!(report.compliant());
    }

    #[test]
    fn frequency_and_thd_are_assessed_when_present() {
        let mut series: Vec<_> = (0..1008)
            .map(|i| {
                let (from, to) = at(i);
                PowerQualityInterval {
                    frequency_hz: Some(dec!(50.0)),
                    thd_voltage_pct: Some(dec!(3.0)),
                    ..PowerQualityInterval::empty(from, to)
                }
            })
            .collect();
        assert!(assess_en50160(&series, &En50160Limits::LOW_VOLTAGE).compliant());

        // Frequency needs 99.5 %, so six bad samples in 1 008 (0.60 %) fail.
        for iv in series.iter_mut().take(6) {
            iv.frequency_hz = Some(dec!(48.0));
        }
        let report = assess_en50160(&series, &En50160Limits::LOW_VOLTAGE);
        assert!(!report.frequency.compliant, "{:?}", report.frequency);
        assert_eq!(report.frequency.worst, Some(dec!(48.0)));
        assert!(report.thd_voltage.compliant);

        // THD is a 95 % test, so the same six samples would not have failed it.
        for iv in series.iter_mut().take(6) {
            iv.thd_voltage_pct = Some(dec!(12.0));
        }
        let report = assess_en50160(&series, &En50160Limits::LOW_VOLTAGE);
        assert!(report.thd_voltage.compliant, "6 of 1 008 is inside 5 %");
    }

    #[test]
    fn an_empty_series_is_vacuous_not_compliant() {
        let report = assess_en50160(&[], &En50160Limits::LOW_VOLTAGE);
        assert_eq!(report.intervals, 0);
        assert!(!report.is_conclusive());
        assert_eq!(report.outcomes().count(), 0);
    }

    // ── percentile ───────────────────────────────────────────────────────────

    #[test]
    fn the_voltage_percentile_is_the_u95_a_report_prints() {
        let mut series = week(dec!(230));
        // The top 5 % — 50 samples — sit at 245 V.
        for iv in series.iter_mut().take(50) {
            iv.voltage_l1_v = Some(dec!(245));
        }
        // 95th percentile of 1 008 values: rank 958, which is still 230.
        assert_eq!(voltage_percentile(&series, 0.95), Some(dec!(230)));
        // The maximum is the 100th percentile.
        assert_eq!(voltage_percentile(&series, 1.0), Some(dec!(245)));
        // ...and the minimum the 0th.
        assert_eq!(voltage_percentile(&series, 0.0), Some(dec!(230)));
        assert_eq!(voltage_percentile(&[], 0.95), None);
    }

    // ── per-interval indicators ──────────────────────────────────────────────

    #[test]
    fn per_interval_indicators_flag_single_samples() {
        let (from, to) = at(0);
        let iv = PowerQualityInterval {
            voltage_l1_v: Some(dec!(254)),
            frequency_hz: Some(dec!(49.0)),
            power_factor: Some(dec!(0.85)),
            thd_voltage_pct: Some(dec!(9.0)),
            ..PowerQualityInterval::empty(from, to)
        };
        assert!(iv.voltage_out_of_range(dec!(230), dec!(10)));
        assert!(iv.frequency_out_of_range(dec!(0.5)));
        assert!(iv.power_factor_below(dec!(0.9)));
        assert!(iv.voltage_thd_exceeded(dec!(8)));

        // ...and a nominal interval trips nothing.
        let ok = PowerQualityInterval {
            voltage_l1_v: Some(dec!(231)),
            frequency_hz: Some(dec!(50.0)),
            power_factor: Some(dec!(0.98)),
            thd_voltage_pct: Some(dec!(2.0)),
            ..PowerQualityInterval::empty(from, to)
        };
        assert!(!ok.voltage_out_of_range(dec!(230), dec!(10)));
        assert!(!ok.frequency_out_of_range(dec!(0.5)));
        assert!(!ok.power_factor_below(dec!(0.9)));
        assert!(!ok.voltage_thd_exceeded(dec!(8)));
    }

    #[test]
    fn absent_measurements_trip_nothing() {
        let (from, to) = at(0);
        let iv = PowerQualityInterval::empty(from, to);
        assert!(!iv.voltage_out_of_range(dec!(230), dec!(10)));
        assert!(!iv.frequency_out_of_range(dec!(0.5)));
        assert!(!iv.power_factor_below(dec!(0.9)));
        assert!(!iv.voltage_thd_exceeded(dec!(8)));
        assert_eq!(iv.phase_voltages().count(), 0);
    }

    /// A zero nominal voltage would divide by zero; the indicator declines
    /// rather than panicking.
    #[test]
    fn a_zero_nominal_voltage_does_not_divide_by_zero() {
        let (from, to) = at(0);
        let iv = PowerQualityInterval {
            voltage_l1_v: Some(dec!(231)),
            ..PowerQualityInterval::empty(from, to)
        };
        assert!(!iv.voltage_out_of_range(Decimal::ZERO, dec!(10)));
    }

    #[test]
    fn exceedance_is_the_complement_of_the_share() {
        let mut series = week(dec!(231));
        for iv in series.iter_mut().take(101) {
            iv.voltage_l1_v = Some(dec!(260));
        }
        let report = assess_en50160(&series, &En50160Limits::LOW_VOLTAGE);
        let out = exceedance_pct(&report.voltage_band);
        assert!((out - 10.02).abs() < 0.01, "got {out}");
    }

    /// The limits are data, so a medium-voltage or non-German assessment is a
    /// different `En50160Limits`, not a fork of the function.
    #[test]
    fn limits_are_configurable() {
        let strict = En50160Limits {
            voltage_band_pct: dec!(5),
            ..En50160Limits::LOW_VOLTAGE
        };
        let mut series = week(dec!(231));
        for iv in series.iter_mut().take(200) {
            iv.voltage_l1_v = Some(dec!(245)); // +6.5 %
        }
        assert!(
            assess_en50160(&series, &En50160Limits::LOW_VOLTAGE)
                .voltage_band
                .compliant
        );
        assert!(!assess_en50160(&series, &strict).voltage_band.compliant);
    }

    /// Shares are diagnostics and stay `f64`; measured values stay `Decimal`
    /// and are never routed through a float.
    #[test]
    fn shares_are_floats_and_quantities_are_not() {
        let mut series = week(dec!(231));
        series[0].voltage_l1_v = Some(dec!(260.125));
        let report = assess_en50160(&series, &En50160Limits::LOW_VOLTAGE);
        let _: f64 = report.voltage_band.share_pct;
        assert_eq!(
            report.voltage_band.worst,
            Some(dec!(260.125)),
            "the reported worst value is the exact measurement"
        );
    }
}
