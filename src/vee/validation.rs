//! Plausibilisierung — the content rules a validated [`Series`] is checked
//! against, and the [`Report`] they produce.
//!
//! ## Rules
//!
//! | [`Rule`] | Severity | What it catches | Basis |
//! |---|---|---|---|
//! | `GAP` | error | a slot of the declared period with no value — head, tail or interior | MeteringCode 2006 A7.1.2.1/A7.1.2.2 |
//! | `NON_BILLABLE` | error | a value flagged `Faulty` or `Unknown` | A7.1.2.3 (status information) |
//! | `NEGATIVE` | error | a negative quantity on a MaKo channel | Codeliste OBIS 2.5c § 2.1 |
//! | `CAPACITY` | error | average power above the plant's capacity | A7.2 (Grenzleistung der Anlage) |
//! | `FUTURE` | blocking | a slot that has not ended at the caller's `now` | — |
//! | `LAENGSVERGLEICH` | blocking | Σ load curve ≠ register advance beyond a tolerance | A7.2 (Längsvergleich) |
//! | `OUTSIDE_PERIOD` | warning | data outside the declared period | — |
//! | `ZERO_RUN` | warning | a run of zeros at least as long as the configured duration | A7.2 (Prüfung auf Nullwerte) |
//! | `STALE` | warning | a non-zero value frozen for a window of slots | pvanalytics `stale_values_diff` |
//! | `SPIKE` | warning | a value far from its neighbours (Hampel, MAD floor) | A7.2 (relative Messwerte) |
//! | `SEASONAL` | warning | a value far from the same slot on comparable days | A7.2 (relative Messwerte) |
//!
//! [`substitute`](crate::vee::substitute::substitute) repairs the slots an
//! **error** covers, never a **blocking** finding. The [`Grade`] follows from
//! the findings: `A` none, `B` warnings only, `C` errors, `F` anything blocking.
//!
//! MeteringCode 2006 Anlage 7 (superseded by VDE-AR-N 4400) names the checks,
//! not their limits; the limits are [`Rules`] parameters with presets per [`Sparte`].
//!
//! ```rust
//! use metering::prelude::*;
//! use metering::vee::validation::{Grade, Rule};
//! use time::{Duration, macros::{date, datetime}};
//!
//! let day = DayBoundary::Strom.day(date!(2026 - 06 - 01)).unwrap();
//! let slots = (0..96)
//!     .filter(|i| *i != 40) // one quarter-hour never arrived
//!     .map(|i| MeterInterval::quarter_hour(day.start() + Duration::minutes(15 * i), dec!(1.2), QualityFlag::Measured))
//!     .collect::<Result<Vec<_>, _>>()?;
//! let series = Series::new(Resolution::QUARTER_HOUR, DayBoundary::Strom, slots)?;
//!
//! let report = validate(&series, &Rules::strom(day, datetime!(2026-07-01 0:00 UTC), None));
//! assert_eq!(report.grade(), Grade::C);
//! assert_eq!(report.by_rule(Rule::Gap).count(), 1);
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

use rust_decimal::{Decimal, dec};
use time::{Duration, OffsetDateTime};

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use crate::precision::{PERCENT_DP, PERCENT_STRATEGY};
use crate::series::Series;
use crate::series::interval::{MeterInterval, Sparte};
use crate::series::reading::MeterReading;
use crate::time::calendar::{DayKind, Period};
use crate::time::holiday::Bundesland;
use crate::vee::{Data, hampel, is_actual, references, slots};

/// A content rule — see the [module table](self#rules).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum Rule {
    /// A slot of the declared period with no value.
    Gap,
    /// A value whose quality flag is not billable.
    NonBillable,
    /// A negative quantity.
    Negative,
    /// Average power above the plant capacity.
    Capacity,
    /// A slot that had not ended at `now`.
    Future,
    /// Σ load curve against the register advance.
    Laengsvergleich,
    /// Data outside the declared period.
    OutsidePeriod,
    /// A run of zeros.
    ZeroRun,
    /// A frozen non-zero value.
    Stale,
    /// A value far from its neighbours.
    Spike,
    /// A value far from the same slot on comparable days.
    Seasonal,
}

impl Rule {
    /// Every rule, in declaration order.
    pub const ALL: [Self; 11] = [
        Self::Gap,
        Self::NonBillable,
        Self::Negative,
        Self::Capacity,
        Self::Future,
        Self::Laengsvergleich,
        Self::OutsidePeriod,
        Self::ZeroRun,
        Self::Stale,
        Self::Spike,
        Self::Seasonal,
    ];

    /// Stable code; the `serde` tag and [`FromStr`](std::str::FromStr) input.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Gap => "GAP",
            Self::NonBillable => "NON_BILLABLE",
            Self::Negative => "NEGATIVE",
            Self::Capacity => "CAPACITY",
            Self::Future => "FUTURE",
            Self::Laengsvergleich => "LAENGSVERGLEICH",
            Self::OutsidePeriod => "OUTSIDE_PERIOD",
            Self::ZeroRun => "ZERO_RUN",
            Self::Stale => "STALE",
            Self::Spike => "SPIKE",
            Self::Seasonal => "SEASONAL",
        }
    }

    /// How serious a finding of this rule is; fixed per rule.
    #[must_use]
    pub const fn severity(self) -> Severity {
        match self {
            Self::Gap | Self::NonBillable | Self::Negative | Self::Capacity => Severity::Error,
            Self::Future | Self::Laengsvergleich => Severity::Blocking,
            Self::OutsidePeriod | Self::ZeroRun | Self::Stale | Self::Spike | Self::Seasonal => {
                Severity::Warning
            }
        }
    }
}

/// How serious a finding is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum Severity {
    /// Worth a look; the values stay billable.
    Warning,
    /// The slots it covers must not be billed as they are; a substitute repairs them.
    Error,
    /// Not repairable by a substitute: the series as a whole needs clarifying.
    Blocking,
}

impl Severity {
    /// Every severity, least to most serious.
    pub const ALL: [Self; 3] = [Self::Warning, Self::Error, Self::Blocking];

    /// Stable code; the `serde` tag and [`FromStr`](std::str::FromStr) input.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Warning => "WARNING",
            Self::Error => "ERROR",
            Self::Blocking => "BLOCKING",
        }
    }
}

/// The verdict over a [`Report`] — a pure function of its findings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum Grade {
    /// No finding.
    A,
    /// Warnings only: bill it and note it.
    B,
    /// Errors a substitute can repair.
    C,
    /// A blocking finding.
    F,
}

impl Grade {
    /// Every grade, best first.
    pub const ALL: [Self; 4] = [Self::A, Self::B, Self::C, Self::F];

    /// Stable code; the `serde` tag and [`FromStr`](std::str::FromStr) input.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::A => "A",
            Self::B => "B",
            Self::C => "C",
            Self::F => "F",
        }
    }

    /// The grade of a set of findings.
    #[must_use]
    pub fn of<'a>(findings: impl IntoIterator<Item = &'a Finding>) -> Self {
        match findings.into_iter().map(|f| f.rule.severity()).max() {
            None => Self::A,
            Some(Severity::Warning) => Self::B,
            Some(Severity::Error) => Self::C,
            Some(Severity::Blocking) => Self::F,
        }
    }
}

crate::ids::codes::string_codes! {
    Rule;
    Severity;
    Grade;
}

/// One finding, over the half-open span `[from, to)` it concerns.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[non_exhaustive]
pub struct Finding {
    /// The rule that fired.
    pub rule: Rule,
    /// First instant concerned (UTC).
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::rfc3339"))]
    pub from: OffsetDateTime,
    /// End of the span concerned (UTC, exclusive).
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::rfc3339"))]
    pub to: OffsetDateTime,
    /// The observed quantity: the slot's value, or Σ load curve for
    /// `LAENGSVERGLEICH`.
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::decimal_option"))]
    pub value: Option<Decimal>,
    /// What it was compared with: the median, the capacity in kW, the
    /// register advance, or the count of missing slots for `GAP`.
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::decimal_option"))]
    pub reference: Option<Decimal>,
    /// A sentence for a human.
    pub message: String,
}

impl Finding {
    fn new(rule: Rule, from: OffsetDateTime, to: OffsetDateTime, message: String) -> Self {
        Self {
            rule,
            from,
            to,
            value: None,
            reference: None,
            message,
        }
    }

    fn data(mut self, value: Option<Decimal>, reference: Option<Decimal>) -> Self {
        self.value = value;
        self.reference = reference;
        self
    }

    /// The finding's severity — [`Rule::severity`].
    #[must_use]
    pub const fn severity(&self) -> Severity {
        self.rule.severity()
    }
}

/// How much of a period a series covers, both as durations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[non_exhaustive]
pub struct Coverage {
    /// Time covered by intervals, clipped to the period.
    pub covered: Duration,
    /// The period's length.
    pub expected: Duration,
}

impl Coverage {
    /// Covered over expected, in percent, rounded to [`PERCENT_DP`]. `None` for an empty period.
    #[must_use]
    pub fn pct(&self) -> Option<Decimal> {
        let expected = self.expected.whole_seconds();
        if expected <= 0 {
            return None;
        }
        Some(
            (Decimal::from(self.covered.whole_seconds()) * Decimal::ONE_HUNDRED
                / Decimal::from(expected))
            .round_dp_with_strategy(PERCENT_DP, PERCENT_STRATEGY),
        )
    }

    /// `true` when every instant of the period is covered.
    #[must_use]
    pub fn is_complete(&self) -> bool {
        self.covered == self.expected
    }
}

/// Each interval `include` accepts, clipped to `period` and summed (a [`Series`] has no overlaps).
pub(crate) fn coverage(
    series: &Series,
    period: &Period,
    include: impl Fn(&MeterInterval) -> bool,
) -> Coverage {
    let covered = series
        .iter()
        .filter(|iv| include(iv))
        .map(|iv| {
            let from = iv.from().max(period.start());
            let to = iv.to().min(period.end());
            (to - from).max(Duration::ZERO)
        })
        .fold(Duration::ZERO, |sum, d| sum + d);
    Coverage {
        covered,
        expected: period.duration(),
    }
}

/// What [`validate`] found.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[non_exhaustive]
pub struct Report {
    /// The period the series was declared to cover.
    pub period: Period,
    /// The findings, by start instant, then rule.
    pub findings: Vec<Finding>,
    /// The rules that ran; [`skipped`](Self::skipped) names the rest.
    pub evaluated: Vec<Rule>,
    /// Delivered intervals (any quality) against the period.
    pub coverage: Coverage,
    /// Fingerprint of the validated series: interval count and span.
    pub(crate) len: usize,
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::rfc3339_option"))]
    pub(crate) first: Option<OffsetDateTime>,
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::rfc3339_option"))]
    pub(crate) end: Option<OffsetDateTime>,
}

impl Report {
    /// `true` when this report was produced for `series`.
    pub(crate) fn describes(&self, series: &Series) -> bool {
        self.len == series.len() && self.first.zip(self.end) == series.span()
    }

    /// `A` none, `B` warnings only, `C` errors, `F` blocking — see [`Grade::of`].
    #[must_use]
    pub fn grade(&self) -> Grade {
        Grade::of(&self.findings)
    }

    /// The rules that did not run — switched off, or stopped by the data.
    #[must_use]
    pub fn skipped(&self) -> Vec<Rule> {
        Rule::ALL
            .into_iter()
            .filter(|r| !self.evaluated.contains(r))
            .collect()
    }

    /// The findings of one rule.
    pub fn by_rule(&self, rule: Rule) -> impl Iterator<Item = &Finding> {
        self.findings.iter().filter(move |f| f.rule == rule)
    }

    /// The findings at or above `severity`.
    pub fn at_least(&self, severity: Severity) -> impl Iterator<Item = &Finding> {
        self.findings
            .iter()
            .filter(move |f| f.severity() >= severity)
    }

    /// `true` when nothing was found.
    #[must_use]
    pub fn is_clean(&self) -> bool {
        self.findings.is_empty()
    }
}

/// The rule set [`validate`] applies: a preset per [`Sparte`], the facts only
/// the caller knows, and setters per rule.
///
/// The facts are constructor arguments: the declared `period` (head and tail
/// gaps, coverage), `now` (this crate reads no clock) and the plant's
/// `capacity_kw` (`None` switches `CAPACITY` off, listed in [`Report::skipped`]).
///
/// | Preset | zero run | stale | spike half-window | Hampel `t` | MAD floor |
/// |---|---|---|---|---|---|
/// | Strom | 1 h | 6 values, exact | 12 | 6 | 0.01 |
/// | Gas | 48 h | off | 12 | 6 | 0.01 |
/// | Wärme, Wasser | 720 h | off | 12 | 6 | 0.05, 0.001 |
///
/// No source states these numbers; each is a setter. The floor is in channel units per slot.
#[derive(Debug, Clone, PartialEq)]
pub struct Rules {
    period: Period,
    now: OffsetDateTime,
    capacity_kw: Option<Decimal>,
    negative: bool,
    zero_run: Option<Duration>,
    stale: Option<(usize, Decimal)>,
    spike: Option<usize>,
    threshold: Decimal,
    floor: Decimal,
    seasonal: Option<(Bundesland, usize)>,
    laengsvergleich: Option<(MeterReading, MeterReading, Decimal)>,
    history: Option<Series>,
}

impl Rules {
    /// The preset for `sparte`.
    #[must_use]
    pub fn new(
        sparte: Sparte,
        period: Period,
        now: OffsetDateTime,
        capacity_kw: Option<Decimal>,
    ) -> Self {
        let (zero_hours, stale, floor) = match sparte {
            Sparte::Strom => (1, Some((6, Decimal::ZERO)), dec!(0.01)),
            Sparte::Gas => (48, None, dec!(0.01)),
            Sparte::Waerme => (720, None, dec!(0.05)),
            Sparte::Wasser => (720, None, dec!(0.001)),
        };
        Self {
            period,
            now,
            capacity_kw,
            negative: true,
            zero_run: Some(Duration::hours(zero_hours)),
            stale,
            spike: Some(12),
            threshold: dec!(6),
            floor,
            seasonal: None,
            laengsvergleich: None,
            history: None,
        }
    }

    /// The electricity preset.
    #[must_use]
    pub fn strom(period: Period, now: OffsetDateTime, capacity_kw: Option<Decimal>) -> Self {
        Self::new(Sparte::Strom, period, now, capacity_kw)
    }

    /// The gas preset.
    #[must_use]
    pub fn gas(period: Period, now: OffsetDateTime, capacity_kw: Option<Decimal>) -> Self {
        Self::new(Sparte::Gas, period, now, capacity_kw)
    }

    /// The declared period.
    #[must_use]
    pub const fn period(&self) -> Period {
        self.period
    }

    /// Accept negative values — a Korrekturenergiemenge or a derived signed
    /// series (Codeliste OBIS 2.5c § 2.1 allows them only there).
    #[must_use]
    pub const fn without_negative(mut self) -> Self {
        self.negative = false;
        self
    }

    /// Report zero runs at least `min` long.
    #[must_use]
    pub const fn zero_run(mut self, min: Duration) -> Self {
        self.zero_run = Some(min);
        self
    }

    /// Do not report zero runs.
    #[must_use]
    pub const fn without_zero_run(mut self) -> Self {
        self.zero_run = None;
        self
    }

    /// Report a non-zero value repeated for at least `window` adjacent slots, each
    /// within absolute `tolerance` of the first (pvanalytics `stale_values_diff`).
    #[must_use]
    pub const fn stale(mut self, window: usize, tolerance: Decimal) -> Self {
        self.stale = Some((window, tolerance));
        self
    }

    /// Do not report stale values.
    #[must_use]
    pub const fn without_stale(mut self) -> Self {
        self.stale = None;
        self
    }

    /// Judge each value against `half_window` neighbours either side.
    #[must_use]
    pub const fn spike(mut self, half_window: usize) -> Self {
        self.spike = Some(half_window);
        self
    }

    /// Do not report spikes.
    #[must_use]
    pub const fn without_spike(mut self) -> Self {
        self.spike = None;
        self
    }

    /// The Hampel threshold `t`, in robust sigma, for `SPIKE` and `SEASONAL`.
    #[must_use]
    pub const fn threshold(mut self, t: Decimal) -> Self {
        self.threshold = t;
        self
    }

    /// The floor on the robust sigma, in channel units per slot.
    #[must_use]
    pub const fn floor(mut self, floor: Decimal) -> Self {
        self.floor = floor;
        self
    }

    /// Compare each value with the same slot on the previous `days` days of the same
    /// day class in `land` (holidays count as Sundays); needs that much history.
    #[must_use]
    pub const fn seasonal(mut self, land: Bundesland, days: usize) -> Self {
        self.seasonal = Some((land, days));
        self
    }

    /// Prior data for `SEASONAL`; never itself validated.
    #[must_use]
    pub fn history(mut self, prior: Series) -> Self {
        self.history = Some(prior);
        self
    }

    /// The Längsvergleich (MeteringCode 2006 A7.2): Σ load curve over
    /// `[start.at, end.at)` against `end.value − start.value`, as a relative
    /// `tolerance` (`0.007` = 0,7 %). Elexon BSCP502 § 4.1.5 recommends at
    /// most ±0,7 % for a weekly and ±5 % for a daily check; the caller picks.
    #[must_use]
    pub fn laengsvergleich(
        mut self,
        start: MeterReading,
        end: MeterReading,
        tolerance: Decimal,
    ) -> Self {
        self.laengsvergleich = Some((start, end, tolerance));
        self
    }
}

/// Check `series` against `rules`.
///
/// Runs are of adjacent intervals (`next.from == previous.to`), so a gap ends every run.
#[must_use]
pub fn validate(series: &Series, rules: &Rules) -> Report {
    let mut findings = Vec::new();
    let mut evaluated = vec![
        Rule::Gap,
        Rule::NonBillable,
        Rule::Future,
        Rule::OutsidePeriod,
    ];
    gaps(series, rules, &mut findings);
    per_interval(series, rules, &mut findings, &mut evaluated);
    if let Some(min) = rules.zero_run {
        evaluated.push(Rule::ZeroRun);
        runs(
            series,
            |a, _| a.is_zero(),
            |len, d| d >= min && len > 0,
            Rule::ZeroRun,
            &mut findings,
        );
    }
    if let Some((window, tol)) = rules.stale {
        evaluated.push(Rule::Stale);
        runs(
            series,
            |v, anchor| !v.is_zero() && v.checked_sub(anchor).is_some_and(|d| d.abs() <= tol),
            |len, _| len >= window.max(2),
            Rule::Stale,
            &mut findings,
        );
    }
    if spikes(series, rules, &mut findings) {
        evaluated.push(Rule::Spike);
    }
    if seasonal(series, rules, &mut findings) {
        evaluated.push(Rule::Seasonal);
    }
    if laengsvergleich(series, rules, &mut findings) {
        evaluated.push(Rule::Laengsvergleich);
    }
    findings.sort_by_key(|f| (f.from, f.rule));
    evaluated.sort_unstable();
    Report {
        period: rules.period,
        findings,
        evaluated,
        coverage: coverage(series, &rules.period, |_| true),
        len: series.len(),
        first: series.span().map(|(f, _)| f),
        end: series.span().map(|(_, e)| e),
    }
}

/// `GAP`: every slot of the period with no interval, as maximal runs.
fn gaps(series: &Series, rules: &Rules, out: &mut Vec<Finding>) {
    let p = rules.period;
    let mut run: Option<(OffsetDateTime, OffsetDateTime, u32)> = None;
    let mut close = |run: Option<(OffsetDateTime, OffsetDateTime, u32)>| {
        if let Some((from, to, n)) = run {
            out.push(
                Finding::new(Rule::Gap, from, to, gap_message(series, from, to, n))
                    .data(None, Some(Decimal::from(n))),
            );
        }
    };
    for slot in slots(series.boundary(), series.resolution(), p.start(), p.end()) {
        if series.get(slot.start()).is_some() {
            close(run.take());
        } else {
            run = Some(match run {
                Some((from, _, n)) => (from, slot.end(), n + 1),
                None => (slot.start(), slot.end(), 1),
            });
        }
    }
    close(run);
}

fn gap_message(series: &Series, from: OffsetDateTime, to: OffsetDateTime, n: u32) -> String {
    let span = series.span();
    let place = match span {
        None => "the series is empty",
        Some((first, _)) if to <= first => "before the first value",
        Some((_, last)) if from >= last => "after the last value",
        Some(_) => "between two values",
    };
    // The fall-back day repeats local 02:00–03:00: [t − 1 h, t + 1 h) in UTC.
    let boundary = series.boundary();
    let mut dst = String::new();
    let (Some(mut day), Some(last)) = (
        boundary.day_of(from),
        boundary.day_of(to - Duration::nanoseconds(1)),
    ) else {
        return format!("{n} slot(s) missing from {from} to {to}, {place}");
    };
    while day <= last {
        if let Some(period) = boundary.day(day)
            && period.kind() == DayKind::LongDay
            && let Some(t) = period.transition()
            && from < t + Duration::hours(1)
            && to > t - Duration::hours(1)
        {
            dst = format!("; it includes the repeated hour 02:00–03:00 of {day}");
        }
        let Some(next) = day.next_day() else { break };
        day = next;
    }
    format!("{n} slot(s) missing from {from} to {to}, {place}{dst}")
}

/// `NON_BILLABLE`, `FUTURE` and `OUTSIDE_PERIOD` as runs; `NEGATIVE` and
/// `CAPACITY` per slot, with the value.
fn per_interval(series: &Series, rules: &Rules, out: &mut Vec<Finding>, evaluated: &mut Vec<Rule>) {
    if rules.negative {
        evaluated.push(Rule::Negative);
    }
    let capacity = rules
        .capacity_kw
        .filter(|_| series.unit().is_none_or(|u| u.is_energy()));
    if capacity.is_some() {
        evaluated.push(Rule::Capacity);
    }
    let p = rules.period;
    let mut grouped: Vec<(Rule, OffsetDateTime, OffsetDateTime)> = Vec::new();
    let mut push = |rule: Rule, iv: &MeterInterval| {
        if let Some(group) = grouped.iter_mut().rev().find(|(r, ..)| *r == rule)
            && group.2 == iv.from()
        {
            group.2 = iv.to();
            return;
        }
        grouped.push((rule, iv.from(), iv.to()));
    };
    for iv in series {
        if !iv.quality().is_billable() {
            push(Rule::NonBillable, iv);
        }
        if iv.to() > rules.now {
            push(Rule::Future, iv);
        }
        if iv.from() < p.start() || iv.to() > p.end() {
            push(Rule::OutsidePeriod, iv);
        }
        if rules.negative && iv.value() < Decimal::ZERO {
            out.push(
                Finding::new(
                    Rule::Negative,
                    iv.from(),
                    iv.to(),
                    format!("negative quantity {} at {}", iv.value(), iv.from()),
                )
                .data(Some(iv.value()), None),
            );
        }
        if let Some(cap) = capacity
            && let Some(kw) = iv.demand_kw()
            && kw > cap
        {
            out.push(
                Finding::new(
                    Rule::Capacity,
                    iv.from(),
                    iv.to(),
                    format!(
                        "average power {kw} kW at {} exceeds the capacity {cap} kW",
                        iv.from()
                    ),
                )
                .data(Some(iv.value()), Some(cap)),
            );
        }
    }
    for (rule, from, to) in grouped {
        let message = match rule {
            Rule::NonBillable => format!("non-billable quality from {from} to {to}"),
            Rule::Future => format!("slots from {from} to {to} end after now ({})", rules.now),
            _ => format!("data from {from} to {to} lies outside the declared period"),
        };
        out.push(Finding::new(rule, from, to, message));
    }
}

/// Maximal runs of adjacent intervals whose value `member(value, first)`
/// accepts; a run that `report(len, duration)` accepts is one finding.
fn runs(
    series: &Series,
    member: impl Fn(Decimal, Decimal) -> bool,
    report: impl Fn(usize, Duration) -> bool,
    rule: Rule,
    out: &mut Vec<Finding>,
) {
    let mut run: Option<(OffsetDateTime, OffsetDateTime, Decimal, usize)> = None;
    let mut close = |run: Option<(OffsetDateTime, OffsetDateTime, Decimal, usize)>| {
        if let Some((from, to, first, len)) = run
            && report(len, to - from)
        {
            let what = if rule == Rule::ZeroRun {
                "zero"
            } else {
                "an unchanged value"
            };
            out.push(
                Finding::new(
                    rule,
                    from,
                    to,
                    format!("{len} adjacent slots of {what} from {from} to {to}"),
                )
                .data(Some(first), None),
            );
        }
    };
    for iv in series {
        run = match run {
            Some((from, to, first, len)) if to == iv.from() && member(iv.value(), first) => {
                Some((from, iv.to(), first, len + 1))
            }
            previous => {
                close(previous);
                member(iv.value(), iv.value()).then(|| (iv.from(), iv.to(), iv.value(), 1))
            }
        };
    }
    close(run);
}

/// `SPIKE`: the Hampel test over `half_window` billable neighbours either side.
/// `false` when off or when the series is shorter than one window.
fn spikes(series: &Series, rules: &Rules, out: &mut Vec<Finding>) -> bool {
    let Some(k) = rules.spike.filter(|k| *k > 0) else {
        return false;
    };
    let billable: Vec<&MeterInterval> = series
        .iter()
        .filter(|iv| iv.quality().is_billable())
        .collect();
    if billable.len() <= 2 * k {
        return false;
    }
    let values: Vec<Decimal> = billable.iter().map(|iv| iv.value()).collect();
    for (i, iv) in billable.iter().enumerate() {
        let mut window = values[i.saturating_sub(k)..(i + k + 1).min(values.len())].to_vec();
        if let Some(median) = hampel(iv.value(), &mut window, rules.threshold, rules.floor) {
            out.push(
                Finding::new(
                    Rule::Spike,
                    iv.from(),
                    iv.to(),
                    format!(
                        "{} at {} is far from its neighbours' median {median}",
                        iv.value(),
                        iv.from()
                    ),
                )
                .data(Some(iv.value()), Some(median)),
            );
        }
    }
    true
}

/// `SEASONAL`: the Hampel test against the same slot on comparable days.
/// `true` when at least one value had a full set of references.
fn seasonal(series: &Series, rules: &Rules, out: &mut Vec<Finding>) -> bool {
    let Some((land, days)) = rules.seasonal.filter(|(_, d)| *d > 0) else {
        return false;
    };
    let data = Data {
        series,
        prior: rules.history.as_ref(),
    };
    let mut ran = false;
    for iv in series.iter().filter(|iv| iv.quality().is_billable()) {
        let refs = references(&data, iv.from(), iv.from(), land, days, |r| {
            is_actual(r.quality())
        });
        if refs.len() < days {
            continue;
        }
        ran = true;
        let mut window: Vec<Decimal> = refs.iter().map(|(_, v)| *v).collect();
        if let Some(median) = hampel(iv.value(), &mut window, rules.threshold, rules.floor) {
            out.push(
                Finding::new(
                    Rule::Seasonal,
                    iv.from(),
                    iv.to(),
                    format!(
                        "{} at {} is far from the median {median} of the same slot on {days} comparable days",
                        iv.value(),
                        iv.from()
                    ),
                )
                .data(Some(iv.value()), Some(median)),
            );
        }
    }
    ran
}

/// `LAENGSVERGLEICH`. Evaluated only when every slot of the reading interval
/// is present and billable — a gap explains a shortfall and is its own finding.
fn laengsvergleich(series: &Series, rules: &Rules, out: &mut Vec<Finding>) -> bool {
    let Some((start, end, tolerance)) = &rules.laengsvergleich else {
        return false;
    };
    let (from, to) = (start.at, end.at);
    if to <= from {
        return false;
    }
    let mut sum = Decimal::ZERO;
    let mut last_end = from;
    for slot in slots(series.boundary(), series.resolution(), from, to) {
        let Some(iv) = series.get(slot.start()) else {
            return false;
        };
        if slot.start() < from || !iv.quality().is_billable() {
            return false;
        }
        let Some(s) = sum.checked_add(iv.value()) else {
            return false;
        };
        sum = s;
        last_end = slot.end();
    }
    if last_end != to {
        return false;
    }
    let Some(advance) = end.value.checked_sub(start.value) else {
        return false;
    };
    let Some(deviation) = sum.checked_sub(advance).map(|d| d.abs()) else {
        return false;
    };
    let allowed = advance
        .abs()
        .checked_mul(*tolerance)
        .unwrap_or(Decimal::MAX);
    if deviation > allowed {
        out.push(
            Finding::new(
                Rule::Laengsvergleich,
                from,
                to,
                format!(
                    "the load curve sums to {sum} from {from} to {to}, the register advanced by \
                     {advance}; the tolerance is {tolerance} of the advance"
                ),
            )
            .data(Some(sum), Some(advance)),
        );
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::series::interval::QualityFlag;
    use crate::time::calendar::DayBoundary;
    use crate::time::resolution::Resolution;
    use time::macros::{date, datetime};

    const NOW: OffsetDateTime = datetime!(2027-01-01 0:00 UTC);

    fn day(d: time::Date) -> Period {
        DayBoundary::Strom.day(d).unwrap()
    }

    /// Quarter-hours from `start`; `None` is a missing slot.
    fn series(start: OffsetDateTime, values: &[Option<Decimal>]) -> Series {
        let ivs = values
            .iter()
            .enumerate()
            .filter_map(|(i, v)| {
                let from = start + Duration::minutes(15 * i as i64);
                v.map(|v| MeterInterval::quarter_hour(from, v, QualityFlag::Measured).unwrap())
            })
            .collect();
        Series::new(Resolution::QUARTER_HOUR, DayBoundary::Strom, ivs).unwrap()
    }

    fn noisy(n: usize) -> Vec<Option<Decimal>> {
        (0..n)
            .map(|i| Some(Decimal::from(10 + (i * 7) % 5)))
            .collect()
    }

    #[test]
    fn a_clean_series_grades_a_and_the_grade_follows_the_findings() {
        let d = day(date!(2026 - 06 - 03));
        let report = validate(&series(d.start(), &noisy(96)), &Rules::strom(d, NOW, None));
        assert!(report.is_clean(), "{:?}", report.findings);
        assert_eq!(report.grade(), Grade::A);
        assert_eq!(Grade::of([]), Grade::A);
        assert_eq!(
            report.skipped(),
            vec![Rule::Capacity, Rule::Laengsvergleich, Rule::Seasonal]
        );
        assert_eq!(report.coverage.pct(), Some(dec!(100)));
    }

    #[test]
    fn head_tail_and_interior_gaps_are_one_finding_each() {
        let d = day(date!(2026 - 06 - 03));
        let mut v = noisy(96);
        for i in [0, 1, 50, 95] {
            v[i] = None;
        }
        let report = validate(&series(d.start(), &v), &Rules::strom(d, NOW, None));
        let gaps: Vec<&Finding> = report.by_rule(Rule::Gap).collect();
        assert_eq!(gaps.len(), 3);
        assert!(gaps[0].message.contains("before the first"));
        assert!(gaps[1].message.contains("between"));
        assert!(gaps[2].message.contains("after the last"));
        assert_eq!(gaps[0].reference, Some(dec!(2)));
        assert_eq!(report.grade(), Grade::C);
        assert_eq!(report.coverage.pct(), Some(dec!(95.83)));
    }

    #[test]
    fn a_gap_in_the_repeated_hour_says_so() {
        let d = day(date!(2026 - 10 - 25));
        let mut v = noisy(100);
        v[12] = None; // 01:00 UTC = the second 02:00 local
        let report = validate(&series(d.start(), &v), &Rules::strom(d, NOW, None));
        let gap = report.by_rule(Rule::Gap).next().unwrap();
        assert!(gap.message.contains("repeated hour"), "{}", gap.message);
        v[12] = Some(dec!(1));
        v[60] = None;
        let report = validate(&series(d.start(), &v), &Rules::strom(d, NOW, None));
        assert!(
            !report
                .by_rule(Rule::Gap)
                .next()
                .unwrap()
                .message
                .contains("repeated")
        );
    }

    #[test]
    fn coverage_is_clipped_to_the_declared_period() {
        let d = day(date!(2026 - 06 - 03));
        let report = validate(&series(d.start(), &noisy(200)), &Rules::strom(d, NOW, None));
        assert!(report.coverage.is_complete());
        assert_eq!(report.coverage.pct(), Some(dec!(100)));
        assert_eq!(report.by_rule(Rule::OutsidePeriod).count(), 1);
        assert_eq!(report.grade(), Grade::B);
    }

    #[test]
    fn a_gap_ends_a_zero_run() {
        let d = day(date!(2026 - 06 - 03));
        let z = Some(Decimal::ZERO);
        let mut v = noisy(96);
        v[10..13].copy_from_slice(&[z, z, z]);
        v[13] = None;
        v[14..17].copy_from_slice(&[z, z, z]);
        let rules = Rules::strom(d, NOW, None).without_spike();
        assert_eq!(
            validate(&series(d.start(), &v), &rules)
                .by_rule(Rule::ZeroRun)
                .count(),
            0
        );
        v[13] = z;
        let report = validate(&series(d.start(), &v), &rules);
        let run = report.by_rule(Rule::ZeroRun).next().unwrap();
        assert_eq!(run.to - run.from, Duration::minutes(105));
    }

    #[test]
    fn a_frozen_non_zero_value_is_stale_and_zeros_are_not() {
        let d = day(date!(2026 - 06 - 03));
        let mut v = noisy(96);
        for slot in v.iter_mut().take(26).skip(20) {
            *slot = Some(dec!(3.3));
        }
        let rules = Rules::strom(d, NOW, None).without_spike();
        let report = validate(&series(d.start(), &v), &rules);
        assert_eq!(report.by_rule(Rule::Stale).count(), 1);
        v[25] = Some(dec!(3.4));
        let report = validate(&series(d.start(), &v), &rules);
        assert_eq!(report.by_rule(Rule::Stale).count(), 0);
        let zeros = vec![Some(Decimal::ZERO); 96];
        assert_eq!(
            validate(&series(d.start(), &zeros), &rules)
                .by_rule(Rule::Stale)
                .count(),
            0
        );
    }

    #[test]
    fn spikes_use_a_mad_floor_that_is_positive_for_strom() {
        let d = day(date!(2026 - 06 - 03));
        let mut v = vec![Some(dec!(1)); 96];
        v[40] = Some(dec!(1.001));
        let rules = Rules::strom(d, NOW, None).without_stale();
        assert_eq!(
            validate(&series(d.start(), &v), &rules)
                .by_rule(Rule::Spike)
                .count(),
            0
        );
        let bare = rules.clone().floor(Decimal::ZERO);
        assert_eq!(
            validate(&series(d.start(), &v), &bare)
                .by_rule(Rule::Spike)
                .count(),
            1
        );
        v[40] = Some(dec!(9));
        let report = validate(&series(d.start(), &v), &rules);
        let spike = report.by_rule(Rule::Spike).next().unwrap();
        assert_eq!(
            (spike.value, spike.reference),
            (Some(dec!(9)), Some(dec!(1)))
        );
        // Too short for one window: not evaluated, and the report says so.
        let short = validate(&series(d.start(), &v[..20]), &rules);
        assert!(short.skipped().contains(&Rule::Spike));
    }

    #[test]
    fn an_alternating_two_level_load_is_not_a_spike_but_a_tenfold_value_is() {
        let d = day(date!(2026 - 06 - 03));
        let mut v: Vec<Option<Decimal>> = (0..96)
            .map(|i| Some(if i % 2 == 0 { dec!(1) } else { dec!(2) }))
            .collect();
        let rules = Rules::strom(d, NOW, None).without_stale();
        assert_eq!(
            validate(&series(d.start(), &v), &rules)
                .by_rule(Rule::Spike)
                .count(),
            0
        );
        v[41] = Some(dec!(20));
        let report = validate(&series(d.start(), &v), &rules);
        let spikes: Vec<_> = report.by_rule(Rule::Spike).collect();
        assert_eq!(spikes.len(), 1, "{spikes:?}");
        assert_eq!(spikes[0].value, Some(dec!(20)));
    }

    #[test]
    fn seasonal_compares_with_the_same_slot_on_comparable_days() {
        let start = day(date!(2026 - 06 - 01)).start();
        let target = day(date!(2026 - 06 - 24));
        let days = (target.start() - start).whole_days() as usize;
        let mut v = vec![Some(dec!(2)); 96 * (days + 1) - 4];
        let i = 96 * days + 40;
        v[i] = Some(dec!(9));
        let rules = Rules::strom(target, NOW, None)
            .without_spike()
            .without_stale()
            .seasonal(Bundesland::Be, 4);
        let all = series(start, &v);
        let tail = Series::new(
            Resolution::QUARTER_HOUR,
            DayBoundary::Strom,
            all.iter()
                .filter(|iv| target.contains(iv.from()))
                .cloned()
                .collect(),
        )
        .unwrap();
        let history = Series::new(
            Resolution::QUARTER_HOUR,
            DayBoundary::Strom,
            all.iter()
                .filter(|iv| iv.from() < target.start())
                .cloned()
                .collect(),
        )
        .unwrap();
        let report = validate(&tail, &rules.clone().history(history));
        let f: Vec<_> = report.by_rule(Rule::Seasonal).collect();
        assert_eq!(f.len(), 1, "{f:?}");
        assert_eq!(f[0].reference, Some(dec!(2)));
        assert!(validate(&tail, &rules).skipped().contains(&Rule::Seasonal));
    }

    #[test]
    fn rejected_values_are_errors_and_the_future_blocks() {
        let d = day(date!(2026 - 06 - 03));
        let mut v = noisy(96);
        v[3] = Some(dec!(-1));
        v[7] = Some(dec!(40)); // 160 kW
        let mut s = series(d.start(), &v).into_intervals();
        s[9] = s[9].clone().with_quality(QualityFlag::Faulty);
        let s = Series::new(Resolution::QUARTER_HOUR, DayBoundary::Strom, s).unwrap();
        let rules = Rules::strom(d, NOW, Some(dec!(100))).without_spike();
        let report = validate(&s, &rules);
        assert_eq!(report.by_rule(Rule::Negative).count(), 1);
        assert_eq!(
            report.by_rule(Rule::Capacity).next().unwrap().reference,
            Some(dec!(100))
        );
        assert_eq!(report.by_rule(Rule::NonBillable).count(), 1);
        assert_eq!(report.grade(), Grade::C);
        assert!(report.evaluated.contains(&Rule::Capacity));

        let early = Rules::strom(d, d.start() + Duration::hours(12), None);
        let report = validate(&s, &early);
        let future = report.by_rule(Rule::Future).next().unwrap();
        assert_eq!(
            (future.from, future.to),
            (d.start() + Duration::hours(12), d.end())
        );
        assert_eq!(report.grade(), Grade::F);
    }

    #[test]
    fn the_laengsvergleich_compares_the_sum_with_the_register_advance() {
        let d = day(date!(2026 - 06 - 03));
        let s = series(d.start(), &vec![Some(dec!(0.25)); 96]);
        let reading = |at, v| MeterReading::measured(at, v);
        let check = |advance: Decimal| {
            let rules = Rules::strom(d, NOW, None).laengsvergleich(
                reading(d.start(), dec!(1000)),
                reading(d.end(), dec!(1000) + advance),
                dec!(0.007),
            );
            validate(&s, &rules)
        };
        // 24 against 24.1 is 0.41 % — inside a 0.7 % tolerance.
        assert!(
            check(dec!(24.1))
                .by_rule(Rule::Laengsvergleich)
                .next()
                .is_none()
        );
        let off = check(dec!(25));
        let f = off.by_rule(Rule::Laengsvergleich).next().unwrap();
        assert_eq!((f.value, f.reference), (Some(dec!(24.00)), Some(dec!(25))));
        assert_eq!(off.grade(), Grade::F);
        // A gap inside the interval explains a shortfall: not evaluated.
        let mut v = vec![Some(dec!(0.25)); 96];
        v[5] = None;
        let rules = Rules::strom(d, NOW, None).laengsvergleich(
            reading(d.start(), dec!(0)),
            reading(d.end(), dec!(24)),
            dec!(0.007),
        );
        assert!(
            validate(&series(d.start(), &v), &rules)
                .skipped()
                .contains(&Rule::Laengsvergleich)
        );
    }
}
