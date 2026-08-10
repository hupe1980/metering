//! Quality grading — a single A/B/C/F verdict over a validated series.
//!
//! ## What this module is
//!
//! Two things, and nothing in between:
//!
//! 1. The **Hampel filter** ([`hampel_filter`]), the robust outlier primitive
//!    that [`crate::validation`] rule V04 is built on.
//! 2. A **grader** ([`score_intervals`]) that runs the validation engine and
//!    condenses its findings into one letter, for the case where a caller has
//!    to decide "bill this or route it to review" and cannot read a list.
//!
//! ## Why it does not compute its own statistics
//!
//! It used to. There were three scorers — one over `&[MeterInterval]`, one over
//! `&[f64]`, and one over `&[f64]` plus a parallel array of nanosecond epochs —
//! each with its own copy of gap detection, zero-run counting, interval
//! consistency and coverage, and each subtly different from the validation
//! engine's copy of the same four rules. A series could be graded `A` while
//! validation reported errors on it. There is now one implementation of each
//! rule, in [`crate::validation`], and this module reads its output.
//!
//! ## The Hampel filter
//!
//! A point is an outlier when it deviates from its local **median** by more
//! than `t` robust sigma, where sigma is the median absolute deviation scaled
//! by [`K_MAD`] — the constant that makes MAD a consistent estimator of the
//! standard deviation for normally distributed data. Median and MAD both have a
//! 50 % breakdown point, so up to half the window can be corrupt without moving
//! the threshold that is meant to catch it. A mean-and-standard-deviation test
//! has a breakdown point of zero: one bad value moves both.
//!
//! ```rust
//! use metering::hampel_filter;
//!
//! let values = vec![1.0, 1.1, 1.0, 50.0, 1.0, 1.1, 1.0];
//! assert!(hampel_filter(&values, 3, 3.0).contains(&3));
//! ```
//!
//! ## The zero-MAD edge, and the floor that softens it
//!
//! When more than half a window holds the same value the MAD is exactly zero,
//! so `t × sigma` is zero and the test degenerates to *"differs from the median
//! at all"*. In the series above that flags the two `1.1`s alongside the real
//! `50.0`. This is the filter behaving as defined, not a bug — but on a
//! flat-profile medium it is useless, which is what
//! [`hampel_filter_with_floor`] exists for.

use crate::interval::{MeterInterval, Sparte};
use crate::validation::{
    ValidationConfig, ValidationIssue, ValidationRuleId, ValidationSeverity, validate_intervals,
};

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// Scales the median absolute deviation to a Gaussian-equivalent standard
/// deviation: `1 / Φ⁻¹(0.75)`.
pub const K_MAD: f64 = 1.4826;

// ── Hampel filter ─────────────────────────────────────────────────────────────

/// Indices of the values that deviate from their local median by more than
/// `t` robust sigma.
///
/// `k` is the half-window, so each point is judged against the `2k+1` values
/// centred on it, truncated at the ends of the slice.
///
/// # Example
/// ```rust
/// use metering::hampel_filter;
///
/// let values = vec![1.0, 1.1, 1.0, 50.0, 1.0, 1.1, 1.0];
/// let outliers = hampel_filter(&values, 3, 3.0);
/// assert!(outliers.contains(&3), "spike at index 3 must be detected");
/// ```
#[must_use]
pub fn hampel_filter(values: &[f64], k: usize, t: f64) -> Vec<usize> {
    hampel_filter_with_floor(values, k, t, 0.0)
}

/// [`hampel_filter`] with an absolute floor on the robust sigma.
///
/// Across a perfectly flat window the MAD is zero, so `t × sigma` is zero and
/// every nonzero deviation scores as an outlier. On a flat-profile medium — a
/// vacant flat's water meter — that flags the first genuine draw after a quiet
/// spell. See [`ValidationConfig::outlier_min_sigma`].
#[must_use]
pub fn hampel_filter_with_floor(values: &[f64], k: usize, t: f64, min_sigma: f64) -> Vec<usize> {
    let n = values.len();
    if n == 0 {
        return Vec::new();
    }
    let mut outliers = Vec::new();
    // Reused across iterations so a long series does not allocate per point.
    let mut window: Vec<f64> = Vec::with_capacity(2 * k + 1);

    for i in 0..n {
        let lo = i.saturating_sub(k);
        let hi = (i + k + 1).min(n);

        window.clear();
        window.extend_from_slice(&values[lo..hi]);
        let median = median_of(&mut window);

        // Reuse the same buffer for the absolute deviations.
        for v in &mut window {
            *v = (*v - median).abs();
        }
        let mad = median_of(&mut window);

        let sigma = (K_MAD * mad).max(min_sigma);
        let deviation = (values[i] - median).abs();
        // With sigma pinned at zero — a flat window and no floor — any movement
        // at all counts, so the test degenerates to "differs from the median".
        // That is the behaviour the floor exists to soften.
        let is_outlier = if sigma <= 0.0 {
            deviation > 0.0
        } else {
            deviation > t * sigma
        };
        if is_outlier {
            outliers.push(i);
        }
    }
    outliers
}

/// Median of `buf`, sorting it in place.
///
/// `f64` has no total order, so `sort_unstable_by` needs a comparator. NaN
/// cannot reach here from `Decimal` conversion, and if it did it would sort to
/// one end rather than panic.
fn median_of(buf: &mut [f64]) -> f64 {
    if buf.is_empty() {
        return 0.0;
    }
    buf.sort_unstable_by(f64::total_cmp);
    let mid = buf.len() / 2;
    if buf.len().is_multiple_of(2) {
        f64::midpoint(buf[mid - 1], buf[mid])
    } else {
        buf[mid]
    }
}

// ── QualityGrade ──────────────────────────────────────────────────────────────

/// Quality grade: A / B / C / F.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum QualityGrade {
    /// Clean — no findings. Bill it.
    A,
    /// Minor findings, none of them blocking. Bill it, note the caveat.
    B,
    /// Blocking findings, few enough to fix by hand. Route to review.
    C,
    /// Unusable as delivered — block automated billing.
    F,
}

impl QualityGrade {
    /// Every grade, best first.
    pub const ALL: [Self; 4] = [Self::A, Self::B, Self::C, Self::F];

    /// `true` when this grade blocks automated billing.
    #[must_use]
    pub const fn blocks_billing(self) -> bool {
        matches!(self, Self::F)
    }

    /// Grade as a static string.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::A => "A",
            Self::B => "B",
            Self::C => "C",
            Self::F => "F",
        }
    }
}

impl std::fmt::Display for QualityGrade {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

// ── QualityConfig ─────────────────────────────────────────────────────────────

/// Configuration for [`score_intervals`]: the rules to apply, plus the one
/// tolerance that is a grading question rather than a validation one.
#[derive(Debug, Clone, PartialEq)]
pub struct QualityConfig {
    /// The validation rules to run. Grading is a summary of their output, so
    /// every threshold that decides *whether something is a finding* lives
    /// here rather than being duplicated.
    pub validation: ValidationConfig,

    /// Longest run of consecutive zero intervals that is not itself a downgrade.
    ///
    /// Separate from [`ValidationConfig::zero_run_threshold`] because the two
    /// answer different questions: validation asks "is this worth reporting",
    /// grading asks "is this bad enough to withhold an A". Electricity has a
    /// standby floor, so a short zero run means a dead meter; water and heat
    /// have none, and an empty flat reads zero for weeks.
    pub max_zero_run_allowed: usize,

    /// Coverage below which the grade cannot be better than `C`.
    ///
    /// Default: `99.0`.
    pub min_coverage_pct: f64,
}

impl Default for QualityConfig {
    fn default() -> Self {
        Self {
            validation: ValidationConfig::default(),
            max_zero_run_allowed: 2,
            min_coverage_pct: 99.0,
        }
    }
}

impl QualityConfig {
    /// Media-aware defaults.
    ///
    /// The electricity thresholds suit 15-minute RLM load profiles, which are
    /// noisy and rarely flat. Heat and water submetering profiles are dominated
    /// by long legitimate zero runs and need both a wider zero tolerance and a
    /// sigma floor, or the first draw after a quiet spell reads as an outlier.
    #[must_use]
    pub fn for_sparte(sparte: Sparte) -> Self {
        let with = |zero_run: usize, min_sigma: f64, interval_secs: u32| Self {
            validation: ValidationConfig {
                expected_interval_secs: Some(interval_secs),
                outlier_min_sigma: min_sigma,
                // Kept in step with `max_zero_run_allowed`: a run the grader is
                // told to tolerate must not still be reported as a finding, or
                // the tolerance only ever downgrades A to B.
                zero_run_threshold: zero_run + 1,
                ..ValidationConfig::default()
            },
            max_zero_run_allowed: zero_run,
            min_coverage_pct: 99.0,
        };
        match sparte {
            Sparte::Strom => Self::default(),
            // Gas heating is seasonal: a summer week of near-zero draw is normal.
            Sparte::Gas => with(48, 0.01, 3600),
            // Heat: unheated months are ordinary, and the resolution is coarse.
            Sparte::Waerme => with(720, 0.05, 3600),
            // Water: a vacant flat reads exactly zero indefinitely, and the
            // resolution is litres, so the sigma floor must be small.
            Sparte::Wasser => with(720, 0.001, 86_400),
        }
    }

    /// Declare the period the series must cover — see
    /// [`ValidationConfig::period`]. Without it, coverage is measured against
    /// the extent of the data itself and a truncated delivery reads as 100 %.
    #[must_use]
    pub fn over_period(mut self, from: time::OffsetDateTime, to: time::OffsetDateTime) -> Self {
        self.validation = self.validation.over_period(from, to);
        self
    }
}

// ── QualityReport ─────────────────────────────────────────────────────────────

/// The graded outcome of validating a series, with the findings that produced it.
#[derive(Debug, Clone)]
pub struct QualityReport {
    /// Number of intervals analysed.
    pub intervals_analysed: usize,
    /// Overall grade.
    pub grade: QualityGrade,
    /// Coverage: covered duration ÷ period duration × 100, capped at 100.
    pub coverage_pct: f64,
    /// Longest run of consecutive zero-value intervals, in timestamp order.
    pub max_zero_run: usize,
    /// Number of V01 gap findings.
    pub gaps_detected: usize,
    /// Number of V04 statistical-outlier findings.
    pub outliers_detected: usize,
    /// Number of findings that block billing (`Error` severity).
    pub blocking_findings: usize,
    /// `true` when every interval has the same duration.
    pub intervals_consistent: bool,
    /// The findings themselves, so a caller can act on a grade rather than
    /// merely record it.
    pub issues: Vec<ValidationIssue>,
}

impl QualityReport {
    /// `true` when the grade blocks automated billing.
    #[must_use]
    pub fn blocks_billing(&self) -> bool {
        self.grade.blocks_billing()
    }

    /// `true` when anything at all was found.
    #[must_use]
    pub fn has_warnings(&self) -> bool {
        !self.issues.is_empty()
    }
}

// ── score_intervals ───────────────────────────────────────────────────────────

/// Validate a series and condense the result into a [`QualityGrade`].
///
/// ## Grading
///
/// | Grade | Condition |
/// |---|---|
/// | `A` | no findings, coverage at or above the configured minimum |
/// | `B` | findings, but none blocking and coverage still adequate |
/// | `C` | at most three blocking findings |
/// | `F` | more than three blocking findings, or an empty series |
///
/// The distinction that matters is `B` versus `C`: `B` is "bill it and note
/// it", `C` is "somebody has to look". That maps onto validation severity —
/// `Warning` versus `Error` — rather than onto a count of anomalies, so a
/// series with twenty spike warnings and no gaps still bills.
///
/// An empty series grades `F`: there is nothing to bill and nothing to
/// substitute from.
///
/// # Example
///
/// ```rust
/// use metering::{score_intervals, QualityConfig, QualityGrade, MeterInterval, QualityFlag};
/// use rust_decimal::dec;
/// use time::macros::datetime;
///
/// let samples: Vec<MeterInterval> = (0..96).map(|i| MeterInterval {
///     from: datetime!(2026-01-01 0:00 UTC) + time::Duration::minutes(i * 15),
///     to:   datetime!(2026-01-01 0:00 UTC) + time::Duration::minutes(i * 15 + 15),
///     value: dec!(2.0),
///     quality: QualityFlag::Measured,
///     obis_code: None,
/// }).collect();
///
/// let report = score_intervals(&samples, &QualityConfig::default());
/// assert_eq!(report.grade, QualityGrade::A);
/// assert!(!report.blocks_billing());
/// ```
#[must_use]
pub fn score_intervals(samples: &[MeterInterval], cfg: &QualityConfig) -> QualityReport {
    let result = validate_intervals(samples, &cfg.validation);
    let issues = result.issues;

    if samples.is_empty() {
        return QualityReport {
            intervals_analysed: 0,
            grade: QualityGrade::F,
            coverage_pct: 0.0,
            max_zero_run: 0,
            gaps_detected: issues
                .iter()
                .filter(|i| i.rule_id == ValidationRuleId::GapDetected)
                .count(),
            outliers_detected: 0,
            blocking_findings: issues
                .iter()
                .filter(|i| i.severity == ValidationSeverity::Error)
                .count(),
            intervals_consistent: true,
            issues,
        };
    }

    let count = |rule: ValidationRuleId| issues.iter().filter(|i| i.rule_id == rule).count();
    let gaps_detected = count(ValidationRuleId::GapDetected);
    let outliers_detected = count(ValidationRuleId::StatisticalOutlier);
    let intervals_consistent = count(ValidationRuleId::InconsistentIntervalLength) == 0;
    let blocking_findings = issues
        .iter()
        .filter(|i| i.severity == ValidationSeverity::Error)
        .count();

    let max_zero_run = longest_zero_run(samples);
    let coverage_pct = coverage_pct(samples, &cfg.validation);

    let coverage_ok = coverage_pct >= cfg.min_coverage_pct;
    let zero_run_ok = max_zero_run <= cfg.max_zero_run_allowed;

    let grade = if issues.is_empty() && coverage_ok && zero_run_ok {
        QualityGrade::A
    } else if blocking_findings == 0 && coverage_ok && zero_run_ok {
        QualityGrade::B
    } else if blocking_findings <= 3 && coverage_pct >= 95.0 {
        QualityGrade::C
    } else {
        QualityGrade::F
    };

    QualityReport {
        intervals_analysed: samples.len(),
        grade,
        coverage_pct,
        max_zero_run,
        gaps_detected,
        outliers_detected,
        blocking_findings,
        intervals_consistent,
        issues,
    }
}

/// Longest run of consecutive zero values, in timestamp order.
fn longest_zero_run(samples: &[MeterInterval]) -> usize {
    let mut order: Vec<&MeterInterval> = samples.iter().collect();
    order.sort_by_key(|iv| iv.from);
    let mut run = 0usize;
    let mut longest = 0usize;
    for iv in order {
        if iv.value.is_zero() {
            run += 1;
            longest = longest.max(run);
        } else {
            run = 0;
        }
    }
    longest
}

/// Share of the period actually covered by intervals, in percent.
///
/// Measured as **covered duration over period duration**, not as a count of
/// intervals over an expected count. A duration ratio is right at every
/// resolution and across both DST transitions without knowing either; a count
/// ratio needs an expected count, and the expected count for a German day is
/// 92, 96 or 100 depending on the date.
///
/// The period is [`ValidationConfig::period`] when set, and the extent of the
/// data otherwise — in which case the answer is 100 % minus the interior gaps,
/// because data cannot be missing from outside its own extent.
fn coverage_pct(samples: &[MeterInterval], cfg: &ValidationConfig) -> f64 {
    let covered: i64 = samples
        .iter()
        .map(|iv| (iv.to - iv.from).whole_seconds().max(0))
        .sum();

    let period_secs = match cfg.period {
        Some((from, to)) => (to - from).whole_seconds(),
        None => {
            let first = samples.iter().map(|iv| iv.from).min();
            let last = samples.iter().map(|iv| iv.to).max();
            match (first, last) {
                (Some(f), Some(l)) => (l - f).whole_seconds(),
                _ => 0,
            }
        }
    };

    if period_secs <= 0 {
        return if covered > 0 { 100.0 } else { 0.0 };
    }
    ((covered as f64 / period_secs as f64) * 100.0).clamp(0.0, 100.0)
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod hampel_tests {
    use super::*;

    #[test]
    fn detects_a_single_spike() {
        let values = vec![1.0, 1.1, 1.0, 50.0, 1.0, 1.1, 1.0];
        assert!(hampel_filter(&values, 3, 3.0).contains(&3));
    }

    /// Over a window where more than half the values are identical the MAD is
    /// zero, so the threshold collapses and every departure from the median —
    /// including a 0.1 wobble — is an outlier. Pinned because it is surprising,
    /// and because it is the motivation for `min_sigma`.
    #[test]
    fn a_zero_mad_window_flags_every_departure() {
        let values = vec![1.0, 1.1, 1.0, 50.0, 1.0, 1.1, 1.0];
        assert_eq!(hampel_filter(&values, 3, 3.0), vec![1, 3, 5]);

        // A floor of 0.5 kWh leaves only the value that genuinely is one.
        assert_eq!(hampel_filter_with_floor(&values, 3, 3.0, 0.5), vec![3]);
    }

    /// The property that makes the median/MAD pair the right choice: up to half
    /// the window can be corrupt without the threshold moving to hide it. A
    /// mean-and-sd test fails this outright.
    #[test]
    fn a_run_of_spikes_cannot_hide_itself() {
        let mut values = vec![1.0_f64; 41];
        for v in values.iter_mut().skip(18).take(5) {
            *v = 100.0;
        }
        let flagged = hampel_filter(&values, 10, 3.0);
        assert!(
            (18..23).all(|i| flagged.contains(&i)),
            "all five spikes must be flagged, got {flagged:?}"
        );

        // The same data through a mean/sd test: sd is inflated by the spikes.
        let mean = values.iter().sum::<f64>() / values.len() as f64;
        let sd = (values.iter().map(|v| (v - mean).powi(2)).sum::<f64>()
            / (values.len() as f64 - 1.0))
            .sqrt();
        assert!(
            values[20] < mean + 3.0 * sd,
            "a 3-sigma mean test lets a 100x spike through at mean {mean:.2}, sd {sd:.2}"
        );
    }

    /// Across a flat window MAD is zero, so without a floor every nonzero
    /// deviation is an outlier.
    #[test]
    fn sigma_floor_stops_mad_implosion_on_a_flat_series() {
        let values = vec![0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.012, 0.0, 0.0, 0.0, 0.0];

        assert!(
            !hampel_filter(&values, 3, 3.0).is_empty(),
            "without a floor the flat window flags the real draw"
        );
        assert!(
            hampel_filter_with_floor(&values, 3, 3.0, 0.05).is_empty(),
            "a 12 L draw is below the 50 L floor and must not be an outlier"
        );
    }

    #[test]
    fn a_clean_series_has_no_outliers() {
        let values: Vec<f64> = (0..50).map(|i| 2.0 + (i % 5) as f64 * 0.01).collect();
        assert!(hampel_filter(&values, 5, 3.0).is_empty());
    }

    #[test]
    fn degenerate_inputs_do_not_panic() {
        assert!(hampel_filter(&[], 3, 3.0).is_empty());
        assert!(hampel_filter(&[1.0], 0, 3.0).is_empty());
        // A window wider than the slice truncates rather than reading past it.
        assert!(hampel_filter(&[1.0, 1.0, 1.0], 100, 3.0).is_empty());
    }

    /// The even/odd median split is a classic off-by-one; pin both.
    #[test]
    fn medians_handle_both_parities() {
        assert_eq!(median_of(&mut [3.0, 1.0, 2.0]), 2.0);
        assert_eq!(median_of(&mut [4.0, 1.0, 3.0, 2.0]), 2.5);
        assert_eq!(median_of(&mut []), 0.0);
    }
}

#[cfg(test)]
mod grading_tests {
    use super::*;
    use crate::interval::QualityFlag;
    use rust_decimal::{Decimal, dec};
    use time::macros::datetime;
    use time::{Duration, OffsetDateTime};

    fn iv(from: OffsetDateTime, kwh: Decimal) -> MeterInterval {
        MeterInterval {
            from,
            to: from + Duration::minutes(15),
            value: kwh,
            quality: QualityFlag::Measured,
            obis_code: None,
        }
    }

    fn clean_series(n: i64) -> Vec<MeterInterval> {
        let base = datetime!(2026-01-01 0:00 UTC);
        (0..n)
            .map(|i| iv(base + Duration::minutes(i * 15), dec!(2.0)))
            .collect()
    }

    #[test]
    fn a_clean_day_grades_a() {
        let report = score_intervals(&clean_series(96), &QualityConfig::default());
        assert_eq!(report.grade, QualityGrade::A);
        assert!(!report.has_warnings());
        assert!(!report.blocks_billing());
        assert_eq!(report.intervals_analysed, 96);
        assert!((report.coverage_pct - 100.0).abs() < 1e-9);
        assert!(report.intervals_consistent);
    }

    #[test]
    fn an_empty_series_grades_f() {
        let report = score_intervals(&[], &QualityConfig::default());
        assert_eq!(report.grade, QualityGrade::F);
        assert!(report.blocks_billing());
        assert_eq!(report.intervals_analysed, 0);
    }

    /// A gap is an Error, so it costs the A **and** the B.
    #[test]
    fn a_gap_drops_the_grade_below_b() {
        let mut samples = clean_series(96);
        samples.remove(48);
        let report = score_intervals(&samples, &QualityConfig::default());
        assert_eq!(report.gaps_detected, 1);
        assert_eq!(report.blocking_findings, 1);
        assert_eq!(report.grade, QualityGrade::C);
    }

    /// Warnings alone never block billing — the B/C line is severity, not count.
    #[test]
    fn many_warnings_still_bill() {
        let mut samples = clean_series(96);
        // Twenty spikes: all V04 warnings, no errors.
        for s in samples.iter_mut().step_by(5) {
            s.value = dec!(9);
        }
        let report = score_intervals(&samples, &QualityConfig::default());
        assert!(
            report.outliers_detected > 1,
            "{:?}",
            report.outliers_detected
        );
        assert_eq!(report.blocking_findings, 0);
        assert_eq!(report.grade, QualityGrade::B);
        assert!(!report.blocks_billing());
    }

    /// The grade and the findings must agree: nothing may grade `A` while
    /// validation reports an error on the same data. This is the invariant the
    /// three separate scorers could not hold.
    #[test]
    fn an_a_grade_never_contradicts_the_validator() {
        let base = datetime!(2026-01-01 0:00 UTC);
        let cases: Vec<Vec<MeterInterval>> = vec![
            clean_series(96),
            {
                let mut s = clean_series(96);
                s.remove(10);
                s
            },
            {
                let mut s = clean_series(96);
                s[10].quality = QualityFlag::Faulty;
                s
            },
            {
                let mut s = clean_series(96);
                s[10].value = dec!(-5);
                s
            },
            vec![iv(base, dec!(1.0))],
        ];
        for samples in cases {
            let cfg = QualityConfig::default();
            let report = score_intervals(&samples, &cfg);
            let validation = crate::validation::validate_intervals(&samples, &cfg.validation);
            if report.grade == QualityGrade::A {
                assert!(
                    validation.is_clean(),
                    "graded A but validation found {:?}",
                    validation.issues
                );
            }
            assert_eq!(report.blocking_findings, validation.billing_block_count());
        }
    }

    /// Coverage against a declared period is the only way to see a truncated
    /// delivery — the reason the field exists.
    #[test]
    fn coverage_is_measured_against_the_declared_period() {
        // Half a day delivered where a full day was due.
        let samples = clean_series(48);

        let unscoped = score_intervals(&samples, &QualityConfig::default());
        assert!(
            (unscoped.coverage_pct - 100.0).abs() < 1e-9,
            "without a period the data defines its own extent"
        );

        let cfg = QualityConfig::default().over_period(
            datetime!(2026-01-01 0:00 UTC),
            datetime!(2026-01-02 0:00 UTC),
        );
        let scoped = score_intervals(&samples, &cfg);
        assert!(
            (scoped.coverage_pct - 50.0).abs() < 1e-9,
            "got {}",
            scoped.coverage_pct
        );
        assert_eq!(scoped.grade, QualityGrade::F, "half a day is not billable");
    }

    /// Coverage as a duration ratio is right on a DST day without being told
    /// which one it is: 100 quarter-hours over a 25-hour day is 100 %.
    #[test]
    fn coverage_is_correct_across_both_dst_transitions() {
        use time::macros::date;
        for (day, expected_intervals) in [
            (date!(2026 - 03 - 29), 92i64),
            (date!(2026 - 10 - 25), 100i64),
            (date!(2026 - 07 - 20), 96i64),
        ] {
            let start = crate::calendar::day_start_utc(day);
            let end = crate::calendar::day_end_utc(day);
            let samples: Vec<_> = (0..expected_intervals)
                .map(|i| iv(start + Duration::minutes(i * 15), dec!(1.0)))
                .collect();
            let cfg = QualityConfig::default().over_period(start, end);
            let report = score_intervals(&samples, &cfg);
            assert!(
                (report.coverage_pct - 100.0).abs() < 1e-9,
                "{day}: {expected_intervals} intervals must be full coverage, got {}",
                report.coverage_pct
            );
            assert_eq!(report.grade, QualityGrade::A, "{day}: {:?}", report.issues);
        }

        // 96 intervals on the 25-hour autumn day is four short, not complete.
        let day = date!(2026 - 10 - 25);
        let start = crate::calendar::day_start_utc(day);
        let samples: Vec<_> = (0..96)
            .map(|i| iv(start + Duration::minutes(i * 15), dec!(1.0)))
            .collect();
        let cfg = QualityConfig::default().over_period(start, crate::calendar::day_end_utc(day));
        let report = score_intervals(&samples, &cfg);
        assert!(report.coverage_pct < 100.0, "got {}", report.coverage_pct);
        assert_ne!(report.grade, QualityGrade::A);
    }

    /// A vacant flat's water series is normal for water and a dead meter for
    /// electricity — the same data, two verdicts.
    #[test]
    fn zero_run_tolerance_is_media_specific() {
        let base = datetime!(2026-01-01 0:00 UTC);
        let daily: Vec<MeterInterval> = (0..24)
            .map(|i| MeterInterval {
                from: base + Duration::days(i),
                to: base + Duration::days(i + 1),
                value: Decimal::ZERO,
                quality: QualityFlag::Measured,
                obis_code: None,
            })
            .collect();

        let water = score_intervals(&daily, &QualityConfig::for_sparte(Sparte::Wasser));
        assert_eq!(water.max_zero_run, 24);
        assert_eq!(
            water.grade,
            QualityGrade::A,
            "a vacant flat reading zero is normal: {:?}",
            water.issues
        );

        let strom = score_intervals(&daily, &QualityConfig::for_sparte(Sparte::Strom));
        assert_ne!(
            strom.grade,
            QualityGrade::A,
            "24 zero days on electricity means a dead meter"
        );

        assert_eq!(
            QualityConfig::for_sparte(Sparte::Strom).max_zero_run_allowed,
            2
        );
        assert_eq!(
            QualityConfig::for_sparte(Sparte::Gas).max_zero_run_allowed,
            48
        );
        assert_eq!(
            QualityConfig::for_sparte(Sparte::Waerme).max_zero_run_allowed,
            720
        );
    }

    #[test]
    fn grade_ordering_is_best_to_worst() {
        assert!(QualityGrade::A < QualityGrade::B);
        assert!(QualityGrade::B < QualityGrade::C);
        assert!(QualityGrade::C < QualityGrade::F);
        assert!(QualityGrade::F.blocks_billing());
        for g in QualityGrade::ALL {
            assert_eq!(g.blocks_billing(), g == QualityGrade::F);
            assert!(!g.as_str().is_empty());
            assert_eq!(g.to_string(), g.as_str());
        }
    }
}
