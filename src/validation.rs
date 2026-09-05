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
//! **V10 is deliberately unused.** A rollover is a property of a *register*,
//! and a [`MeterInterval`] carries the energy in one interval rather than a
//! cumulative Zählerstand — so it is detected in [`crate::reading`], where
//! readings live. The number stays unused rather than being recycled, so a
//! stored `V10` cannot be reinterpreted as something else.
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

use crate::calendar::DayBoundary;
use crate::interval::MeterInterval;

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

impl ValidationSeverity {
    /// Every severity, least to most serious.
    pub const ALL: [Self; 3] = [Self::Info, Self::Warning, Self::Error];

    /// Stable DB/wire label. Matches the `serde` tag and `FromStr` input.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Info => "INFO",
            Self::Warning => "WARNING",
            Self::Error => "ERROR",
        }
    }
}

// ── ValidationRuleId ─────────────────────────────────────────────────────────

/// Identifies which validation rule triggered an issue.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum ValidationRuleId {
    /// V01 — an expected interval is missing.
    #[cfg_attr(feature = "serde", serde(rename = "V01"))]
    GapDetected,
    /// V02 — two intervals cover the same instant.
    #[cfg_attr(feature = "serde", serde(rename = "V02"))]
    OverlapDetected,
    /// V03 — consumption value is negative (impossible for Bezug-only meters).
    #[cfg_attr(feature = "serde", serde(rename = "V03"))]
    NegativeEnergy,
    /// V04 — the value is a statistical outlier against its own neighbourhood.
    #[cfg_attr(feature = "serde", serde(rename = "V04"))]
    StatisticalOutlier,
    /// V05 — consecutive zero values suggest a stuck / frozen meter.
    ///
    /// One finding per run, anchored at its first interval and carrying the
    /// length the run actually reached — not the threshold that armed it.
    #[cfg_attr(feature = "serde", serde(rename = "V05"))]
    SuspiciousZeroRun,
    /// V06 — interval length differs from the expected granularity.
    #[cfg_attr(feature = "serde", serde(rename = "V06"))]
    InconsistentIntervalLength,
    /// V07 — the DST fall-back hour was collapsed (local time leaked in).
    #[cfg_attr(feature = "serde", serde(rename = "V07"))]
    DstAmbiguity,
    /// V08 — interval starts after the reference instant.
    #[cfg_attr(feature = "serde", serde(rename = "V08"))]
    FutureTimestamp,
    /// V09 — quality flag is non-billable (`Faulty` or `Unknown`).
    #[cfg_attr(feature = "serde", serde(rename = "V09"))]
    NonBillableQuality,
    /// V11 — the series was not sorted ascending by `from`.
    ///
    /// Reported once per call. The remaining rules are evaluated in timestamp
    /// order regardless, so their findings stay correct; this says the *input*
    /// was out of order, which is itself a defect worth surfacing — an MSCONS
    /// series arriving shuffled usually means a broken merge upstream.
    #[cfg_attr(feature = "serde", serde(rename = "V11"))]
    UnorderedSeries,
    /// V12 — average power over the interval exceeds the plant's capacity.
    ///
    /// Unlike [`StatisticalOutlier`](Self::StatisticalOutlier), which compares a
    /// value against its neighbours, this compares it against a physical
    /// ceiling the metered plant cannot exceed. A value above it is not
    /// unusual, it is impossible — hence `Error` rather than `Warning`.
    #[cfg_attr(feature = "serde", serde(rename = "V12"))]
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

    /// The [`ValidationConfig`] field that switches this rule on, or `None` for
    /// one of the four that cannot be switched off.
    ///
    /// Lets a caller name the missing setting without knowing the rule table:
    /// *"V12 is off; set `ValidationConfig::max_plant_power_kw`"*.
    ///
    /// A field switches its rule off either by being `None`
    /// (`expected_interval_secs`, `outlier_sigma`, `now`,
    /// `max_plant_power_kw`) or by carrying a value that means "do not check"
    /// (`negative_energy_is_error = false`, `zero_run_threshold = 0`). Either
    /// way [`ValidationConfig::enabled_rules`] reports the outcome, so the
    /// distinction never has to be reasoned about at a call site.
    ///
    /// ```rust
    /// use metering::{ValidationConfig, ValidationRuleId};
    ///
    /// let cfg = ValidationConfig::default();
    /// for rule in cfg.disabled_rules() {
    ///     let field = rule.enabling_field().unwrap_or("(always on)");
    ///     println!("{rule} is off — set ValidationConfig::{field}");
    /// }
    /// assert_eq!(
    ///     ValidationRuleId::ImplausiblePower.enabling_field(),
    ///     Some("max_plant_power_kw"),
    /// );
    /// ```
    #[must_use]
    pub const fn enabling_field(self) -> Option<&'static str> {
        match self {
            Self::GapDetected | Self::InconsistentIntervalLength => Some("expected_interval_secs"),
            Self::StatisticalOutlier => Some("outlier_sigma"),
            Self::SuspiciousZeroRun => Some("zero_run_threshold"),
            Self::FutureTimestamp => Some("now"),
            Self::ImplausiblePower => Some("max_plant_power_kw"),
            Self::NegativeEnergy => Some("negative_energy_is_error"),
            Self::OverlapDetected
            | Self::DstAmbiguity
            | Self::NonBillableQuality
            | Self::UnorderedSeries => None,
        }
    }

    /// This rule's bit position in a [`RuleSet`] — its index in
    /// [`ALL`](Self::ALL).
    const fn bit(self) -> u16 {
        1 << (self as u16)
    }

    /// The `Vnn` code — the one spelling, shared by `Display`, [`FromStr`] and
    /// the `serde` tag.
    ///
    /// [`FromStr`]: std::str::FromStr
    #[must_use]
    pub const fn as_str(self) -> &'static str {
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

crate::codes::string_codes! {
    ValidationRuleId;
    ValidationSeverity;
}

// ── RuleSet ──────────────────────────────────────────────────────────────────

/// A set of [`ValidationRuleId`]s — which rules a configuration arms, and which
/// a run actually evaluated.
///
/// Four of the eleven rules are **opt-in**: they need a number this library
/// refuses to invent, and leaving the corresponding [`ValidationConfig`] field
/// `None` turns the rule off. A clean [`ValidationResult`] therefore means *"the
/// rules that ran found nothing"*, which is weaker than "nothing is wrong" —
/// so [`ValidationConfig::disabled_rules`] answers before a run,
/// [`ValidationResult::evaluated`] after one, and the two differ exactly when
/// the **data** rather than the config stopped a rule.
///
/// A bitset: no duplicates, `contains` is a mask, no allocation. It serialises
/// as a list of codes, never as an integer.
///
/// ```rust
/// use metering::{RuleSet, ValidationConfig, ValidationRuleId as R};
///
/// // Nothing is configured beyond the defaults, so three rules are inert.
/// let cfg = ValidationConfig::default();
/// assert!(cfg.disabled_rules().contains(R::ImplausiblePower));
/// assert!(cfg.disabled_rules().contains(R::FutureTimestamp));
/// assert!(cfg.enabled_rules().contains(R::GapDetected));
///
/// // Supplying the ceiling turns V12 on.
/// let armed = cfg.with_plant_capacity_kw(rust_decimal::dec!(30));
/// assert!(armed.enabled_rules().contains(R::ImplausiblePower));
/// assert!(!armed.disabled_rules().contains(R::ImplausiblePower));
/// # assert_eq!(RuleSet::ALL.len(), 11);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct RuleSet(u16);

impl RuleSet {
    /// The empty set.
    pub const EMPTY: Self = Self(0);

    /// Every rule.
    pub const ALL: Self = {
        let mut bits = 0u16;
        let mut i = 0;
        while i < ValidationRuleId::ALL.len() {
            bits |= ValidationRuleId::ALL[i].bit();
            i += 1;
        }
        Self(bits)
    };

    /// `true` when `rule` is in the set.
    #[must_use]
    pub const fn contains(self, rule: ValidationRuleId) -> bool {
        self.0 & rule.bit() != 0
    }

    /// This set plus `rule`.
    #[must_use]
    pub const fn with(self, rule: ValidationRuleId) -> Self {
        Self(self.0 | rule.bit())
    }

    /// This set minus `rule`.
    #[must_use]
    pub const fn without(self, rule: ValidationRuleId) -> Self {
        Self(self.0 & !rule.bit())
    }

    /// Every rule **not** in this set.
    #[must_use]
    pub const fn complement(self) -> Self {
        Self(Self::ALL.0 & !self.0)
    }

    /// The rules in both sets.
    #[must_use]
    pub const fn intersection(self, other: Self) -> Self {
        Self(self.0 & other.0)
    }

    /// The rules in either set.
    #[must_use]
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    /// How many rules are in the set.
    #[must_use]
    pub const fn len(self) -> usize {
        self.0.count_ones() as usize
    }

    /// `true` when the set is empty.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// The rules in the set, in [`ValidationRuleId::ALL`] order.
    pub fn iter(self) -> impl Iterator<Item = ValidationRuleId> {
        ValidationRuleId::ALL
            .into_iter()
            .filter(move |r| self.contains(*r))
    }
}

impl FromIterator<ValidationRuleId> for RuleSet {
    fn from_iter<I: IntoIterator<Item = ValidationRuleId>>(iter: I) -> Self {
        iter.into_iter().fold(Self::EMPTY, Self::with)
    }
}

impl IntoIterator for RuleSet {
    type Item = ValidationRuleId;
    type IntoIter = std::vec::IntoIter<ValidationRuleId>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter().collect::<Vec<_>>().into_iter()
    }
}

impl std::fmt::Display for RuleSet {
    /// The codes, comma-separated — `"V01, V02, V03"` — or `"none"`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.is_empty() {
            return f.write_str("none");
        }
        let mut first = true;
        for rule in self.iter() {
            if !first {
                f.write_str(", ")?;
            }
            f.write_str(rule.as_str())?;
            first = false;
        }
        Ok(())
    }
}

#[cfg(feature = "serde")]
mod rule_set_serde {
    use super::{RuleSet, ValidationRuleId};
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    impl Serialize for RuleSet {
        /// A list of codes — `["V01","V02"]` — never the integer, which would
        /// silently change meaning if a variant were ever reordered.
        fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
            serializer.collect_seq(self.iter().map(ValidationRuleId::as_str))
        }
    }

    impl<'de> Deserialize<'de> for RuleSet {
        fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
            Ok(Vec::<ValidationRuleId>::deserialize(deserializer)?
                .into_iter()
                .collect())
        }
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
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::rfc3339_option"))]
    pub affected_from: Option<OffsetDateTime>,
    /// The measured value at the affected interval, when there is one.
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::decimal_option"))]
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
    /// Default: `4` — one hour at quarter-hour granularity. **`0` disables the
    /// rule**, and [`enabled_rules`](Self::enabled_rules) reports it as off.
    ///
    /// The finding carries the run's real length, not this threshold.
    pub zero_run_threshold: usize,

    /// Treat negative energy as an Error (V03).
    ///
    /// The default is the market's own rule. EDI@Energy *Codeliste der
    /// OBIS-Kennzahlen und Medien* v2.5c, §2.1: *"Die Energieflussrichtung wird
    /// mittels der OBIS-Kennzahl definiert. Mit Ausnahme der Übermittlung von
    /// Korrekturenergiemengen (hier können die Werte auch negativ sein), sind
    /// die Mengenangaben nur mit positiven Werten oder 0 anzugeben."* Direction
    /// lives in value group C — `1-b:1.x.y` against `1-b:2.x.y` — not in the
    /// sign, so a negative quantity on a MaKo channel is a defect.
    ///
    /// Set to `false` for the two cases where a signed series is correct: a
    /// Korrekturenergiemenge, and a derived series such as the residual load
    /// [`crate::virtual_meter`] computes. Default: `true`.
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

    /// Where a **daily** series is cut, for the V06 length check.
    ///
    /// Only consulted when [`expected_interval_secs`](Self::expected_interval_secs)
    /// is `86_400`. A German day is 82 800 s each spring and 90 000 s each
    /// autumn, so a daily series is judged against the calendar rather than
    /// against the flat second count — and the gas market's day starts at
    /// 06:00, so which day an interval belongs to depends on the boundary.
    ///
    /// [`DayBoundary::Midnight`] by default; a daily gas series wants
    /// [`DayBoundary::Gastag`], or it draws a length warning on both transition
    /// days every year for being exactly right.
    pub day_boundary: DayBoundary,
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
            day_boundary: DayBoundary::Midnight,
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

    /// Configuration for a series where a negative value is legitimate — a
    /// Korrekturenergiemenge, or a derived signed series. See
    /// [`negative_energy_is_error`](ValidationConfig::negative_energy_is_error).
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

    /// Set the physical capacity ceiling (kW) for V12, arming the rule.
    ///
    /// The same physical fact as
    /// [`LastgangConfig::with_capacity_kw`](crate::reading::LastgangConfig::with_capacity_kw),
    /// expressed at the other end of the pipeline: that one *prevents* a bad
    /// value from being differenced into existence, this one *flags* one that
    /// arrived already formed.
    #[must_use]
    pub fn with_plant_capacity_kw(mut self, kw: Decimal) -> Self {
        self.max_plant_power_kw = Some(kw);
        self
    }

    /// Set the reference instant for V08, arming the rule.
    ///
    /// A parameter rather than a clock read — see the crate-level
    /// **Determinism** section. Callers holding a clock pass
    /// `OffsetDateTime::now_utc()`.
    #[must_use]
    pub fn at_reference_instant(mut self, now: OffsetDateTime) -> Self {
        self.now = Some(now);
        self
    }

    /// Declare the period the series must cover, extending V01 to the head and
    /// tail of the series.
    #[must_use]
    pub fn over_period(mut self, from: OffsetDateTime, to: OffsetDateTime) -> Self {
        self.period = Some((from, to));
        self
    }

    /// Cut daily intervals on `boundary` for the V06 length check
    /// (builder style).
    ///
    /// ```rust
    /// use metering::{ValidationConfig, calendar::DayBoundary};
    ///
    /// // A daily gas series is a whole number of Gastage, 23–25 h each.
    /// let cfg = ValidationConfig { expected_interval_secs: Some(86_400), ..Default::default() }
    ///     .on(DayBoundary::Gastag);
    /// assert_eq!(cfg.day_boundary, DayBoundary::Gastag);
    /// ```
    #[must_use]
    pub const fn on(mut self, boundary: DayBoundary) -> Self {
        self.day_boundary = boundary;
        self
    }

    /// The rules this configuration permits to fire.
    ///
    /// Config-level only. Whether a rule then *ran* on a particular series is
    /// [`ValidationResult::evaluated`], which can be smaller.
    #[must_use]
    pub fn enabled_rules(&self) -> RuleSet {
        use ValidationRuleId as R;
        let mut set = RuleSet::EMPTY
            .with(R::OverlapDetected)
            .with(R::DstAmbiguity)
            .with(R::NonBillableQuality)
            .with(R::UnorderedSeries);
        if self.negative_energy_is_error {
            set = set.with(R::NegativeEnergy);
        }
        // A threshold of zero is not "report every zero"; it is off, and
        // `enabled_rules` has to say so or a caller reads a clean report as
        // "no stuck meter" when nothing looked.
        if self.zero_run_threshold > 0 {
            set = set.with(R::SuspiciousZeroRun);
        }
        if self.expected_interval_secs.is_some_and(|s| s > 0) {
            set = set.with(R::GapDetected).with(R::InconsistentIntervalLength);
        }
        if self.outlier_sigma.is_some_and(|t| t.is_finite() && t > 0.0) && self.outlier_window > 0 {
            set = set.with(R::StatisticalOutlier);
        }
        if self.now.is_some() {
            set = set.with(R::FutureTimestamp);
        }
        if self.max_plant_power_kw.is_some() {
            set = set.with(R::ImplausiblePower);
        }
        set
    }

    /// The rules this configuration leaves **inert** — the complement of
    /// [`enabled_rules`](Self::enabled_rules).
    ///
    /// Log it at startup or assert on it in a test; each rule's
    /// [`enabling_field`](ValidationRuleId::enabling_field) names the setting
    /// that would switch it on.
    ///
    /// ```rust
    /// use metering::{QualityConfig, Sparte, ValidationRuleId};
    ///
    /// // A "now" and a nameplate capacity are not properties of a commodity,
    /// // so the per-commodity defaults leave V08 and V12 off.
    /// let cfg = QualityConfig::for_sparte(Sparte::Strom);
    /// let off = cfg.validation.disabled_rules();
    /// assert!(off.contains(ValidationRuleId::ImplausiblePower));
    /// assert_eq!(off.to_string(), "V08, V12");
    /// ```
    #[must_use]
    pub fn disabled_rules(&self) -> RuleSet {
        self.enabled_rules().complement()
    }
}

// ── Validation result ─────────────────────────────────────────────────────────

/// Result of validating a slice of meter intervals.
#[derive(Debug, Clone)]
pub struct ValidationResult {
    /// All issues found, ordered by interval index.
    pub issues: Vec<ValidationIssue>,

    /// The rules that were actually evaluated on this call.
    ///
    /// **A clean result means "these rules found nothing", not "nothing is
    /// wrong".** A rule missing from [`ValidationConfig::enabled_rules`] was
    /// switched off by the config; one missing only here was stopped by the
    /// data — V04 needs more points than its window is wide.
    /// [`skipped`](Self::skipped) is the complement.
    pub evaluated: RuleSet,
}

impl ValidationResult {
    /// The rules that did **not** run, for whatever reason.
    #[must_use]
    pub fn skipped(&self) -> RuleSet {
        self.evaluated.complement()
    }

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
    let enabled = config.enabled_rules();

    if intervals.is_empty() {
        // An empty series still fails a declared period: nothing arrived at all.
        if let (Some((from, to)), Some(secs)) = (config.period, config.expected_interval_secs)
            && to > from
            && secs > 0
        {
            issues.push(gap_issue(from, to, secs, None));
        }
        // Nothing else can have run: every other rule needs an interval to
        // look at. Reporting `enabled` here would claim ten clean rules over
        // an empty series.
        return ValidationResult {
            issues,
            evaluated: enabled.intersection(RuleSet::EMPTY.with(ValidationRuleId::GapDetected)),
        };
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
    issues.extend(zero_run_rule(intervals, &order, config));
    issues.extend(outlier_rule(intervals, &order, config));
    issues.extend(gap_rules(intervals, &order, config));

    // V07 — a fall-back day that lost its repeated hour.
    let ordered: Vec<&MeterInterval> = order.iter().map(|&i| &intervals[i]).collect();
    issues.extend(detect_dst_ambiguity(&ordered));

    // Deterministic output: by interval index, then by rule, so two runs over
    // the same data produce byte-identical reports.
    issues.sort_by_key(|i| (i.interval_index.unwrap_or(usize::MAX), i.rule_id.as_str()));

    // What actually ran: everything the config armed, less the one rule whose
    // window the data was too short for.
    let mut evaluated = enabled;
    if !outlier_window_fits(intervals.len(), config) {
        evaluated = evaluated.without(ValidationRuleId::StatisticalOutlier);
    }

    ValidationResult { issues, evaluated }
}

// ── per-interval rules (V03, V05, V06, V08, V09, V12) ────────────────────────

fn per_interval_rules(
    intervals: &[MeterInterval],
    order: &[usize],
    config: &ValidationConfig,
) -> Vec<ValidationIssue> {
    let mut issues = Vec::new();

    for &idx in order {
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

        // V06 — interval length consistency
        if let Some(expected_secs) = config.expected_interval_secs {
            let actual_secs = (iv.to - iv.from).whole_seconds();
            if actual_secs != expected_length_secs(iv, expected_secs, config.day_boundary) {
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
/// When the expectation is 86 400 s **and** the interval starts exactly on a
/// day boundary, the real length of that day is used instead. The boundary
/// condition matters: a fixed 24-hour window that happens to be 86 400 s long
/// is a different thing from a calendar day, and only the latter gets the DST
/// allowance.
///
/// Which boundary is [`ValidationConfig::day_boundary`], so a daily **gas**
/// series on the 06:00 Gastag gets the same allowance as an electricity series
/// on the Liefertag. The two transition days are not even the same date for
/// the two boundaries: the clocks move at 02:00/03:00, inside the Gastag that
/// began the previous morning.
fn expected_length_secs(iv: &MeterInterval, expected_secs: u32, boundary: DayBoundary) -> i64 {
    const ONE_DAY: u32 = 86_400;
    if expected_secs != ONE_DAY {
        return i64::from(expected_secs);
    }
    let day = boundary.local_day(iv.from);
    if boundary.day_start_utc(day) != iv.from {
        return i64::from(ONE_DAY);
    }
    boundary.day_length(day).whole_seconds()
}

// ── V05 — stuck meter ────────────────────────────────────────────────────────

/// Report each run of consecutive zero intervals that reaches the threshold,
/// with the length the run **actually** reached.
///
/// Emitted when the run closes, not when it crosses the threshold: a meter
/// stuck for three weeks says three weeks, not the four intervals that armed
/// the rule.
///
/// One finding per run, anchored at its first interval, so a series with two
/// separate outages reports two.
///
/// **A gap ends a run.** Zeros either side of a hole are not consecutive
/// readings of zero — they are two runs with an unknown stretch between them,
/// and joining them would report a stuck meter over a period nobody measured.
/// Adjacency is `next.from == previous.to`, the same test V01 uses for a gap,
/// so the two rules cannot disagree about whether a series is continuous.
fn zero_run_rule(
    intervals: &[MeterInterval],
    order: &[usize],
    config: &ValidationConfig,
) -> Vec<ValidationIssue> {
    if config.zero_run_threshold == 0 {
        return Vec::new();
    }
    let mut issues = Vec::new();
    let mut run: Option<(usize, usize)> = None; // (first index, length)

    let close = |run: Option<(usize, usize)>, issues: &mut Vec<ValidationIssue>| {
        let Some((start_idx, len)) = run else { return };
        if len < config.zero_run_threshold {
            return;
        }
        let first = &intervals[start_idx];
        issues.push(
            ValidationIssue::new(
                ValidationRuleId::SuspiciousZeroRun,
                ValidationSeverity::Warning,
                format!(
                    "{len} consecutive zero intervals from {} — at or above the \
                     configured threshold of {}",
                    first.from, config.zero_run_threshold
                ),
            )
            .at(start_idx, first),
        );
    };

    let mut previous_end: Option<OffsetDateTime> = None;
    for &idx in order {
        let iv = &intervals[idx];
        let contiguous = previous_end.is_none_or(|end| iv.from == end);
        if !contiguous {
            close(run.take(), &mut issues);
        }
        if iv.value.is_zero() {
            run = Some(match run {
                Some((start_idx, len)) => (start_idx, len + 1),
                None => (idx, 1),
            });
        } else {
            close(run.take(), &mut issues);
        }
        previous_end = Some(iv.to);
    }
    close(run, &mut issues);
    issues
}

// ── V04 — robust statistical outlier ─────────────────────────────────────────

/// Flag values that sit far from their local median, measured in MAD-derived
/// sigma.
///
/// This delegates to [`crate::quality::hampel_filter`] rather than reimplementing
/// the statistics, so validation and quality scoring cannot disagree about what
/// an outlier is.
///
/// A mean-based test cannot do this job. The mean includes the spike, so a
/// large value raises its own threshold; and a global mean has no notion of the
/// daily shape, so the quiet hours are judged against a threshold set by the
/// busy ones.
/// Whether a series of `len` intervals is long enough for the V04 window.
///
/// A window needs more points than it has room for, or every point is its own
/// median and nothing can deviate. This is the one rule the *data* can switch
/// off rather than the configuration, which is why
/// [`ValidationResult::evaluated`] can be smaller than
/// [`ValidationConfig::enabled_rules`].
fn outlier_window_fits(len: usize, config: &ValidationConfig) -> bool {
    config.outlier_window > 0 && len > config.outlier_window * 2
}

fn outlier_rule(
    intervals: &[MeterInterval],
    order: &[usize],
    config: &ValidationConfig,
) -> Vec<ValidationIssue> {
    let Some(sigma) = config.outlier_sigma.filter(|s| s.is_finite() && *s > 0.0) else {
        return Vec::new();
    };
    let k = config.outlier_window;
    if !outlier_window_fits(order.len(), config) {
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
    // A hole shorter than one interval is not "n intervals missing" — it is a
    // series that does not sit on the grid at all, which is a different defect
    // and needs saying differently. It is still an Error: the energy in that
    // hole is missing either way.
    let message = if count >= 1 {
        format!("gap of {count} interval(s) between {from} and {to} — Ersatzwerte required")
    } else {
        format!(
            "{gap_secs} s uncovered between {from} and {to} — shorter than the \
             {expected_secs} s grid, so the series is off-grid rather than merely short"
        )
    };
    let mut issue = ValidationIssue::new(
        ValidationRuleId::GapDetected,
        ValidationSeverity::Error,
        message,
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

    // Interior gaps. **Any** uncovered second counts, not only a whole missing
    // interval: an off-grid series — 00:00–00:15 then 00:20–00:35 — leaves five
    // minutes uncovered that V06 cannot see, every interval being exactly 900 s.
    //
    // Measured against the furthest end seen so far, as V02 is: sorted by
    // `from`, one long interval can swallow several short ones, and a pairwise
    // comparison would report the space behind a swallowed interval as missing
    // while the long one covers it.
    let mut covered_to: Option<OffsetDateTime> = None;
    for &idx in order {
        let iv = &intervals[idx];
        if let Some(end) = covered_to
            && iv.from > end
        {
            issues.push(gap_issue(end, iv.from, expected_secs, Some(idx)));
        }
        covered_to = Some(covered_to.map_or(iv.to, |end| end.max(iv.to)));
    }

    // Head and tail, against the declared period. Without a period the series
    // defines its own extent and a truncated delivery is invisible.
    if let Some((period_from, period_to)) = config.period {
        let first = &intervals[order[0]];
        // The furthest end, again: with an overlapping series the last
        // interval by `from` need not be the one that reaches furthest.
        let last_to = covered_to.unwrap_or(first.to);
        if first.from > period_from {
            issues.push(gap_issue(
                period_from,
                first.from,
                expected_secs,
                Some(order[0]),
            ));
        }
        if period_to > last_to {
            issues.push(gap_issue(last_to, period_to, expected_secs, None));
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
/// The two passes occupy `[transition − 1 h, transition + 1 h)` in UTC — one at
/// UTC+2, one at UTC+1 — and this looks only there. Comparing the whole day's
/// covered duration against 25 hours instead would report any two missing
/// quarter-hours anywhere on the day as a collapsed hour, which sends the
/// reader to the wrong place. A gap at midday is a V01 gap and nothing else; a
/// genuinely collapsed hour shows up here even on a day that is otherwise
/// complete.
///
/// The rule only judges a series that demonstrably **spans** that window, so a
/// truncated query window is short rather than corrupt.
///
/// Every Berlin calendar day the series spans is examined, not only the first —
/// a month of MSCONS, an annual export or a MaBiS Summenzeitreihe can each hold
/// a fall-back day anywhere inside it. The transition comes from the tz
/// database, which has not always put it on the last Sunday in October.
fn detect_dst_ambiguity(intervals: &[&MeterInterval]) -> Vec<ValidationIssue> {
    let (Some(first), Some(last)) = (intervals.first(), intervals.last()) else {
        return Vec::new();
    };

    let mut issues = Vec::new();
    let end = crate::calendar::local_day(last.to);
    let mut day = crate::calendar::local_day(first.from);
    while day <= end {
        if crate::calendar::day_kind(day) == crate::calendar::DayKind::LongDay
            && let Some(issue) = collapsed_repeated_hour(intervals, day)
        {
            issues.push(issue);
        }
        let Some(next) = day.next_day() else { break };
        day = next;
    }
    issues
}

/// The V07 finding for one fall-back day, when that day's repeated hour is
/// demonstrably collapsed.
fn collapsed_repeated_hour(
    intervals: &[&MeterInterval],
    local_day: time::Date,
) -> Option<ValidationIssue> {
    let (first, last) = (intervals.first()?, intervals.last()?);
    let transition = crate::calendar::dst_transition_utc(local_day)?;

    let window_start = transition - Duration::hours(1);
    let window_end = transition + Duration::hours(1);

    // Only judge a series that covers the window at both ends; anything else is
    // a truncated read, not a collapsed hour.
    if first.from > window_start || last.to < window_end {
        return None;
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
        return None;
    }

    Some(
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
    )
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

    /// The false positive this rule must not produce: a gap at **midday** on a
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

    /// The failure the day walk fixes: a collapsed hour inside a **multi-day**
    /// delivery. Keying off the day the series starts on meant a month of
    /// MSCONS, an annual export and every MaBiS Summenzeitreihe escaped V07
    /// entirely — the very deliveries where the loss is invisible by eye.
    #[test]
    fn a_collapsed_hour_is_found_anywhere_in_the_series() {
        // 24 Oct (96 quarter-hours) + 25 Oct (100), with the second pass of the
        // repeated hour dropped.
        let mut span = qh(datetime!(2026-10-23 22:00 UTC), 96 + 100);
        span.retain(|iv| {
            !(iv.from >= datetime!(2026-10-25 1:00 UTC) && iv.from < datetime!(2026-10-25 2:00 UTC))
        });
        let issues = detect(&span);
        assert_eq!(
            issues.len(),
            1,
            "expected V07 on the interior day: {issues:?}"
        );
        assert_eq!(issues[0].rule_id, ValidationRuleId::DstAmbiguity);
        assert!(
            issues[0].message.contains("2026-10-25"),
            "{}",
            issues[0].message
        );

        // ...and through the public entry point.
        let report = validate_intervals(&span, &ValidationConfig::default());
        assert_eq!(report.by_rule(ValidationRuleId::DstAmbiguity).count(), 1);

        // An intact multi-day span raises nothing.
        assert!(detect(&qh(datetime!(2026-10-23 22:00 UTC), 96 + 100)).is_empty());
    }

    /// Two fall-back days in one series are two findings.
    #[test]
    fn each_fall_back_day_is_judged_on_its_own() {
        let drop_repeat = |ivs: &mut Vec<MeterInterval>, from: OffsetDateTime| {
            ivs.retain(|iv| !(iv.from >= from && iv.from < from + Duration::hours(1)));
        };
        // 2026-10-25 and 2027-10-31 in one (sparse but contiguous) series would
        // be a year of data; assemble the two days plus the day between them
        // instead, which is what the walk actually needs to reach both.
        let mut a = qh(datetime!(2026-10-24 22:00 UTC), 100);
        drop_repeat(&mut a, datetime!(2026-10-25 1:00 UTC));
        assert_eq!(detect(&a).len(), 1);

        let mut b = qh(datetime!(2027-10-30 22:00 UTC), 100);
        drop_repeat(&mut b, datetime!(2027-10-31 1:00 UTC));
        assert_eq!(detect(&b).len(), 1);
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

#[cfg(test)]
mod gap_grid_tests {
    use super::*;
    use crate::interval::QualityFlag;
    use rust_decimal::dec;
    use time::Duration;
    use time::macros::datetime;

    fn iv(from: OffsetDateTime, to: OffsetDateTime) -> MeterInterval {
        MeterInterval {
            from,
            to,
            value: dec!(1),
            quality: QualityFlag::Measured,
            obis_code: None,
        }
    }

    /// Two zeros either side of a hole are not four consecutive zeros: what
    /// happened in the hole is unknown, and reporting a stuck meter across it
    /// claims a measurement nobody took.
    #[test]
    fn a_gap_ends_a_zero_run() {
        let zero = |from: OffsetDateTime, to: OffsetDateTime| MeterInterval {
            value: dec!(0),
            ..iv(from, to)
        };
        // Four zeros, but the middle two are an hour later — one hole between
        // two runs of two, and the threshold is four.
        let series = vec![
            zero(
                datetime!(2026-06-01 0:00 UTC),
                datetime!(2026-06-01 0:15 UTC),
            ),
            zero(
                datetime!(2026-06-01 0:15 UTC),
                datetime!(2026-06-01 0:30 UTC),
            ),
            zero(
                datetime!(2026-06-01 1:30 UTC),
                datetime!(2026-06-01 1:45 UTC),
            ),
            zero(
                datetime!(2026-06-01 1:45 UTC),
                datetime!(2026-06-01 2:00 UTC),
            ),
        ];
        let result = validate_intervals(&series, &ValidationConfig::default());
        assert_eq!(
            result.by_rule(ValidationRuleId::SuspiciousZeroRun).count(),
            0,
            "two runs of two, not one run of four"
        );

        // Contiguous, and the same four zeros do trip it.
        let contiguous: Vec<_> = (0..4)
            .map(|i| {
                zero(
                    datetime!(2026-06-01 0:00 UTC) + Duration::minutes(15 * i),
                    datetime!(2026-06-01 0:15 UTC) + Duration::minutes(15 * i),
                )
            })
            .collect();
        let tripped = validate_intervals(&contiguous, &ValidationConfig::default());
        assert_eq!(
            tripped.by_rule(ValidationRuleId::SuspiciousZeroRun).count(),
            1
        );
    }

    /// A swallowed interval leaves no gap behind it.
    ///
    /// Sorted by `from`, the short interval inside a long one is followed by a
    /// start earlier than the long one's end, so a pairwise comparison reports
    /// the space *after* the short one as missing — while the long interval
    /// covers it. An overlapping series is already an error; a second, wrong
    /// finding on top of it sends the reader to a slot that has data.
    #[test]
    fn an_overlap_does_not_manufacture_a_gap() {
        let series = vec![
            iv(
                datetime!(2026-06-01 0:00 UTC),
                datetime!(2026-06-01 1:00 UTC),
            ),
            iv(
                datetime!(2026-06-01 0:15 UTC),
                datetime!(2026-06-01 0:30 UTC),
            ),
        ];
        let result = validate_intervals(&series, &ValidationConfig::default());
        assert_eq!(result.by_rule(ValidationRuleId::OverlapDetected).count(), 1);
        assert_eq!(
            result.by_rule(ValidationRuleId::GapDetected).count(),
            0,
            "the hour covers 00:30–01:00, so nothing is missing"
        );
    }

    /// The same reasoning at the tail: the interval that reaches furthest is
    /// not always the last one by `from`.
    #[test]
    fn the_tail_gap_is_measured_from_the_furthest_end() {
        let series = vec![
            iv(
                datetime!(2026-06-01 0:00 UTC),
                datetime!(2026-06-01 1:00 UTC),
            ),
            iv(
                datetime!(2026-06-01 0:15 UTC),
                datetime!(2026-06-01 0:30 UTC),
            ),
        ];
        let cfg = ValidationConfig::default().over_period(
            datetime!(2026-06-01 0:00 UTC),
            datetime!(2026-06-01 1:00 UTC),
        );
        let result = validate_intervals(&series, &cfg);
        assert_eq!(
            result.by_rule(ValidationRuleId::GapDetected).count(),
            0,
            "measured from 01:00, not from the swallowed interval's 00:30"
        );
    }

    /// A hole shorter than one interval is still a hole. V06 cannot see it —
    /// every interval is the right length — so without this a series sitting
    /// off the grid validates clean while five minutes of energy per slot goes
    /// unaccounted for.
    #[test]
    fn a_sub_interval_hole_is_still_a_gap() {
        let series = vec![
            iv(
                datetime!(2026-06-01 0:00 UTC),
                datetime!(2026-06-01 0:15 UTC),
            ),
            iv(
                datetime!(2026-06-01 0:20 UTC),
                datetime!(2026-06-01 0:35 UTC),
            ),
        ];
        let report = validate_intervals(&series, &ValidationConfig::default());
        let gaps: Vec<_> = report.by_rule(ValidationRuleId::GapDetected).collect();
        assert_eq!(gaps.len(), 1, "{:?}", report.issues);
        assert_eq!(gaps[0].severity, ValidationSeverity::Error);
        assert!(gaps[0].message.contains("300 s"), "{}", gaps[0].message);
        assert!(gaps[0].message.contains("off-grid"), "{}", gaps[0].message);
        // ...while every interval is exactly the expected length, so V06 stays
        // silent — the two rules answer different questions.
        assert_eq!(
            report
                .by_rule(ValidationRuleId::InconsistentIntervalLength)
                .count(),
            0
        );
    }

    /// The head and tail of a declared period are held to the same standard.
    #[test]
    fn a_sub_interval_hole_at_the_edges_is_reported_too() {
        let series = vec![iv(
            datetime!(2026-06-01 0:05 UTC),
            datetime!(2026-06-01 0:20 UTC),
        )];
        let cfg = ValidationConfig::default().over_period(
            datetime!(2026-06-01 0:00 UTC),
            datetime!(2026-06-01 0:25 UTC),
        );
        let report = validate_intervals(&series, &cfg);
        assert_eq!(
            report.by_rule(ValidationRuleId::GapDetected).count(),
            2,
            "five minutes short at each end: {:?}",
            report.issues
        );
    }

    /// A contiguous series is still clean — the looser test must not turn
    /// every boundary into a finding.
    #[test]
    fn a_contiguous_series_remains_clean() {
        let series: Vec<_> = (0..96)
            .map(|i| {
                let from = datetime!(2026-06-01 0:00 UTC) + Duration::minutes(15 * i);
                iv(from, from + Duration::minutes(15))
            })
            .collect();
        let cfg = ValidationConfig::default().over_period(
            datetime!(2026-06-01 0:00 UTC),
            datetime!(2026-06-02 0:00 UTC),
        );
        assert!(
            validate_intervals(&series, &cfg).is_clean(),
            "{:?}",
            validate_intervals(&series, &cfg).issues
        );
    }
}

#[cfg(test)]
mod rule_set_tests {
    use super::*;
    use crate::interval::QualityFlag;
    use rust_decimal::dec;
    use time::macros::datetime;

    fn day(n: i64) -> Vec<MeterInterval> {
        (0..n)
            .map(|i| {
                let from = datetime!(2026-06-01 0:00 UTC) + Duration::minutes(15 * i);
                MeterInterval {
                    from,
                    to: from + Duration::minutes(15),
                    value: dec!(2),
                    quality: QualityFlag::Measured,
                    obis_code: None,
                }
            })
            .collect()
    }

    /// The failure this exists to surface: the per-commodity configuration
    /// cannot fire V12, so a service that assumed otherwise would describe an
    /// Error-severity rule it never evaluated.
    #[test]
    fn the_per_commodity_defaults_leave_v12_inert_and_say_so() {
        let cfg = crate::QualityConfig::for_sparte(crate::Sparte::Strom);
        let off = cfg.validation.disabled_rules();

        assert!(
            off.contains(ValidationRuleId::ImplausiblePower),
            "for_sparte supplies no plant capacity, so V12 cannot fire"
        );
        assert!(
            off.contains(ValidationRuleId::FutureTimestamp),
            "nor a `now`"
        );
        assert_eq!(off.to_string(), "V08, V12");
        assert_eq!(off.len(), 2);

        // ...and each names the field that would arm it.
        assert_eq!(
            ValidationRuleId::ImplausiblePower.enabling_field(),
            Some("max_plant_power_kw")
        );
        assert_eq!(
            ValidationRuleId::FutureTimestamp.enabling_field(),
            Some("now")
        );
        assert_eq!(ValidationRuleId::OverlapDetected.enabling_field(), None);

        // Supplying the ceiling arms it, and the report then says it ran.
        let armed = cfg.validation.clone().with_plant_capacity_kw(dec!(30));
        assert!(
            armed
                .enabled_rules()
                .contains(ValidationRuleId::ImplausiblePower)
        );
        let report = validate_intervals(&day(96), &armed);
        assert!(
            report
                .evaluated
                .contains(ValidationRuleId::ImplausiblePower)
        );
        assert!(report.is_clean(), "{:?}", report.issues);
    }

    /// Every field that can turn a rule off is accounted for, and no rule is
    /// claimed as enabled without the number it needs.
    #[test]
    fn every_optional_field_maps_to_the_rules_it_arms() {
        use ValidationRuleId as R;
        let full = ValidationConfig::default()
            .with_plant_capacity_kw(dec!(30))
            .at_reference_instant(datetime!(2026-06-02 0:00 UTC));
        assert_eq!(full.enabled_rules(), RuleSet::ALL);
        assert!(full.disabled_rules().is_empty());

        for (mutate, expected) in [
            (
                Box::new(|c: ValidationConfig| ValidationConfig {
                    expected_interval_secs: None,
                    ..c
                }) as Box<dyn Fn(ValidationConfig) -> ValidationConfig>,
                vec![R::GapDetected, R::InconsistentIntervalLength],
            ),
            (
                Box::new(|c: ValidationConfig| c.without_outlier_detection()),
                vec![R::StatisticalOutlier],
            ),
            (
                Box::new(|c: ValidationConfig| ValidationConfig { now: None, ..c }),
                vec![R::FutureTimestamp],
            ),
            (
                Box::new(|c: ValidationConfig| ValidationConfig {
                    max_plant_power_kw: None,
                    ..c
                }),
                vec![R::ImplausiblePower],
            ),
            (
                Box::new(|c: ValidationConfig| ValidationConfig {
                    negative_energy_is_error: false,
                    ..c
                }),
                vec![R::NegativeEnergy],
            ),
        ] {
            let off = mutate(full.clone()).disabled_rules();
            assert_eq!(
                off,
                expected.iter().copied().collect::<RuleSet>(),
                "expected exactly {expected:?} to go inert"
            );
        }
    }

    /// The config permits a rule; the data can still stop it. V04 needs more
    /// points than its window is wide, so `evaluated` is the smaller set and
    /// the difference tells the caller which of the two was responsible.
    #[test]
    fn the_data_can_disable_a_rule_the_config_armed() {
        let cfg = ValidationConfig::default(); // outlier_window = 12
        assert!(
            cfg.enabled_rules()
                .contains(ValidationRuleId::StatisticalOutlier)
        );

        let short = validate_intervals(&day(20), &cfg);
        assert!(
            !short
                .evaluated
                .contains(ValidationRuleId::StatisticalOutlier),
            "20 intervals cannot fill a 25-point window"
        );
        assert!(
            short
                .skipped()
                .contains(ValidationRuleId::StatisticalOutlier)
        );

        let long = validate_intervals(&day(96), &cfg);
        assert!(
            long.evaluated
                .contains(ValidationRuleId::StatisticalOutlier)
        );

        // The config said yes both times — so the *data* is what differed.
        assert!(
            cfg.enabled_rules()
                .contains(ValidationRuleId::StatisticalOutlier)
        );
    }

    /// An empty series evaluates at most V01: there is no interval for any
    /// other rule to look at, and claiming ten clean rules over nothing would
    /// be the same lie in a different place.
    #[test]
    fn an_empty_series_evaluates_almost_nothing() {
        let cfg = ValidationConfig::default().over_period(
            datetime!(2026-06-01 0:00 UTC),
            datetime!(2026-06-02 0:00 UTC),
        );
        let report = validate_intervals(&[], &cfg);
        assert_eq!(
            report.evaluated,
            RuleSet::EMPTY.with(ValidationRuleId::GapDetected)
        );
        assert!(report.has_errors());

        // Without a grid there is not even that.
        let blind = ValidationConfig {
            expected_interval_secs: None,
            ..ValidationConfig::default()
        };
        assert!(validate_intervals(&[], &blind).evaluated.is_empty());
    }

    #[test]
    fn rule_set_is_a_set() {
        use ValidationRuleId as R;
        let s = RuleSet::EMPTY.with(R::GapDetected).with(R::GapDetected);
        assert_eq!(s.len(), 1, "a set holds a rule once");
        assert!(s.contains(R::GapDetected) && !s.contains(R::OverlapDetected));
        assert_eq!(s.without(R::GapDetected), RuleSet::EMPTY);
        assert_eq!(RuleSet::ALL.len(), ValidationRuleId::ALL.len());
        assert_eq!(RuleSet::ALL.complement(), RuleSet::EMPTY);
        assert_eq!(RuleSet::EMPTY.complement(), RuleSet::ALL);
        assert_eq!(RuleSet::EMPTY.to_string(), "none");
        assert_eq!(
            RuleSet::ALL.to_string(),
            "V01, V02, V03, V04, V05, V06, V07, V08, V09, V11, V12"
        );
        // Iteration is in ALL order, and collects back to the same set.
        assert_eq!(RuleSet::ALL.iter().collect::<RuleSet>(), RuleSet::ALL);
        assert_eq!(
            RuleSet::ALL.iter().collect::<Vec<_>>(),
            ValidationRuleId::ALL.to_vec()
        );
        assert_eq!(s.union(RuleSet::ALL), RuleSet::ALL);
        assert_eq!(s.intersection(RuleSet::EMPTY), RuleSet::EMPTY);
    }

    /// A grade summarises the rules that ran, and says which those were.
    #[test]
    fn a_grade_reports_what_it_speaks_for() {
        let full = crate::QualityConfig {
            validation: ValidationConfig::default()
                .with_plant_capacity_kw(dec!(1000))
                .at_reference_instant(datetime!(2026-06-02 0:00 UTC)),
            ..crate::QualityConfig::default()
        };
        let report = crate::score_intervals(&day(96), &full);
        assert_eq!(report.grade, crate::QualityGrade::A);
        assert!(report.covers_every_rule(), "{}", report.skipped_rules());

        // The per-commodity defaults grade A on nine rules, and say so.
        let partial = crate::score_intervals(&day(96), &crate::QualityConfig::default());
        assert_eq!(partial.grade, crate::QualityGrade::A);
        assert!(!partial.covers_every_rule());
        assert_eq!(partial.skipped_rules().to_string(), "V08, V12");
    }
}

#[cfg(test)]
mod zero_run_tests {
    use super::*;
    use crate::interval::QualityFlag;
    use rust_decimal::dec;
    use time::macros::datetime;

    /// `n` quarter-hours from `start`, with the values given.
    fn series(values: &[Decimal]) -> Vec<MeterInterval> {
        values
            .iter()
            .enumerate()
            .map(|(i, &value)| {
                let from = datetime!(2026-06-01 0:00 UTC) + Duration::minutes(15 * i as i64);
                MeterInterval {
                    from,
                    to: from + Duration::minutes(15),
                    value,
                    quality: QualityFlag::Measured,
                    obis_code: None,
                }
            })
            .collect()
    }

    fn cfg() -> ValidationConfig {
        ValidationConfig {
            outlier_sigma: None,
            ..ValidationConfig::default()
        }
    }

    /// The number in the message is the run, not the threshold: it is the
    /// figure a reader acts on.
    #[test]
    fn the_finding_carries_the_real_run_length() {
        let mut values = vec![dec!(2); 40];
        for v in values.iter_mut().skip(4).take(30) {
            *v = Decimal::ZERO;
        }
        let result = validate_intervals(&series(&values), &cfg());
        let found: Vec<&ValidationIssue> = result
            .by_rule(ValidationRuleId::SuspiciousZeroRun)
            .collect();
        assert_eq!(found.len(), 1, "one finding per run: {found:?}");
        assert!(
            found[0].message.starts_with("30 consecutive"),
            "{}",
            found[0].message
        );
        assert_eq!(
            found[0].interval_index,
            Some(4),
            "anchored at the run start"
        );
    }

    /// Two outages are two findings, not one merged one and not one per zero.
    #[test]
    fn each_run_is_reported_once() {
        let mut values = vec![dec!(2); 40];
        for i in [4, 5, 6, 7, 20, 21, 22, 23, 24] {
            values[i] = Decimal::ZERO;
        }
        let result = validate_intervals(&series(&values), &cfg());
        let lengths: Vec<&str> = result
            .by_rule(ValidationRuleId::SuspiciousZeroRun)
            .map(|i| i.message.split(' ').next().unwrap_or(""))
            .collect();
        assert_eq!(lengths, ["4", "5"]);
    }

    /// A run still open at the end of the series is reported — the earlier
    /// implementation happened to catch it, and the new one must too.
    #[test]
    fn a_run_that_never_ends_is_still_reported() {
        let mut values = vec![dec!(2); 10];
        for v in values.iter_mut().skip(6) {
            *v = Decimal::ZERO;
        }
        let result = validate_intervals(&series(&values), &cfg());
        assert_eq!(
            result.by_rule(ValidationRuleId::SuspiciousZeroRun).count(),
            1
        );
    }

    /// A run below the threshold is silence, and a threshold of zero is the
    /// rule switched off — which `enabled_rules` has to admit.
    #[test]
    fn the_threshold_is_a_switch_the_ruleset_reports() {
        let values = vec![dec!(2), Decimal::ZERO, Decimal::ZERO, dec!(2)];
        let result = validate_intervals(&series(&values), &cfg());
        assert!(
            result.is_clean(),
            "two zeros is under the threshold of four"
        );

        let off = ValidationConfig {
            zero_run_threshold: 0,
            ..cfg()
        };
        assert!(
            !off.enabled_rules()
                .contains(ValidationRuleId::SuspiciousZeroRun),
            "a zero threshold is off, and a clean report must not claim otherwise"
        );
        assert!(
            off.disabled_rules()
                .contains(ValidationRuleId::SuspiciousZeroRun)
        );
        assert_eq!(
            ValidationRuleId::SuspiciousZeroRun.enabling_field(),
            Some("zero_run_threshold"),
        );

        let all_zero = vec![Decimal::ZERO; 20];
        assert!(validate_intervals(&series(&all_zero), &off).is_clean());
    }

    /// Shuffled input reports the same runs: the rule walks timestamp order.
    #[test]
    fn runs_are_found_in_timestamp_order_not_slice_order() {
        let mut values = vec![dec!(2); 20];
        for v in values.iter_mut().skip(8).take(6) {
            *v = Decimal::ZERO;
        }
        let ordered = series(&values);
        let mut shuffled = ordered.clone();
        shuffled.reverse();

        let a = validate_intervals(&ordered, &cfg());
        let b = validate_intervals(&shuffled, &cfg());
        let msg = |r: &ValidationResult| {
            r.by_rule(ValidationRuleId::SuspiciousZeroRun)
                .map(|i| i.message.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(msg(&a), msg(&b));
        assert!(msg(&a)[0].starts_with("6 consecutive"), "{:?}", msg(&a));
    }
}

#[cfg(test)]
mod daily_length_tests {
    use super::*;
    use crate::calendar;
    use crate::interval::QualityFlag;
    use rust_decimal::dec;
    use time::Date;
    use time::macros::date;

    /// One interval per day of `days`, cut on `boundary`.
    fn daily(first: Date, days: i64, boundary: DayBoundary) -> Vec<MeterInterval> {
        let mut out = Vec::new();
        let mut day = first;
        for _ in 0..days {
            out.push(MeterInterval {
                from: boundary.day_start_utc(day),
                to: boundary.day_end_utc(day),
                value: dec!(100),
                quality: QualityFlag::Measured,
                obis_code: None,
            });
            day = day.next_day().expect("in range");
        }
        out
    }

    fn daily_cfg(boundary: DayBoundary) -> ValidationConfig {
        ValidationConfig {
            expected_interval_secs: Some(86_400),
            outlier_sigma: None,
            ..ValidationConfig::default()
        }
        .on(boundary)
    }

    /// The 23- and 25-hour days are the right length, on either boundary — a
    /// midnight-only allowance would warn on a daily Gastag series for being
    /// exactly right.
    #[test]
    fn a_daily_series_is_judged_against_its_own_boundarys_calendar() {
        for (boundary, spring, autumn) in [
            // The calendar day that holds the transition…
            (
                DayBoundary::Midnight,
                date!(2026 - 03 - 29),
                date!(2026 - 10 - 25),
            ),
            // …is the Saturday before it for gas: the clocks move at
            // 02:00/03:00, inside the Gastag that began the previous morning.
            (
                DayBoundary::Gastag,
                date!(2026 - 03 - 28),
                date!(2026 - 10 - 24),
            ),
        ] {
            assert_eq!(boundary.day_length(spring).whole_hours(), 23);
            assert_eq!(boundary.day_length(autumn).whole_hours(), 25);

            for day in [spring, autumn] {
                let series = daily(day, 1, boundary);
                let result = validate_intervals(&series, &daily_cfg(boundary));
                assert!(
                    result
                        .by_rule(ValidationRuleId::InconsistentIntervalLength)
                        .next()
                        .is_none(),
                    "{boundary:?} {day}: {:?}",
                    result.issues
                );
            }
        }
    }

    /// A week over the autumn transition validates clean end to end — no gap,
    /// no length warning, on either boundary.
    #[test]
    fn a_week_of_gas_days_across_the_transition_is_clean() {
        let series = daily(date!(2026 - 10 - 21), 7, DayBoundary::Gastag);
        let cfg = daily_cfg(DayBoundary::Gastag).over_period(
            calendar::gas_day_start_utc(date!(2026 - 10 - 21)),
            calendar::gas_day_end_utc(date!(2026 - 10 - 27)),
        );
        let result = validate_intervals(&series, &cfg);
        assert!(result.is_clean(), "{:?}", result.issues);
    }

    /// The allowance is for a *calendar* day only: a fixed 24-hour window that
    /// starts somewhere else is still held to 86 400 s.
    #[test]
    fn an_off_boundary_24_hour_window_gets_no_dst_allowance() {
        let from = calendar::day_start_utc(date!(2026 - 10 - 25)) + Duration::hours(9);
        let series = vec![MeterInterval {
            from,
            to: from + Duration::hours(25),
            value: dec!(100),
            quality: QualityFlag::Measured,
            obis_code: None,
        }];
        let result = validate_intervals(&series, &daily_cfg(DayBoundary::Midnight));
        assert_eq!(
            result
                .by_rule(ValidationRuleId::InconsistentIntervalLength)
                .count(),
            1,
            "a 25-hour window that is not a calendar day is 25 hours too long"
        );
    }
}
