//! Validation engine for meter interval time series.
//!
//! ## Rules
//!
//! | Rule | ID | What it catches |
//! |---|---|---|
//! | Gap | V01 | A missing interval, including before the first and after the last |
//! | Overlap | V02 | Two intervals covering the same instant |
//! | Negative energy | V03 | A value below zero on a single-direction meter |
//! | Statistical outlier | V04 | A value far from its neighbours, by a robust (Hampel) test |
//! | Zero run | V05 | A run of zeros long enough to suggest a stuck meter |
//! | Interval length | V06 | An interval that is not the expected length |
//! | Collapsed DST hour | V07 | A fall-back day carrying 24 hours instead of 25 |
//! | Future timestamp | V08 | An interval starting after the supplied reference instant |
//! | Non-billable quality | V09 | `Faulty` or `Unknown`, which must not be billed |
//! | Implausible power | V12 | Average power above the plant's physical capacity |
//! | Unordered series | V11 | Input was not ascending by `from` — usually a broken merge |
//!
//! **V10 does not exist.** It used to be a "register rollover" rule that
//! compared consecutive `value` and flagged a drop of more than 50 000 kWh.
//! A [`MeterInterval`] carries the energy *in* one interval, not a cumulative
//! Zählerstand, so the comparison was meaningless: for it to fire, one
//! quarter-hour would have had to carry 50 MWh — 200 MW of average load. A
//! rollover is a property of a meter register and is detected where register
//! readings live, not here. The number is left unused rather than recycled, so
//! a stored `V10` row cannot be silently reinterpreted as something else.
//!
//! ## Timestamps are UTC
//!
//! Every interval boundary is a UTC instant, per EDI@Energy *Allgemeine
//! Festlegungen* Kap. 3: the wire format is UTC and the process times are
//! gesetzliche deutsche Zeit. The one rule that reasons about local time is
//! V07, which is about a series that lost the distinction.
//!
//! ## Order independence
//!
//! Every adjacency rule is evaluated in timestamp order whatever order the
//! caller supplies, so shuffled input cannot produce spurious gaps or overlaps.
//! The disorder itself is reported once as [`ValidationRuleId::UnorderedSeries`],
//! and every `interval_index` still points into the caller's slice.

use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive as _;
use time::{Duration, OffsetDateTime};

use crate::interval::MeterInterval;
use time_tz::{OffsetDateTimeExt as _, timezones};

// ── ValidationSeverity ────────────────────────────────────────────────────────

/// Severity level of a validation finding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "SCREAMING_SNAKE_CASE"))]
pub enum ValidationSeverity {
    /// Informational — no action required, but worth noting.
    Info,
    /// Warning — value may be usable for billing but should be reviewed.
    Warning,
    /// Error — value must NOT be used for billing; substitute value required.
    Error,
}

// ── ValidationRuleId ─────────────────────────────────────────────────────────

/// Identifies which validation rule triggered an issue.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "SCREAMING_SNAKE_CASE"))]
pub enum ValidationRuleId {
    /// V01 — an expected interval is missing.
    GapDetected,
    /// V02 — two intervals cover the same instant.
    OverlapDetected,
    /// V03 — consumption value is negative (impossible for Bezug-only meters).
    NegativeEnergy,
    /// V04 — the value is a statistical outlier against its own neighbourhood.
    StatisticalOutlier,
    /// V05 — consecutive zero values suggest a stuck / frozen meter.
    SuspiciousZeroRun,
    /// V06 — interval length differs from the expected granularity.
    InconsistentIntervalLength,
    /// V07 — the DST fall-back hour was collapsed (local time leaked in).
    DstAmbiguity,
    /// V08 — interval starts after the reference instant.
    FutureTimestamp,
    /// V09 — quality flag is non-billable (`Faulty` or `Unknown`).
    NonBillableQuality,
    /// V11 — the series was not sorted ascending by `from`.
    ///
    /// Reported once per call. The remaining rules are evaluated in timestamp
    /// order regardless, so their findings stay correct; this says the *input*
    /// was out of order, which is itself a defect worth surfacing — an MSCONS
    /// series arriving shuffled usually means a broken merge upstream.
    UnorderedSeries,
    /// V12 — average power over the interval exceeds the plant's capacity.
    ///
    /// Unlike [`StatisticalOutlier`](Self::StatisticalOutlier), which compares a
    /// value against its neighbours, this compares it against a physical
    /// ceiling the metered plant cannot exceed. A value above it is not
    /// unusual, it is impossible — hence `Error` rather than `Warning`.
    ImplausiblePower,
}

impl ValidationRuleId {
    /// Every rule, in code order.
    pub const ALL: [Self; 11] = [
        Self::GapDetected,
        Self::OverlapDetected,
        Self::NegativeEnergy,
        Self::StatisticalOutlier,
        Self::SuspiciousZeroRun,
        Self::InconsistentIntervalLength,
        Self::DstAmbiguity,
        Self::FutureTimestamp,
        Self::NonBillableQuality,
        Self::UnorderedSeries,
        Self::ImplausiblePower,
    ];

    /// The `Vnn` code, as it appears in logs and stored findings.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::GapDetected => "V01",
            Self::OverlapDetected => "V02",
            Self::NegativeEnergy => "V03",
            Self::StatisticalOutlier => "V04",
            Self::SuspiciousZeroRun => "V05",
            Self::InconsistentIntervalLength => "V06",
            Self::DstAmbiguity => "V07",
            Self::FutureTimestamp => "V08",
            Self::NonBillableQuality => "V09",
            Self::UnorderedSeries => "V11",
            Self::ImplausiblePower => "V12",
        }
    }
}

impl std::fmt::Display for ValidationRuleId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.code())
    }
}

// ── ValidationIssue ──────────────────────────────────────────────────────────

/// A single validation finding on a meter interval or time series.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ValidationIssue {
    /// Which validation rule triggered this issue.
    pub rule_id: ValidationRuleId,
    /// Severity: `Info`, `Warning`, or `Error`.
    pub severity: ValidationSeverity,
    /// Human-readable description of the issue.
    pub message: String,
    /// Index into the validated slice where the issue was found.
    ///
    /// `None` for a finding that is not about an interval the caller supplied —
    /// a gap before the first one, for instance.
    pub interval_index: Option<usize>,
    /// The instant the finding is anchored at.
    pub affected_from: Option<OffsetDateTime>,
    /// The measured value at the affected interval, when there is one.
    pub affected_value: Option<Decimal>,
}

impl ValidationIssue {
    fn new(
        rule_id: ValidationRuleId,
        severity: ValidationSeverity,
        message: impl Into<String>,
    ) -> Self {
        Self {
            rule_id,
            severity,
            message: message.into(),
            interval_index: None,
            affected_from: None,
            affected_value: None,
        }
    }

    fn at(mut self, idx: usize, interval: &MeterInterval) -> Self {
        self.interval_index = Some(idx);
        self.affected_from = Some(interval.from);
        self.affected_value = Some(interval.value);
        self
    }

    fn anchored_at(mut self, from: OffsetDateTime) -> Self {
        self.affected_from = Some(from);
        self
    }

    /// `true` when this issue prevents the interval from being billed.
    #[must_use]
    pub fn blocks_billing(&self) -> bool {
        self.severity == ValidationSeverity::Error
    }
}

// ── ValidationConfig ─────────────────────────────────────────────────────────

/// Configuration for [`validate_intervals`].
#[derive(Debug, Clone, PartialEq)]
pub struct ValidationConfig {
    /// Expected interval duration in seconds (e.g. `900` = 15 min).
    ///
    /// `None` disables both the length check (V06) and gap detection (V01),
    /// which cannot say what is missing without knowing the grid.
    pub expected_interval_secs: Option<u32>,

    /// The period the series is supposed to cover, as a half-open UTC range.
    ///
    /// When set, V01 also reports intervals missing **before the first and
    /// after the last** supplied one. Without it, gap detection sees only the
    /// holes *between* intervals — so a month whose last week never arrived
    /// validates clean, which is the failure mode that matters most at billing
    /// time.
    pub period: Option<(OffsetDateTime, OffsetDateTime)>,

    /// V04 threshold in robust-sigma units, or `None` to disable the check.
    ///
    /// The test is a Hampel identifier: a value is an outlier when it deviates
    /// from its local **median** by more than `t × 1.4826 × MAD`. Median and
    /// MAD both have a 50 % breakdown point, so — unlike a mean-based test — a
    /// spike cannot inflate the threshold that is meant to catch it.
    ///
    /// Default: `6.0`, deliberately loose. Load profiles are not Gaussian and a
    /// three-sigma rule flags every legitimate morning ramp.
    pub outlier_sigma: Option<f64>,

    /// Half-window for the V04 median, in intervals (total window `2k+1`).
    ///
    /// Default: `12` — three hours either side at quarter-hour resolution,
    /// wide enough to have a stable median and narrow enough to track the
    /// daily shape rather than average it away.
    pub outlier_window: usize,

    /// Absolute floor on the V04 robust sigma, in kWh.
    ///
    /// Across a perfectly flat window the MAD is zero, so `t × sigma` is zero
    /// and *any* nonzero deviation scores as an outlier. On a flat-profile
    /// medium — a vacant flat's water meter, an unheated circuit — that flags
    /// the first genuine consumption after a quiet spell. The floor turns the
    /// test into "deviates by more than `min_sigma`".
    ///
    /// Default: `0.0`, which suits electricity. See
    /// [`QualityConfig::for_sparte`](crate::QualityConfig::for_sparte) for the
    /// media-specific values.
    pub outlier_min_sigma: f64,

    /// Number of consecutive zero-value intervals that triggers V05.
    ///
    /// Default: `4` — one hour at quarter-hour granularity.
    pub zero_run_threshold: usize,

    /// Treat negative energy as an Error (V03).
    ///
    /// Set to `false` for a bidirectional register, where a net-metered value
    /// legitimately goes below zero. Default: `true`.
    pub negative_energy_is_error: bool,

    /// Reference instant for V08. `None` disables the check.
    ///
    /// A parameter rather than a clock read — see the crate-level
    /// **Determinism** section.
    pub now: Option<OffsetDateTime>,

    /// Physical capacity ceiling in kW for V12.
    ///
    /// Nameplate capacity or Anschlussleistung. A value whose average power
    /// over its own interval exceeds this is physically impossible for the
    /// metered plant. `None` disables the check.
    pub max_plant_power_kw: Option<Decimal>,
}

impl Default for ValidationConfig {
    fn default() -> Self {
        Self {
            expected_interval_secs: Some(900), // 15 minutes
            period: None,
            outlier_sigma: Some(6.0),
            outlier_window: 12,
            outlier_min_sigma: 0.0,
            zero_run_threshold: 4,
            negative_energy_is_error: true,
            now: None,
            max_plant_power_kw: None,
        }
    }
}

impl ValidationConfig {
    /// Configuration for 15-minute RLM / iMSys electricity Bezug meters.
    #[must_use]
    pub fn rlm_strom_15min() -> Self {
        Self::default()
    }

    /// Configuration for hourly gas intervals.
    #[must_use]
    pub fn gas_hourly() -> Self {
        Self {
            expected_interval_secs: Some(3600),
            // Three hours either side at hourly resolution would be a 7-point
            // window; gas draw is smoother, so a day-wide median is stabler.
            outlier_window: 12,
            ..Self::default()
        }
    }

    /// Configuration for a bidirectional register, where negative values are
    /// legitimate.
    #[must_use]
    pub fn bidirectional() -> Self {
        Self {
            negative_energy_is_error: false,
            ..Self::default()
        }
    }

    /// Disable the statistical outlier check (V04).
    ///
    /// Appropriate for industrial loads whose genuine step changes would
    /// otherwise be reported on every shift start.
    #[must_use]
    pub fn without_outlier_detection(mut self) -> Self {
        self.outlier_sigma = None;
        self
    }

    /// Set the physical capacity ceiling (kW) for V12.
    #[must_use]
    pub fn with_plant_capacity_kw(mut self, kw: Decimal) -> Self {
        self.max_plant_power_kw = Some(kw);
        self
    }

    /// Declare the period the series must cover, extending V01 to the head and
    /// tail of the series.
    #[must_use]
    pub fn over_period(mut self, from: OffsetDateTime, to: OffsetDateTime) -> Self {
        self.period = Some((from, to));
        self
    }
}

// ── Validation result ─────────────────────────────────────────────────────────

/// Result of validating a slice of meter intervals.
#[derive(Debug, Clone)]
pub struct ValidationResult {
    /// All issues found, ordered by interval index.
    pub issues: Vec<ValidationIssue>,
}

impl ValidationResult {
    /// `true` when there are no validation issues of any severity.
    #[must_use]
    pub fn is_clean(&self) -> bool {
        self.issues.is_empty()
    }

    /// `true` when at least one issue has `Error` severity.
    #[must_use]
    pub fn has_errors(&self) -> bool {
        self.issues
            .iter()
            .any(|i| i.severity == ValidationSeverity::Error)
    }

    /// Number of findings that block billing.
    #[must_use]
    pub fn billing_block_count(&self) -> usize {
        self.issues.iter().filter(|i| i.blocks_billing()).count()
    }

    /// Filter by severity level.
    pub fn by_severity(
        &self,
        severity: ValidationSeverity,
    ) -> impl Iterator<Item = &ValidationIssue> {
        self.issues.iter().filter(move |i| i.severity == severity)
    }

    /// Filter by rule.
    pub fn by_rule(&self, rule: ValidationRuleId) -> impl Iterator<Item = &ValidationIssue> {
        self.issues.iter().filter(move |i| i.rule_id == rule)
    }
}

// ── Main validation function ──────────────────────────────────────────────────

/// Validate a slice of meter intervals against the configured rules.
///
/// **Order-independent** — see the [module docs](self#order-independence).
///
/// ## Example
///
/// ```rust
/// use metering::{MeterInterval, QualityFlag, validate_intervals, ValidationConfig};
/// use rust_decimal::dec;
/// use time::macros::datetime;
///
/// let intervals = vec![
///     MeterInterval {
///         from: datetime!(2026-06-01 0:00 UTC),
///         to:   datetime!(2026-06-01 0:15 UTC),
///         value: dec!(2.5),
///         quality: QualityFlag::Measured,
///         obis_code: None,
///     },
/// ];
/// let result = validate_intervals(&intervals, &ValidationConfig::default());
/// assert!(result.is_clean());
///
/// // Declaring the period the series should cover turns the same data into a
/// // finding: one quarter-hour of an intended hour is three intervals short.
/// let scoped = ValidationConfig::default()
///     .over_period(datetime!(2026-06-01 0:00 UTC), datetime!(2026-06-01 1:00 UTC));
/// assert!(validate_intervals(&intervals, &scoped).has_errors());
/// ```
#[must_use]
pub fn validate_intervals(
    intervals: &[MeterInterval],
    config: &ValidationConfig,
) -> ValidationResult {
    let mut issues: Vec<ValidationIssue> = Vec::new();

    if intervals.is_empty() {
        // An empty series still fails a declared period: nothing arrived at all.
        if let (Some((from, to)), Some(secs)) = (config.period, config.expected_interval_secs)
            && to > from
            && secs > 0
        {
            issues.push(gap_issue(from, to, secs, None));
        }
        return ValidationResult { issues };
    }

    // Evaluate the adjacency rules in timestamp order while still reporting the
    // caller's indices. Sorting a permutation rather than the data keeps
    // `interval_index` pointing at the interval the caller actually passed in.
    let mut order: Vec<usize> = (0..intervals.len()).collect();
    order.sort_by_key(|&i| (intervals[i].from, intervals[i].to));

    // V11 — the input was not already in order.
    if let Some((pos, idx)) = order
        .iter()
        .enumerate()
        .find(|&(pos, &i)| pos != i)
        .map(|(pos, &i)| (pos, i))
    {
        issues.push(
            ValidationIssue::new(
                ValidationRuleId::UnorderedSeries,
                ValidationSeverity::Warning,
                format!(
                    "series is not sorted ascending by `from`: position {pos} holds the \
                     interval starting {}, which belongs at index {idx} — the remaining \
                     rules were evaluated in timestamp order",
                    intervals[idx].from
                ),
            )
            .at(idx, &intervals[idx]),
        );
    }

    issues.extend(per_interval_rules(intervals, &order, config));
    issues.extend(outlier_rule(intervals, &order, config));
    issues.extend(gap_rules(intervals, &order, config));

    // V07 — a fall-back day that lost its repeated hour.
    let ordered: Vec<&MeterInterval> = order.iter().map(|&i| &intervals[i]).collect();
    issues.extend(detect_dst_ambiguity(&ordered));

    // Deterministic output: by interval index, then by rule, so two runs over
    // the same data produce byte-identical reports.
    issues.sort_by_key(|i| (i.interval_index.unwrap_or(usize::MAX), i.rule_id.code()));

    ValidationResult { issues }
}

// ── per-interval rules (V03, V05, V06, V08, V09, V12) ────────────────────────

fn per_interval_rules(
    intervals: &[MeterInterval],
    order: &[usize],
    config: &ValidationConfig,
) -> Vec<ValidationIssue> {
    let mut issues = Vec::new();
    let mut zero_run = 0usize;

    for (pos, &idx) in order.iter().enumerate() {
        let iv = &intervals[idx];

        // V03 — negative energy
        if config.negative_energy_is_error && iv.value < Decimal::ZERO {
            issues.push(
                ValidationIssue::new(
                    ValidationRuleId::NegativeEnergy,
                    ValidationSeverity::Error,
                    format!("negative energy {} kWh at {}", iv.value, iv.from),
                )
                .at(idx, iv),
            );
        }

        // V12 — average power above the plant's physical capacity.
        if let Some(cap_kw) = config.max_plant_power_kw
            && let Some(power_kw) = iv.demand_kw()
            && power_kw > cap_kw
        {
            issues.push(
                ValidationIssue::new(
                    ValidationRuleId::ImplausiblePower,
                    ValidationSeverity::Error,
                    format!(
                        "average power {power_kw} kW over {}–{} exceeds the plant capacity \
                         {cap_kw} kW",
                        iv.from, iv.to
                    ),
                )
                .at(idx, iv),
            );
        }

        // V05 — zero run. Emitted once, when the threshold is first reached.
        if iv.value.is_zero() {
            zero_run += 1;
            if zero_run == config.zero_run_threshold
                && config.zero_run_threshold > 0
                // The run cannot be longer than the positions walked so far, so
                // this holds — but it is a `usize` subtraction, and an
                // invariant three lines away is not a guard.
                && let Some(start) = (pos + 1).checked_sub(config.zero_run_threshold)
            {
                let start_idx = order[start];
                issues.push(
                    ValidationIssue::new(
                        ValidationRuleId::SuspiciousZeroRun,
                        ValidationSeverity::Warning,
                        format!(
                            "{} consecutive zero intervals from {}",
                            config.zero_run_threshold, intervals[start_idx].from
                        ),
                    )
                    .at(start_idx, &intervals[start_idx]),
                );
            }
        } else {
            zero_run = 0;
        }

        // V06 — interval length consistency
        if let Some(expected_secs) = config.expected_interval_secs {
            let actual_secs = (iv.to - iv.from).whole_seconds();
            if actual_secs != expected_length_secs(iv, expected_secs) {
                issues.push(
                    ValidationIssue::new(
                        ValidationRuleId::InconsistentIntervalLength,
                        ValidationSeverity::Warning,
                        format!(
                            "expected a {expected_secs} s interval, got {actual_secs} s at {}",
                            iv.from
                        ),
                    )
                    .at(idx, iv),
                );
            }
        }

        // V08 — future timestamp
        if let Some(now) = config.now
            && iv.from > now
        {
            issues.push(
                ValidationIssue::new(
                    ValidationRuleId::FutureTimestamp,
                    ValidationSeverity::Warning,
                    format!("interval starts in the future: {} > now {}", iv.from, now),
                )
                .at(idx, iv),
            );
        }

        // V09 — non-billable quality
        if !iv.quality.is_billable() {
            issues.push(
                ValidationIssue::new(
                    ValidationRuleId::NonBillableQuality,
                    ValidationSeverity::Error,
                    format!(
                        "quality {} is not billable at {} — an Ersatzwert is required",
                        iv.quality, iv.from
                    ),
                )
                .at(idx, iv),
            );
        }
    }

    issues
}

/// How long `iv` is allowed to be, given a configured expectation in seconds.
///
/// Ordinarily the answer is just `expected_secs`. The exception is a **daily**
/// series: `expected_interval_secs` is a fixed second count, and no fixed count
/// describes a German calendar day. A day is 82 800 s each spring and 90 000 s
/// each autumn, so a gas or water series read once a day would draw a V06
/// warning on both transition days every year — for being exactly right.
///
/// When the expectation is 86 400 s **and** the interval starts at a Berlin
/// local midnight, the real length of that calendar day is used instead. The
/// midnight condition matters: a fixed 24-hour window that happens to be
/// 86 400 s long is a different thing from a calendar day, and only the latter
/// gets the DST allowance.
fn expected_length_secs(iv: &MeterInterval, expected_secs: u32) -> i64 {
    const ONE_DAY: u32 = 86_400;
    if expected_secs != ONE_DAY {
        return i64::from(expected_secs);
    }
    let local = iv.from.to_timezone(timezones::db::europe::BERLIN);
    if local.time() != time::Time::MIDNIGHT {
        return i64::from(ONE_DAY);
    }
    crate::calendar::day_length(local.date()).whole_seconds()
}

// ── V04 — robust statistical outlier ─────────────────────────────────────────

/// Flag values that sit far from their local median, measured in MAD-derived
/// sigma.
///
/// This delegates to [`crate::quality::hampel_filter`] rather than reimplementing
/// the statistics, so validation and quality scoring cannot disagree about what
/// an outlier is.
///
/// The rule it replaced compared each value against the **mean of the whole
/// series** and flagged anything above `factor × mean`. Two things were wrong
/// with that. The mean includes the spike, so a single large value raises its
/// own threshold — with a factor of 10, one interval had to exceed roughly ten
/// times the average of a series it was itself inflating, which for a short
/// series is unreachable. And a global mean has no notion of the daily shape,
/// so on any profile with a real day/night swing the quiet hours are compared
/// against a threshold set by the busy ones.
fn outlier_rule(
    intervals: &[MeterInterval],
    order: &[usize],
    config: &ValidationConfig,
) -> Vec<ValidationIssue> {
    let Some(sigma) = config.outlier_sigma.filter(|s| s.is_finite() && *s > 0.0) else {
        return Vec::new();
    };
    let k = config.outlier_window;
    // A window needs more points than it has room for, or every point is its
    // own median and nothing can deviate.
    if k == 0 || order.len() <= k * 2 {
        return Vec::new();
    }

    let values: Vec<f64> = order
        .iter()
        .map(|&i| intervals[i].value.to_f64().unwrap_or(0.0))
        .collect();

    crate::quality::hampel_filter_with_floor(&values, k, sigma, config.outlier_min_sigma)
        .into_iter()
        .map(|pos| {
            let idx = order[pos];
            let iv = &intervals[idx];
            ValidationIssue::new(
                ValidationRuleId::StatisticalOutlier,
                ValidationSeverity::Warning,
                format!(
                    "{} kWh at {} deviates from its {}-interval neighbourhood by more than \
                     {sigma} robust sigma",
                    iv.value,
                    iv.from,
                    2 * k + 1
                ),
            )
            .at(idx, iv)
        })
        .collect()
}

// ── V01 / V02 — gaps and overlaps ────────────────────────────────────────────

fn gap_issue(
    from: OffsetDateTime,
    to: OffsetDateTime,
    expected_secs: u32,
    index: Option<usize>,
) -> ValidationIssue {
    let gap_secs = (to - from).whole_seconds();
    let count = gap_secs / i64::from(expected_secs);
    let mut issue = ValidationIssue::new(
        ValidationRuleId::GapDetected,
        ValidationSeverity::Error,
        format!("gap of {count} interval(s) between {from} and {to} — Ersatzwerte required"),
    )
    .anchored_at(from);
    issue.interval_index = index;
    issue
}

fn gap_rules(
    intervals: &[MeterInterval],
    order: &[usize],
    config: &ValidationConfig,
) -> Vec<ValidationIssue> {
    let mut issues = Vec::new();

    // V02 — overlap. Compared against the furthest end seen so far, not just
    // the immediately preceding interval: sorted by `from`, a long interval can
    // swallow several short ones, and only the first of them touches its
    // predecessor. The previous pairwise check reported that first collision
    // and silently passed the rest.
    let mut max_end: Option<(OffsetDateTime, usize)> = None;
    for &idx in order {
        let iv = &intervals[idx];
        if let Some((end, prev_idx)) = max_end
            && iv.from < end
        {
            let prev = &intervals[prev_idx];
            issues.push(
                ValidationIssue::new(
                    ValidationRuleId::OverlapDetected,
                    ValidationSeverity::Error,
                    format!(
                        "interval [{}, {}) overlaps [{}, {})",
                        iv.from, iv.to, prev.from, prev.to
                    ),
                )
                .at(idx, iv),
            );
        }
        if max_end.is_none_or(|(end, _)| iv.to > end) {
            max_end = Some((iv.to, idx));
        }
    }

    // V01 — gaps. Needs the grid spacing to say how many intervals are missing.
    let Some(expected_secs) = config.expected_interval_secs.filter(|s| *s > 0) else {
        return issues;
    };
    let step = i64::from(expected_secs);

    // Interior gaps.
    for window in order.windows(2) {
        let (a, b) = (&intervals[window[0]], &intervals[window[1]]);
        if (b.from - a.to).whole_seconds() >= step {
            issues.push(gap_issue(a.to, b.from, expected_secs, Some(window[1])));
        }
    }

    // Head and tail, against the declared period. Without a period the series
    // defines its own extent and a truncated delivery is invisible.
    if let Some((period_from, period_to)) = config.period {
        let first = &intervals[order[0]];
        let last = &intervals[*order.last().expect("non-empty")];
        if (first.from - period_from).whole_seconds() >= step {
            issues.push(gap_issue(
                period_from,
                first.from,
                expected_secs,
                Some(order[0]),
            ));
        }
        if (period_to - last.to).whole_seconds() >= step {
            issues.push(gap_issue(last.to, period_to, expected_secs, None));
        }
    }

    issues
}

// ── V07 — collapsed DST fall-back hour ───────────────────────────────────────

/// Detect a collapsed DST fall-back hour (V07).
///
/// Germany repeats local 02:00–03:00 when CEST ends, so the fall-back day has
/// **25 hours**. A series converted from local time without carrying the UTC
/// offset collapses the two passes into one and silently loses an hour of
/// energy.
///
/// ## The test is the repeated hour, not the day
///
/// An earlier version compared the whole day's covered duration against 25
/// hours. That cannot tell a collapsed hour from an ordinary gap: *any* two
/// missing quarter-hours anywhere on a fall-back day produced a confident
/// report that "the repeated hour 02:00–03:00 was collapsed", which was simply
/// untrue and sent the reader looking in the wrong place.
///
/// The two passes occupy `[transition − 1 h, transition + 1 h)` in UTC — one at
/// UTC+2, one at UTC+1. This looks only there. A gap at midday is a V01 gap and
/// nothing else; a genuinely collapsed hour shows up here even on a day that is
/// otherwise complete.
///
/// The rule only judges a series that demonstrably **spans** that window, so a
/// truncated query window is short rather than corrupt.
fn detect_dst_ambiguity(intervals: &[&MeterInterval]) -> Vec<ValidationIssue> {
    let (Some(first), Some(last)) = (intervals.first(), intervals.last()) else {
        return Vec::new();
    };

    // The repeated hour belongs to the local day the series starts on.
    let local_day = crate::calendar::local_day(first.from);
    if crate::calendar::day_kind(local_day) != crate::calendar::DayKind::LongDay {
        return Vec::new();
    }
    let Some(transition) = crate::calendar::dst_transition_utc(local_day) else {
        return Vec::new();
    };

    let window_start = transition - Duration::hours(1);
    let window_end = transition + Duration::hours(1);

    // Only judge a series that covers the window at both ends; anything else is
    // a truncated read, not a collapsed hour.
    if first.from > window_start || last.to < window_end {
        return Vec::new();
    }

    // How much of the two-hour UTC window the series actually covers. A correct
    // series covers all of it; a collapsed one covers about half.
    let covered: i64 = intervals
        .iter()
        .map(|iv| {
            let from = iv.from.max(window_start);
            let to = iv.to.min(window_end);
            (to - from).whole_seconds().max(0)
        })
        .sum();

    const TWO_HOURS: i64 = 2 * 3600;
    if covered >= TWO_HOURS {
        return Vec::new();
    }

    vec![
        ValidationIssue::new(
            ValidationRuleId::DstAmbiguity,
            ValidationSeverity::Error,
            format!(
                "local day {local_day} repeats 02:00–03:00, so {window_start} … {window_end} \
                 holds two passes of it — but the series covers only {covered} s of that \
                 window. The repeated hour was collapsed, so an hour of energy is missing \
                 and the surviving intervals are ambiguous between the two passes."
            ),
        )
        .anchored_at(window_start),
    ]
}

#[cfg(test)]
mod v07_tests {
    use super::*;
    use crate::interval::QualityFlag;
    use rust_decimal::dec;
    use time::Duration;
    use time::macros::{date, datetime};

    /// `n` consecutive quarter-hours from `start`.
    fn qh(start: OffsetDateTime, n: i64) -> Vec<MeterInterval> {
        (0..n)
            .map(|i| {
                let from = start + Duration::minutes(15 * i);
                MeterInterval {
                    from,
                    to: from + Duration::minutes(15),
                    value: dec!(1.0),
                    quality: QualityFlag::Measured,
                    obis_code: None,
                }
            })
            .collect()
    }

    fn detect(intervals: &[MeterInterval]) -> Vec<ValidationIssue> {
        detect_dst_ambiguity(&intervals.iter().collect::<Vec<_>>())
    }

    /// 2026-10-25 local runs 22:00Z (24 Oct) → 23:00Z (25 Oct): 25 hours,
    /// 100 quarter-hours. A complete day is not ambiguous.
    #[test]
    fn a_complete_25_hour_fall_back_day_is_clean() {
        assert!(detect(&qh(datetime!(2026-10-24 22:00 UTC), 100)).is_empty());
    }

    /// The same local day with the repeated hour missing: the four quarter-hours
    /// of the second pass are gone, so the window holds one hour, not two.
    #[test]
    fn a_collapsed_repeated_hour_raises_v07() {
        let mut day = qh(datetime!(2026-10-24 22:00 UTC), 100);
        // The window is 00:00–02:00 UTC; drop its second half.
        day.retain(|iv| {
            !(iv.from >= datetime!(2026-10-25 1:00 UTC) && iv.from < datetime!(2026-10-25 2:00 UTC))
        });
        let issues = detect(&day);
        assert_eq!(issues.len(), 1, "expected V07: {issues:?}");
        assert_eq!(issues[0].rule_id, ValidationRuleId::DstAmbiguity);
        assert!(
            issues[0].message.contains("3600 s"),
            "{}",
            issues[0].message
        );
    }

    /// The false positive this rule used to produce. A gap at **midday** on a
    /// fall-back day is a V01 gap and nothing more — the repeated hour is
    /// intact, and saying otherwise sends the reader to the wrong place.
    #[test]
    fn an_ordinary_gap_elsewhere_on_the_day_is_not_a_collapsed_hour() {
        let mut day = qh(datetime!(2026-10-24 22:00 UTC), 100);
        // Drop two quarter-hours around local midday, far from the transition.
        day.retain(|iv| {
            !(iv.from >= datetime!(2026-10-25 11:00 UTC)
                && iv.from < datetime!(2026-10-25 11:30 UTC))
        });
        assert!(
            detect(&day).is_empty(),
            "a midday gap must not be reported as a collapsed DST hour"
        );

        // ...and the gap is still caught, by the rule that owns it.
        let report = validate_intervals(&day, &ValidationConfig::default());
        assert_eq!(report.by_rule(ValidationRuleId::GapDetected).count(), 1);
        assert_eq!(report.by_rule(ValidationRuleId::DstAmbiguity).count(), 0);
    }

    /// A window that merely starts inside the repeated hour is short, not
    /// corrupt.
    #[test]
    fn a_truncated_window_across_the_boundary_is_not_flagged() {
        assert!(detect(&qh(datetime!(2026-10-25 0:45 UTC), 4)).is_empty());
    }

    /// A series ending before the window closes cannot be judged either.
    #[test]
    fn a_series_that_stops_inside_the_window_is_not_flagged() {
        // 22:00Z to 01:30Z — covers the first pass and half the second.
        assert!(detect(&qh(datetime!(2026-10-24 22:00 UTC), 14)).is_empty());
    }

    #[test]
    fn an_ordinary_day_raises_nothing() {
        assert!(detect(&qh(datetime!(2026-07-14 22:00 UTC), 96)).is_empty());
    }

    /// Spring forward skips an hour rather than repeating one; a 23-hour day is
    /// correct there, so V07 must stay silent.
    #[test]
    fn spring_forward_raises_nothing() {
        assert!(detect(&qh(datetime!(2026-03-28 23:00 UTC), 92)).is_empty());
    }

    /// V07 must be reachable through the public entry point.
    #[test]
    fn v07_is_emitted_by_validate_intervals() {
        let mut day = qh(datetime!(2026-10-24 22:00 UTC), 100);
        day.retain(|iv| {
            !(iv.from >= datetime!(2026-10-25 1:00 UTC) && iv.from < datetime!(2026-10-25 2:00 UTC))
        });
        let report = validate_intervals(&day, &ValidationConfig::default());
        assert_eq!(report.by_rule(ValidationRuleId::DstAmbiguity).count(), 1);
    }

    /// The rule keys off the calendar, not a hard-coded date.
    #[test]
    fn the_rule_follows_the_tz_database() {
        assert_eq!(
            crate::calendar::day_kind(date!(2026 - 10 - 25)),
            crate::calendar::DayKind::LongDay
        );
        assert_eq!(
            crate::calendar::day_kind(date!(2027 - 10 - 31)),
            crate::calendar::DayKind::LongDay
        );
        // 2027's fall-back day, collapsed.
        let mut day = qh(datetime!(2027-10-30 22:00 UTC), 100);
        day.retain(|iv| {
            !(iv.from >= datetime!(2027-10-31 1:00 UTC) && iv.from < datetime!(2027-10-31 2:00 UTC))
        });
        assert_eq!(detect(&day).len(), 1);
    }
}
