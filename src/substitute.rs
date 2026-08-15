//! Ersatzwertbildung — substitute values for missing or rejected readings.
//!
//! ## Legal basis
//!
//! - **§ 60 Abs. 1 MsbG** places the duty on the Messstellenbetreiber: the data
//!   collected under §§ 55–59 must be *aufbereitet* and transmitted to the
//!   berechtigte Stellen.
//! - **§ 60 Abs. 2 MsbG** names what that preparation includes and where it
//!   should happen, verbatim: *"Bei Messstellen mit intelligenten Messsystemen
//!   sollen die Aufbereitung der Messwerte, insbesondere die Plausibilisierung
//!   und die Ersatzwertbildung im Smart-Meter-Gateway, und die Datenübermittlung
//!   über das Smart-Meter-Gateway direkt an die berechtigten Stellen erfolgen…"*
//!
//!   Note what that sentence does **not** contain: any procedure. It says
//!   Ersatzwertbildung is owed and where it belongs; it prescribes no method,
//!   no reference period and no ranking between them.
//! - **BNetzA Festlegungen** — the current consolidated MaKo Lesefassungen are
//!   **BK6-24-174** (GPKE / WiM / MaBiS, in force 6 June 2025) — carry the
//!   process rules, and **VDE-AR-N 4400 (Metering Code)** the technical ones.
//!
//! ## Why the methods are configuration, not constants
//!
//! VDE-AR-N 4400 is a paywalled VDE Anwendungsregel, so its text cannot be
//! reproduced or verified here. Every threshold this module uses is therefore a
//! parameter with a documented default rather than a hard-coded claim of
//! conformance: the operator's own metering-code settings win. What the module
//! guarantees is the arithmetic and the audit trail, not that a particular
//! default matches a document neither the author nor the reader can cite.
//!
//! | This crate | Corresponds to | Configurable |
//! |---|---|---|
//! | [`SubstituteMethod::LinearInterpolation`] | interpolation across a short gap | [`FillGapsConfig::short_gap_threshold`] |
//! | [`SubstituteMethod::PriorPeriodAverage`] | Vergleichstag: the same slot a week earlier | [`REFERENCE_PERIOD_DAYS`] |
//! | [`SubstituteMethod::LastValueCarryForward`] | Fortschreibung des letzten plausiblen Wertes | — |
//! | [`SubstituteMethod::ZeroFill`] | documented shutdown / confirmed zero delivery | — |
//!
//! ## The audit trail records what ran, not what was asked for
//!
//! A requested method can be impossible: a prior-period average with no
//! matching reference slot, an interpolation with nothing after the gap to
//! interpolate towards. Every such case falls back, and
//! [`SubstituteEntry::method`] reports the method **that actually produced the
//! value**. Recording the request instead would put a claim in the audit trail
//! that the number does not support.
//!
//! ## Retention
//!
//! § 60 Abs. 6 MsbG is a **deletion** obligation, not a retention mandate:
//! personenbezogene Messwerte must be erased or anonymised as soon as they are
//! no longer needed, *"spätestens jedoch nach drei Jahren ab dem Schluss des
//! Kalenderjahres, in dem der jeweilige Messwert erhoben wurde"*. Substitute
//! values are Messwerte for this purpose. A system that keeps them for three
//! years *because the law says so* has read the provision backwards.

use std::collections::BTreeMap;

use rust_decimal::Decimal;
use time::{Duration, OffsetDateTime};
use time_tz::{OffsetDateTimeExt as _, timezones};

use crate::interval::{MeterInterval, QualityFlag};
use crate::resolution::IntervalResolution;

/// Length of the reference period used by
/// [`SubstituteMethod::PriorPeriodAverage`]: the seven **Berlin calendar
/// days** immediately preceding the gap.
///
/// Calendar days, not `7 × 24` hours: the matching slot one week earlier is
/// 169 UTC hours back across the autumn fall-back
/// ([`crate::calendar::shift_back_days`]), and a fixed-duration window would
/// exclude it.
pub const REFERENCE_PERIOD_DAYS: i64 = 7;

// ── SubstituteMethod ──────────────────────────────────────────────────────────

/// How a substitute value was produced.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "SCREAMING_SNAKE_CASE"))]
pub enum SubstituteMethod {
    /// Linear interpolation between the plausible values bracketing the gap.
    ///
    /// The best answer for a short outage, and meaningless for a long one: a
    /// straight line across a week says nothing about Tuesday.
    #[default]
    LinearInterpolation,

    /// Mean of the same time slot over the preceding
    /// [`REFERENCE_PERIOD_DAYS`], matched on (weekday, hour, minute) in German
    /// local time.
    ///
    /// Matching on time of day alone would average a Sunday gap over five
    /// working days; matching in UTC would shift every slot by an hour across
    /// a DST boundary.
    PriorPeriodAverage,

    /// Zero — an affirmatively documented absence of delivery.
    ///
    /// Never a fallback for "no data": that is what the other three are for.
    /// The one exception is a gap with no usable reference of any kind, where
    /// zero is the only value left and the entry says so.
    ZeroFill,

    /// The last plausible value, carried forward.
    LastValueCarryForward,
}

impl SubstituteMethod {
    /// Every method, in declaration order.
    pub const ALL: [Self; 4] = [
        Self::LinearInterpolation,
        Self::PriorPeriodAverage,
        Self::ZeroFill,
        Self::LastValueCarryForward,
    ];

    /// German description, for an audit record or an invoice annex.
    #[must_use]
    pub const fn description(self) -> &'static str {
        match self {
            Self::LinearInterpolation => "Lineare Interpolation zwischen den Randwerten",
            Self::PriorPeriodAverage => "Vorperiodenmittelwert, gleicher Zeitschlitz",
            Self::ZeroFill => "Nullwert (dokumentierter Lieferstopp)",
            Self::LastValueCarryForward => "Letzter plausibler Wert fortgeschrieben",
        }
    }
}

// ── SubstitutionReason ────────────────────────────────────────────────────────

/// Why a substitute value was needed.
///
/// Distinct from [`SubstituteMethod`], which says how one was produced. The
/// reason is an input the caller knows and the method is an output this module
/// determines.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "SCREAMING_SNAKE_CASE"))]
pub enum SubstitutionReason {
    /// No measurement arrived for the interval.
    #[default]
    NoMeasurementAvailable,
    /// Meter hardware failure.
    MeterFault,
    /// The Smart-Meter-Gateway was unreachable.
    GatewayCommFailure,
    /// A plausibility check rejected the delivered value.
    PlausibilityCheckFailed,
    /// Manual correction by the MSB or an operator.
    ManualCorrection,
    /// A meter exchange; the value spans the replacement boundary.
    MeterExchangeInterpolation,
    /// Another documented reason.
    Other,
}

impl SubstitutionReason {
    /// Every reason, in declaration order.
    pub const ALL: [Self; 7] = [
        Self::NoMeasurementAvailable,
        Self::MeterFault,
        Self::GatewayCommFailure,
        Self::PlausibilityCheckFailed,
        Self::ManualCorrection,
        Self::MeterExchangeInterpolation,
        Self::Other,
    ];

    /// German description, for an audit record.
    #[must_use]
    pub const fn description(self) -> &'static str {
        match self {
            Self::NoMeasurementAvailable => "Kein Messwert verfügbar",
            Self::MeterFault => "Zählerdefekt",
            Self::GatewayCommFailure => "SMGW-Kommunikationsfehler",
            Self::PlausibilityCheckFailed => "Plausibilitätsprüfung fehlgeschlagen",
            Self::ManualCorrection => "Manuelle Korrektur durch MSB/Betreiber",
            Self::MeterExchangeInterpolation => "Zählerwechsel — Interpolation über Wechselgrenze",
            Self::Other => "Sonstiger dokumentierter Grund",
        }
    }
}

// ── SubstituteEntry ───────────────────────────────────────────────────────────

/// One generated substitute value, with the provenance to explain it.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct SubstituteEntry {
    /// The synthesised interval. Always carries [`QualityFlag::Substituted`].
    pub interval: MeterInterval,
    /// The method that **actually produced** this value — see the
    /// [module docs](self#the-audit-trail-records-what-ran-not-what-was-asked-for).
    pub method: SubstituteMethod,
    /// Why a substitute was needed at all.
    pub reason: SubstitutionReason,
    /// How many measured values the substitute was derived from.
    ///
    /// Two for an interpolation, one for a carry-forward, the sample count for
    /// a prior-period average, and zero for a value with no evidence behind it.
    pub reference_count: u32,
}

// ── FillGapsConfig ────────────────────────────────────────────────────────────

/// Configuration for [`fill_gaps`]: the grid, the period, and the policy.
///
/// There is no `Default`. The grid resolution and the period are the two
/// things a gap fill cannot proceed without and the two most easily got wrong —
/// they were loose positional arguments until 0.17, where a caller could pass
/// `900` for a daily series or transpose `from` and `to` without the type
/// system noticing. They are constructor arguments now.
#[derive(Debug, Clone, PartialEq)]
pub struct FillGapsConfig {
    /// The grid to fill against.
    ///
    /// An [`IntervalResolution`], not a second count, so a daily, monthly or
    /// yearly fill walks **Europe/Berlin calendar periods**. Stepping a fixed
    /// 86 400 s drifts by an hour at each DST transition and never recovers.
    pub resolution: IntervalResolution,

    /// The half-open UTC period to fill, `[from, to)`.
    ///
    /// Leading and trailing gaps are filled too, so this decides how much
    /// series there is to complete — not just what to patch between the values
    /// that happen to have arrived.
    pub period: (OffsetDateTime, OffsetDateTime),

    /// The method to apply to gaps longer than
    /// [`short_gap_threshold`](Self::short_gap_threshold).
    pub method: SubstituteMethod,

    /// Reference intervals for [`SubstituteMethod::PriorPeriodAverage`].
    ///
    /// Only those falling in the [`REFERENCE_PERIOD_DAYS`] before each gap are
    /// used; the window is applied here rather than trusted from the caller,
    /// because averaging a longer history silently produces a multi-week mean
    /// that nothing in the output would reveal.
    pub prior_period_intervals: Vec<MeterInterval>,

    /// Gaps of at most this many intervals are interpolated whatever
    /// [`method`](Self::method) says.
    ///
    /// Default: `3`. Set to `0` to apply the configured method uniformly.
    pub short_gap_threshold: usize,

    /// Recorded on every generated entry.
    pub reason: SubstitutionReason,
}

impl FillGapsConfig {
    /// Fill `[from, to)` on a `resolution` grid, interpolating short gaps and
    /// carrying longer ones forward.
    #[must_use]
    pub fn new(resolution: IntervalResolution, from: OffsetDateTime, to: OffsetDateTime) -> Self {
        Self {
            resolution,
            period: (from, to),
            method: SubstituteMethod::default(),
            prior_period_intervals: Vec::new(),
            short_gap_threshold: 3,
            reason: SubstitutionReason::NoMeasurementAvailable,
        }
    }

    /// Apply `method` to gaps longer than the short-gap threshold.
    #[must_use]
    pub fn with_method(mut self, method: SubstituteMethod) -> Self {
        self.method = method;
        self
    }

    /// Prior-period averaging against the supplied reference data.
    #[must_use]
    pub fn prior_period(mut self, prior_period_intervals: Vec<MeterInterval>) -> Self {
        self.method = SubstituteMethod::PriorPeriodAverage;
        self.prior_period_intervals = prior_period_intervals;
        self
    }

    /// Affirmatively documented zero delivery.
    ///
    /// Also sets `short_gap_threshold` to 0: a documented shutdown is zero for
    /// its whole duration, including the first three intervals.
    #[must_use]
    pub fn zero_fill(mut self) -> Self {
        self.method = SubstituteMethod::ZeroFill;
        self.short_gap_threshold = 0;
        self
    }

    /// Gaps of at most `n` intervals are interpolated whatever
    /// [`method`](Self::method) says. Set `0` to apply the method uniformly.
    #[must_use]
    pub fn short_gap_threshold(mut self, n: usize) -> Self {
        self.short_gap_threshold = n;
        self
    }

    /// Record `reason` on every generated entry.
    #[must_use]
    pub fn because(mut self, reason: SubstitutionReason) -> Self {
        self.reason = reason;
        self
    }
}

// ── FilledSeries ──────────────────────────────────────────────────────────────

/// The result of a gap fill: a complete series plus the audit trail for the
/// values that were invented.
#[derive(Debug, Clone, PartialEq)]
pub struct FilledSeries {
    /// Every interval of the grid, measured and substituted alike, ascending.
    pub intervals: Vec<MeterInterval>,
    /// One entry per synthesised value, ascending.
    pub substitutions: Vec<SubstituteEntry>,
}

impl FilledSeries {
    /// Number of values that had to be invented.
    #[must_use]
    pub fn substituted_count(&self) -> usize {
        self.substitutions.len()
    }

    /// Share of the series that is measured rather than substituted, 0–100.
    #[must_use]
    pub fn measured_pct(&self) -> f64 {
        if self.intervals.is_empty() {
            return 0.0;
        }
        let measured = self.intervals.len() - self.substitutions.len();
        measured as f64 / self.intervals.len() as f64 * 100.0
    }
}

// ── fill_gaps ─────────────────────────────────────────────────────────────────

/// Fill the gaps in a series, returning the completed series **and** the audit
/// trail.
///
/// Gaps of at most [`FillGapsConfig::short_gap_threshold`] intervals are
/// interpolated regardless of the configured method; longer ones use it. The
/// gap length is measured once, at the gap's first missing slot — measuring it
/// from a moving cursor shrinks it as the gap fills, which silently reverted
/// the last few intervals of every long gap to interpolation.
///
/// A **present but non-billable** slot (`Faulty`, `Unknown`) is never
/// overwritten — this function fills *missing* slots — but it does not anchor
/// an interpolation either: the straight line runs between the billable
/// values either side, each at the grid slot it actually occupies, so the
/// missing slots around a faulty reading land on one consistent line.
///
/// Leading and trailing gaps are filled too, and are the cases most likely to
/// have no bracketing value: the entries record the fallback that ran.
///
/// An interval is matched to a slot by its `from` timestamp, so the period
/// should start on a boundary of the chosen resolution — a daily fill starting
/// at 09:00 produces 09:00-to-09:00 windows, which are not Liefertage.
///
/// ## Example
///
/// ```rust
/// use metering::{FillGapsConfig, IntervalResolution, MeterInterval, QualityFlag, fill_gaps};
/// use rust_decimal::dec;
/// use time::macros::datetime;
///
/// let measured = vec![
///     MeterInterval {
///         from:      datetime!(2026-01-01 0:00 UTC),
///         to:        datetime!(2026-01-01 0:15 UTC),
///         value:     dec!(2.0),
///         quality:   QualityFlag::Measured,
///         obis_code: None,
///     },
///     MeterInterval {
///         from:      datetime!(2026-01-01 0:30 UTC),
///         to:        datetime!(2026-01-01 0:45 UTC),
///         value:     dec!(2.4),
///         quality:   QualityFlag::Measured,
///         obis_code: None,
///     },
/// ];
///
/// let filled = fill_gaps(
///     &measured,
///     &FillGapsConfig::new(
///         IntervalResolution::QuarterHour,
///         datetime!(2026-01-01 0:00 UTC),
///         datetime!(2026-01-01 0:45 UTC),
///     ),
/// );
///
/// assert_eq!(filled.intervals.len(), 3);
/// assert_eq!(filled.intervals[1].quality, QualityFlag::Substituted);
/// // Halfway between 2.0 and 2.4.
/// assert_eq!(filled.intervals[1].value, dec!(2.2));
/// assert_eq!(filled.substituted_count(), 1);
/// ```
#[must_use]
pub fn fill_gaps(intervals: &[MeterInterval], config: &FillGapsConfig) -> FilledSeries {
    let (from, to) = config.period;
    let resolution = config.resolution;

    let sorted_input = || {
        let mut intervals = intervals.to_vec();
        intervals.sort_by_key(|iv| iv.from);
        FilledSeries {
            intervals,
            substitutions: Vec::new(),
        }
    };

    // A resolution with neither a fixed length nor a calendar meaning —
    // `Custom(0)` — cannot describe a grid. Hand the input back rather than
    // discarding it: the caller's parameters are wrong, their data is not.
    if !resolution.is_fixed() && !resolution.is_calendar() {
        return sorted_input();
    }
    // An empty or inverted range has no slots at all, so there is nothing to
    // return — not even the input, which lies outside the requested range.
    if to <= from {
        return FilledSeries {
            intervals: Vec::new(),
            substitutions: Vec::new(),
        };
    }

    // Grid slot → measured interval. A BTreeMap rather than a HashMap because
    // the gap walk below needs ordered lookahead for the closing value.
    let measured: BTreeMap<i64, &MeterInterval> = intervals
        .iter()
        .map(|iv| (iv.from.unix_timestamp(), iv))
        .collect();

    let reference = PriorPeriodIndex::build(&config.prior_period_intervals);

    let mut out: Vec<MeterInterval> = Vec::new();
    let mut substitutions: Vec<SubstituteEntry> = Vec::new();
    // The last billable value seen — measured or substituted — with the grid
    // slot it sits on. The slot index is what lets interpolation across a
    // present-but-faulty slot use the value's true distance rather than
    // pretending it sits adjacent to the gap.
    let mut anchor: Option<(usize, Decimal)> = None;
    // Set at the first missing slot of a gap and cleared when it closes, so the
    // whole gap shares one length and one bracket.
    let mut gap: Option<Gap> = None;

    let obis = intervals.first().and_then(|iv| iv.obis_code);
    let mut cursor = from;
    let mut idx = 0usize;
    while cursor < to {
        let Some(next) = advance(cursor, resolution) else {
            break;
        };
        let ts = cursor.unix_timestamp();

        if let Some(&iv) = measured.get(&ts) {
            out.push(iv.clone());
            if iv.quality.is_billable() {
                anchor = Some((idx, iv.value));
            }
            gap = None;
            cursor = next;
            idx += 1;
            continue;
        }

        let current = match gap {
            Some(ref g) => g.clone(),
            None => {
                let g = Gap::measure(&measured, cursor, idx, resolution, to, anchor);
                gap = Some(g.clone());
                g
            }
        };

        let effective = if current.run_len <= config.short_gap_threshold {
            SubstituteMethod::LinearInterpolation
        } else {
            config.method
        };
        let (value, applied, reference_count) =
            current.synthesise(effective, idx, cursor, &reference, anchor.map(|(_, v)| v));

        let interval = MeterInterval {
            from: cursor,
            to: next,
            value,
            quality: QualityFlag::Substituted,
            obis_code: obis,
        };
        out.push(interval.clone());
        substitutions.push(SubstituteEntry {
            interval,
            method: applied,
            reason: config.reason,
            reference_count,
        });
        anchor = Some((idx, value));
        cursor = next;
        idx += 1;
    }

    FilledSeries {
        intervals: out,
        substitutions,
    }
}

/// The end of the grid slot starting at `cursor`.
///
/// Calendar resolutions resolve through [`crate::calendar`], so a `Day` step is
/// 23, 24 or 25 hours depending on the date. `None` when the resolution has no
/// grid or the arithmetic leaves the representable range.
fn advance(cursor: OffsetDateTime, resolution: IntervalResolution) -> Option<OffsetDateTime> {
    use crate::calendar;
    let next = match resolution {
        IntervalResolution::Day => calendar::day_end_utc(calendar::local_day(cursor)),
        IntervalResolution::Month => calendar::month_end_utc(calendar::local_day(cursor)),
        IntervalResolution::Year => calendar::year_end_utc(calendar::local_year(cursor)),
        fixed => cursor + Duration::seconds(i64::from(fixed.fixed_seconds()?)),
    };
    // A calendar step lands on the *end of the period containing* the cursor,
    // which is the cursor itself when it already sits on a boundary looking
    // backwards. Guard against a step that fails to advance, or the loop above
    // would never terminate.
    (next > cursor).then_some(next)
}

// ── gap resolution ────────────────────────────────────────────────────────────

/// One contiguous run of missing grid slots, measured once when it opens.
///
/// Interpolation anchors on the **billable** values either side, at their true
/// slot distances. The two are not the same thing as "the neighbouring
/// slots": a present-but-faulty slot terminates the missing run — it is never
/// overwritten — but the straight line must still run from the last billable
/// value to the next one, each at the slot it actually occupies. Measuring
/// the span to the nearest *present* slot while taking the endpoint value
/// from the nearest *billable* one placed every interior value at the wrong
/// fraction whenever the two differed.
#[derive(Debug, Clone)]
struct Gap {
    /// Number of contiguous missing slots — what is actually being invented,
    /// and the length the short-gap threshold is compared against.
    run_len: usize,
    /// The last billable value before the run, with its grid slot index.
    preceding: Option<(usize, Decimal)>,
    /// The first billable value at or after the run's end, with its index.
    following: Option<(usize, Decimal)>,
}

impl Gap {
    fn measure(
        measured: &BTreeMap<i64, &MeterInterval>,
        start: OffsetDateTime,
        start_idx: usize,
        resolution: IntervalResolution,
        end: OffsetDateTime,
        preceding: Option<(usize, Decimal)>,
    ) -> Self {
        // The contiguous missing run, bounded by the fill period.
        let mut run_len = 0usize;
        let mut cursor = start;
        while cursor < end && !measured.contains_key(&cursor.unix_timestamp()) {
            run_len += 1;
            let Some(next) = advance(cursor, resolution) else {
                break;
            };
            cursor = next;
        }

        // The first billable value at or after the run's end. `range` is the
        // reason this is a BTreeMap: the closing value may sit several slots
        // beyond — behind faulty slots, or beyond the period on a sparse
        // series — and quality-blind adjacency is not the closing anchor.
        let closing = measured
            .range(cursor.unix_timestamp()..)
            .map(|(_, iv)| *iv)
            .find(|iv| iv.quality.is_billable());

        // ...and the grid slot it occupies, so the interpolation fraction uses
        // its real distance. The walk is strictly monotonic and bounded by the
        // closing timestamp; an off-grid closing value is assigned the first
        // slot at or after it.
        let following = closing.and_then(|iv| {
            let target = iv.from.unix_timestamp();
            let mut walk = cursor;
            let mut walk_idx = start_idx + run_len;
            while walk.unix_timestamp() < target {
                walk = advance(walk, resolution)?;
                walk_idx += 1;
            }
            Some((walk_idx, iv.value))
        });

        Self {
            run_len,
            preceding,
            following,
        }
    }

    /// The substitute value, the method that produced it, and how many measured
    /// values it rests on. `idx` is the grid slot being filled.
    fn synthesise(
        &self,
        requested: SubstituteMethod,
        idx: usize,
        cursor: OffsetDateTime,
        reference: &PriorPeriodIndex,
        last_value: Option<Decimal>,
    ) -> (Decimal, SubstituteMethod, u32) {
        use SubstituteMethod as M;
        let preceding = self.preceding.map(|(_, v)| v);
        let following = self.following.map(|(_, v)| v);
        match requested {
            M::ZeroFill => (Decimal::ZERO, M::ZeroFill, 0),

            M::LastValueCarryForward => match last_value.or(preceding).or(following) {
                Some(v) => (v, M::LastValueCarryForward, 1),
                None => (Decimal::ZERO, M::ZeroFill, 0),
            },

            M::PriorPeriodAverage => match reference.average_for(cursor) {
                Some((avg, n)) => (avg, M::PriorPeriodAverage, n),
                None => match last_value.or(preceding).or(following) {
                    Some(v) => (v, M::LastValueCarryForward, 1),
                    None => (Decimal::ZERO, M::ZeroFill, 0),
                },
            },

            M::LinearInterpolation => match (self.preceding, self.following) {
                // The line runs from the preceding billable anchor to the
                // following one, and this slot sits `idx − pi` steps along a
                // span of `fi − pi` — its *true* offsets, which differ from
                // the naive run-relative fractions whenever a faulty slot
                // borders the run. Every missing slot between the same two
                // anchors lands on the same straight line, however the runs
                // between them are partitioned.
                // `u64`, not `u32`: a `usize` narrowed to `u32` truncates
                // silently, and a truncated denominator is a wrong value
                // rather than a failure.
                (Some((pi, p)), Some((fi, f))) if pi < idx && idx < fi => {
                    let denom = Decimal::from((fi - pi) as u64);
                    let numer = Decimal::from((idx - pi) as u64);
                    (p + (f - p) * numer / denom, M::LinearInterpolation, 2)
                }
                (Some((_, p)), None) | (Some((_, p)), Some(_)) => (p, M::LastValueCarryForward, 1),
                (None, Some((_, f))) => (f, M::LastValueCarryForward, 1),
                (None, None) => (Decimal::ZERO, M::ZeroFill, 0),
            },
        }
    }
}

// ── prior-period reference ────────────────────────────────────────────────────

/// Reference values indexed by (weekday, hour, minute) in German local time.
struct PriorPeriodIndex {
    slots: BTreeMap<(u8, u8, u8), Vec<(OffsetDateTime, Decimal)>>,
}

impl PriorPeriodIndex {
    fn build(intervals: &[MeterInterval]) -> Self {
        let mut slots: BTreeMap<(u8, u8, u8), Vec<(OffsetDateTime, Decimal)>> = BTreeMap::new();
        for iv in intervals.iter().filter(|iv| iv.quality.is_billable()) {
            slots
                .entry(slot_key(iv.from))
                .or_default()
                .push((iv.from, iv.value));
        }
        Self { slots }
    }

    /// Mean of the matching slot over the [`REFERENCE_PERIOD_DAYS`] preceding
    /// `target`, and the number of samples it averaged.
    fn average_for(&self, target: OffsetDateTime) -> Option<(Decimal, u32)> {
        // Seven Berlin **calendar** days, not a fixed 168 hours. The slots are
        // matched on local (weekday, hour, minute), so the only candidate
        // inside a one-week window is the same local slot seven days earlier —
        // which is 169 UTC hours back across the autumn fall-back. A
        // `Duration::days(7)` window excluded exactly that sample, silently
        // degrading the configured method to carry-forward for the week after
        // every October transition.
        let window_start = crate::calendar::shift_back_days(target, REFERENCE_PERIOD_DAYS);
        let samples = self.slots.get(&slot_key(target))?;
        let matching: Vec<Decimal> = samples
            .iter()
            .filter(|(at, _)| *at >= window_start && *at < target)
            .map(|(_, v)| *v)
            .collect();
        let n = u32::try_from(matching.len()).ok()?;
        if n == 0 {
            return None;
        }
        Some((matching.iter().sum::<Decimal>() / Decimal::from(n), n))
    }
}

/// (weekday, hour, minute) in Europe/Berlin.
fn slot_key(ts: OffsetDateTime) -> (u8, u8, u8) {
    let local = ts.to_timezone(timezones::db::europe::BERLIN);
    (
        local.weekday().number_days_from_monday(),
        local.hour(),
        local.minute(),
    )
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal::dec;
    use time::macros::datetime;

    const BASE: OffsetDateTime = datetime!(2026-01-01 0:00 UTC);

    fn iv_at(from: OffsetDateTime, kwh: Decimal) -> MeterInterval {
        MeterInterval {
            from,
            to: from + Duration::minutes(15),
            value: kwh,
            quality: QualityFlag::Measured,
            obis_code: None,
        }
    }

    /// A measured quarter-hour `offset_min` after the fixture base.
    fn iv(offset_min: i64, kwh: Decimal) -> MeterInterval {
        iv_at(BASE + Duration::minutes(offset_min), kwh)
    }

    /// A quarter-hour grid over `[from_min, to_min)` minutes from the base.
    fn cfg(from_min: i64, to_min: i64) -> FillGapsConfig {
        FillGapsConfig::new(
            IntervalResolution::QuarterHour,
            BASE + Duration::minutes(from_min),
            BASE + Duration::minutes(to_min),
        )
    }

    // ── the clean case ───────────────────────────────────────────────────────

    #[test]
    fn a_clean_series_is_returned_untouched() {
        let intervals = vec![iv(0, dec!(2.0)), iv(15, dec!(2.1)), iv(30, dec!(2.2))];
        let filled = fill_gaps(&intervals, &cfg(0, 45));
        assert_eq!(filled.intervals.len(), 3);
        assert!(filled.substitutions.is_empty());
        assert!(
            filled
                .intervals
                .iter()
                .all(|iv| iv.quality == QualityFlag::Measured)
        );
        assert!((filled.measured_pct() - 100.0).abs() < 1e-9);
    }

    // ── interpolation ────────────────────────────────────────────────────────

    /// Three unknowns between 0 and 100 sit at the quarter points. The
    /// forecast-module version this replaced produced 0, 33.3 and 66.7 — the
    /// first substitute *equalled the last measured value* and the series never
    /// approached the closing one.
    #[test]
    fn interpolation_uses_interior_fractions() {
        let intervals = vec![iv(0, dec!(0)), iv(60, dec!(100))];
        let filled = fill_gaps(&intervals, &cfg(0, 75).short_gap_threshold(10));

        let values: Vec<Decimal> = filled.intervals.iter().map(|iv| iv.value).collect();
        assert_eq!(
            values,
            vec![dec!(0), dec!(25), dec!(50), dec!(75), dec!(100)]
        );

        for entry in &filled.substitutions {
            assert!(entry.interval.value > dec!(0) && entry.interval.value < dec!(100));
            assert_eq!(entry.method, SubstituteMethod::LinearInterpolation);
            assert_eq!(entry.reference_count, 2);
        }
    }

    #[test]
    fn a_single_gap_is_the_midpoint() {
        let filled = fill_gaps(&[iv(0, dec!(2.0)), iv(30, dec!(2.4))], &cfg(0, 45));
        assert_eq!(filled.intervals.len(), 3);
        assert_eq!(filled.intervals[1].value, dec!(2.2));
        assert_eq!(filled.intervals[1].quality, QualityFlag::Substituted);
    }

    #[test]
    fn interpolation_is_symmetric() {
        let rising = fill_gaps(
            &[iv(0, dec!(0)), iv(60, dec!(100))],
            &cfg(0, 75).short_gap_threshold(10),
        );
        let falling = fill_gaps(
            &[iv(0, dec!(100)), iv(60, dec!(0))],
            &cfg(0, 75).short_gap_threshold(10),
        );
        for (a, b) in rising.intervals.iter().zip(falling.intervals.iter()) {
            assert_eq!(a.value + b.value, dec!(100), "at {}", a.from);
        }
    }

    // ── method selection ─────────────────────────────────────────────────────

    /// A long gap must keep its configured method to the last interval. The gap
    /// length used to be re-measured from the moving cursor, so it shrank as
    /// the gap filled and the last `short_gap_threshold` intervals silently
    /// reverted to interpolation.
    #[test]
    fn a_long_gap_keeps_its_method_to_the_last_interval() {
        let filled = fill_gaps(
            &[iv(0, dec!(10)), iv(120, dec!(20))],
            &cfg(0, 135)
                .with_method(SubstituteMethod::ZeroFill)
                .short_gap_threshold(2),
        );
        assert_eq!(filled.substituted_count(), 7, "seven quarter-hours missing");
        for entry in &filled.substitutions {
            assert_eq!(
                entry.interval.value,
                dec!(0),
                "every slot of a 7-interval gap uses the configured ZeroFill, \
                 including the last two — got {} at {}",
                entry.interval.value,
                entry.interval.from
            );
            assert_eq!(entry.method, SubstituteMethod::ZeroFill);
        }
    }

    #[test]
    fn short_gaps_are_interpolated_whatever_the_method() {
        let filled = fill_gaps(
            &[iv(0, dec!(2.0)), iv(30, dec!(4.0))],
            &cfg(0, 45)
                .with_method(SubstituteMethod::ZeroFill)
                .short_gap_threshold(3),
        );
        assert_eq!(filled.intervals[1].value, dec!(3.0));
        assert_eq!(
            filled.substitutions[0].method,
            SubstituteMethod::LinearInterpolation
        );
    }

    #[test]
    fn zero_fill_applies_from_the_first_interval() {
        let filled = fill_gaps(
            &[iv(0, dec!(2.0)), iv(30, dec!(2.0))],
            &cfg(0, 45).zero_fill(),
        );
        assert_eq!(filled.intervals[1].value, dec!(0));
        assert_eq!(filled.substitutions[0].method, SubstituteMethod::ZeroFill);
    }

    #[test]
    fn carry_forward_fills_a_trailing_gap() {
        let filled = fill_gaps(
            &[iv(0, dec!(3.0))],
            &cfg(0, 30)
                .with_method(SubstituteMethod::LastValueCarryForward)
                .short_gap_threshold(0),
        );
        assert_eq!(filled.intervals.len(), 2);
        assert_eq!(filled.intervals[1].value, dec!(3.0));
        assert_eq!(
            filled.substitutions[0].method,
            SubstituteMethod::LastValueCarryForward
        );
        assert_eq!(filled.substitutions[0].reference_count, 1);
    }

    /// A leading gap has nothing before it, so interpolation degrades to
    /// carrying the *following* value back — and says so.
    #[test]
    fn a_leading_gap_carries_the_first_value_back() {
        let filled = fill_gaps(&[iv(30, dec!(5.0))], &cfg(0, 45));
        assert_eq!(filled.intervals.len(), 3);
        assert_eq!(filled.intervals[0].value, dec!(5.0));
        assert_eq!(
            filled.substitutions[0].method,
            SubstituteMethod::LastValueCarryForward,
            "the audit record must name the fallback that ran, not the request"
        );
    }

    #[test]
    fn no_data_at_all_yields_zeros_with_no_references() {
        let filled = fill_gaps(&[], &cfg(0, 30));
        assert_eq!(filled.intervals.len(), 2);
        assert!(filled.intervals.iter().all(|iv| iv.value.is_zero()));
        for entry in &filled.substitutions {
            assert_eq!(entry.method, SubstituteMethod::ZeroFill);
            assert_eq!(entry.reference_count, 0);
        }
        assert!((filled.measured_pct() - 0.0).abs() < 1e-9);
    }

    // ── prior-period average ─────────────────────────────────────────────────

    #[test]
    fn prior_period_average_uses_the_matching_slot() {
        let prior = vec![iv_at(datetime!(2025-12-25 0:15 UTC), dec!(3.0))];
        let filled = fill_gaps(
            &[iv(0, dec!(2.0)), iv(30, dec!(4.0))],
            &cfg(0, 45).prior_period(prior).short_gap_threshold(0),
        );
        assert_eq!(filled.intervals[1].value, dec!(3.0));
        assert_eq!(
            filled.substitutions[0].method,
            SubstituteMethod::PriorPeriodAverage
        );
        assert_eq!(filled.substitutions[0].reference_count, 1);
    }

    #[test]
    fn prior_period_average_falls_back_to_carry_forward() {
        // The reference week has data, but not at the slot the gap needs.
        let prior = vec![iv_at(datetime!(2025-12-25 1:00 UTC), dec!(5.0))];
        let filled = fill_gaps(
            &[iv(0, dec!(2.5)), iv(30, dec!(4.0))],
            &cfg(0, 45).prior_period(prior).short_gap_threshold(0),
        );
        assert_eq!(filled.intervals[1].value, dec!(2.5));
        assert_eq!(
            filled.substitutions[0].method,
            SubstituteMethod::LastValueCarryForward
        );
    }

    /// Matching on time of day alone averages a Sunday gap over the working
    /// week, which overstates an industrial load by an order of magnitude.
    #[test]
    fn prior_period_average_distinguishes_weekdays_from_weekends() {
        // 2026-03-01 is a Sunday; 2026-02-23..27 are Mon–Fri.
        let mut prior: Vec<MeterInterval> = (23..=27)
            .map(|day| {
                iv_at(
                    datetime!(2026-02-01 08:00 UTC).replace_day(day).unwrap(),
                    dec!(100),
                )
            })
            .collect();
        prior.push(iv_at(datetime!(2026-02-22 08:00 UTC), dec!(4)));

        let gap = datetime!(2026-03-01 08:00 UTC);
        let filled = fill_gaps(
            &[],
            &FillGapsConfig::new(
                IntervalResolution::QuarterHour,
                gap,
                gap + Duration::minutes(15),
            )
            .prior_period(prior)
            .short_gap_threshold(0),
        );
        assert_eq!(
            filled.intervals[0].value,
            dec!(4),
            "a Sunday gap takes the prior Sunday's value, not the working-week average"
        );
    }

    /// Only the preceding week counts. The window used to be a `debug_assert`
    /// on the caller's slice length, which compiles out in release.
    #[test]
    fn only_the_preceding_week_feeds_the_average() {
        let gap = datetime!(2026-03-09 08:00 UTC); // a Monday
        let prior = vec![
            iv_at(datetime!(2026-03-02 08:00 UTC), dec!(10)), // inside the window
            iv_at(datetime!(2026-02-16 08:00 UTC), dec!(1000)), // three weeks back
        ];
        let filled = fill_gaps(
            &[],
            &FillGapsConfig::new(
                IntervalResolution::QuarterHour,
                gap,
                gap + Duration::minutes(15),
            )
            .prior_period(prior)
            .short_gap_threshold(0),
        );
        assert_eq!(
            filled.intervals[0].value,
            dec!(10),
            "averaging in the older week would give 505"
        );
    }

    /// The Vergleichstag must survive the fall-back week. The matching slot —
    /// same Berlin (weekday, hour, minute), seven calendar days earlier — is
    /// **169 UTC hours** back when the 25-hour day lies between, and a fixed
    /// `Duration::days(7)` window excluded it, silently degrading the
    /// configured method to carry-forward for a week every October.
    #[test]
    fn prior_period_average_survives_the_fall_back_week() {
        // Gap: Wednesday 2026-10-28 12:00 Berlin (CET, 11:00 UTC).
        // Reference: Wednesday 2026-10-21 12:00 Berlin (CEST, 10:00 UTC).
        let gap_start = datetime!(2026-10-28 11:00 UTC);
        let prior = vec![iv_at(datetime!(2026-10-21 10:00 UTC), dec!(7.5))];

        let filled = fill_gaps(
            &[],
            &FillGapsConfig::new(
                IntervalResolution::QuarterHour,
                gap_start,
                gap_start + Duration::minutes(15),
            )
            .prior_period(prior)
            .short_gap_threshold(0),
        );
        assert_eq!(filled.intervals[0].value, dec!(7.5));
        assert_eq!(
            filled.substitutions[0].method,
            SubstituteMethod::PriorPeriodAverage,
            "the matching slot is one local week back and must be found, \
             not silently replaced by carry-forward"
        );
        assert_eq!(filled.substitutions[0].reference_count, 1);
    }

    /// ...while the window still ends where it should: the same slot *eight*
    /// days back is outside the week whatever the season.
    #[test]
    fn the_fall_back_window_does_not_overreach() {
        let gap_start = datetime!(2026-10-29 11:00 UTC); // Thursday 12:00 CET
        // The matching Thursday slot 14 days earlier — outside any one-week
        // window, DST or not.
        let prior = vec![iv_at(datetime!(2026-10-15 10:00 UTC), dec!(999))];
        let filled = fill_gaps(
            &[],
            &FillGapsConfig::new(
                IntervalResolution::QuarterHour,
                gap_start,
                gap_start + Duration::minutes(15),
            )
            .prior_period(prior)
            .short_gap_threshold(0),
        );
        assert_ne!(
            filled.substitutions[0].method,
            SubstituteMethod::PriorPeriodAverage,
            "a fortnight-old sample is not in the reference week"
        );
    }

    // ── interpolation across faulty slots ────────────────────────────────────

    /// A present-but-faulty slot terminates the missing run but is not the
    /// closing anchor: the line runs to the next **billable** value at its
    /// true distance. The old geometry measured the span to the faulty slot
    /// and the value from the billable one — every interior value at the
    /// wrong fraction.
    #[test]
    fn interpolation_spans_to_the_billable_closing_value() {
        let mut faulty = iv(60, dec!(999));
        faulty.quality = QualityFlag::Faulty;
        let series = vec![iv(0, dec!(0)), faulty, iv(75, dec!(100))];

        let filled = fill_gaps(&series, &cfg(0, 90).short_gap_threshold(10));
        let values: Vec<Decimal> = filled.intervals.iter().map(|iv| iv.value).collect();
        // Slots: 0 (billable), three missing, 999 (faulty, untouched), 100.
        // The line runs 0 → 100 over five steps: 20, 40, 60 — not the 25,
        // 50, 75 of a four-step span ending on the faulty slot.
        assert_eq!(
            values,
            vec![dec!(0), dec!(20), dec!(40), dec!(60), dec!(999), dec!(100)]
        );
        assert_eq!(
            filled.substituted_count(),
            3,
            "the faulty slot is passed through, never substituted"
        );
        assert!(
            filled.intervals[4].quality == QualityFlag::Faulty,
            "…and keeps its quality"
        );
    }

    /// The mirror case: a faulty slot *before* the run. The preceding anchor
    /// is the last billable value at its true distance, so the missing slots
    /// sit at offsets 2⁄4 and 3⁄4 of the span — not 1⁄3 and 2⁄3 of a
    /// shortened one.
    #[test]
    fn interpolation_anchors_on_the_billable_preceding_value() {
        let mut faulty = iv(15, dec!(999));
        faulty.quality = QualityFlag::Faulty;
        let series = vec![iv(0, dec!(0)), faulty, iv(60, dec!(100))];

        let filled = fill_gaps(&series, &cfg(0, 75).short_gap_threshold(10));
        let values: Vec<Decimal> = filled.intervals.iter().map(|iv| iv.value).collect();
        assert_eq!(
            values,
            vec![dec!(0), dec!(999), dec!(50), dec!(75), dec!(100)]
        );
        assert_eq!(filled.substituted_count(), 2);
    }

    /// Two missing runs separated by a faulty slot interpolate on **one**
    /// straight line between the same two billable anchors — the run
    /// partitioning must not bend the line.
    #[test]
    fn runs_split_by_a_faulty_slot_share_one_line() {
        let mut faulty = iv(30, dec!(999));
        faulty.quality = QualityFlag::Faulty;
        let series = vec![iv(0, dec!(0)), faulty, iv(75, dec!(100))];

        let filled = fill_gaps(&series, &cfg(0, 90).short_gap_threshold(10));
        let values: Vec<Decimal> = filled.intervals.iter().map(|iv| iv.value).collect();
        // Slots 1, 3, 4 are missing around the faulty slot 2; all three sit
        // on the single 0 → 100 line over five steps.
        assert_eq!(
            values,
            vec![dec!(0), dec!(20), dec!(999), dec!(60), dec!(80), dec!(100)]
        );
    }

    #[test]
    fn faulty_reference_values_are_excluded() {
        let mut faulty = iv_at(datetime!(2025-12-25 0:15 UTC), dec!(999));
        faulty.quality = QualityFlag::Faulty;
        let good = iv_at(datetime!(2025-12-25 0:15 UTC), dec!(3.0));

        let filled = fill_gaps(
            &[iv(0, dec!(2.0))],
            &cfg(15, 30)
                .prior_period(vec![faulty, good])
                .short_gap_threshold(0),
        );
        assert_eq!(filled.intervals[0].value, dec!(3.0));
    }

    // ── audit trail ──────────────────────────────────────────────────────────

    #[test]
    fn every_substitute_carries_its_reason_and_flag() {
        let filled = fill_gaps(
            &[iv(0, dec!(2.0)), iv(45, dec!(2.0))],
            &cfg(0, 60).because(SubstitutionReason::GatewayCommFailure),
        );
        assert_eq!(filled.substituted_count(), 2);
        for entry in &filled.substitutions {
            assert_eq!(entry.reason, SubstitutionReason::GatewayCommFailure);
            assert_eq!(entry.interval.quality, QualityFlag::Substituted);
            assert!(!entry.reason.description().is_empty());
            assert!(!entry.method.description().is_empty());
        }
        assert!((filled.measured_pct() - 50.0).abs() < 1e-9);
    }

    /// An audit trail that disagrees with the data it describes is worse than
    /// none.
    #[test]
    fn the_audit_trail_matches_the_series() {
        let filled = fill_gaps(&[iv(0, dec!(2.0)), iv(60, dec!(6.0))], &cfg(0, 75));
        let substituted: Vec<&MeterInterval> = filled
            .intervals
            .iter()
            .filter(|iv| iv.quality == QualityFlag::Substituted)
            .collect();
        assert_eq!(substituted.len(), filled.substitutions.len());
        for (iv, entry) in substituted.iter().zip(&filled.substitutions) {
            assert_eq!(**iv, entry.interval);
        }
    }

    #[test]
    fn substitutes_inherit_the_obis_channel() {
        let mut first = iv(0, dec!(2.0));
        first.obis_code = Some(crate::ObisCode::STROM_BEZUG_LASTGANG);
        let filled = fill_gaps(&[first], &cfg(0, 30));
        assert_eq!(
            filled.intervals[1].obis_code,
            Some(crate::ObisCode::STROM_BEZUG_LASTGANG)
        );
    }

    // ── the calendar grid ────────────────────────────────────────────────────

    /// A daily fill must walk **calendar** days. Stepping a fixed 86 400 s
    /// drifts by an hour at each DST transition and never recovers.
    #[test]
    fn a_daily_fill_follows_the_calendar_across_dst() {
        use crate::calendar;
        use time::macros::date;

        let days: Vec<time::Date> = (0..14)
            .map(|i| {
                date!(2026 - 03 - 23)
                    .checked_add(Duration::days(i))
                    .unwrap()
            })
            .collect();
        let mut series: Vec<MeterInterval> = days
            .iter()
            .map(|&day| MeterInterval {
                from: calendar::day_start_utc(day),
                to: calendar::day_end_utc(day),
                value: dec!(100),
                quality: QualityFlag::Measured,
                obis_code: None,
            })
            .collect();
        let dropped = series.remove(10);

        let period = (
            calendar::day_start_utc(days[0]),
            calendar::day_end_utc(*days.last().unwrap()),
        );
        let filled = fill_gaps(
            &series,
            &FillGapsConfig::new(IntervalResolution::Day, period.0, period.1),
        );

        assert_eq!(filled.intervals.len(), 14, "one slot per calendar day");
        assert_eq!(
            filled.substituted_count(),
            1,
            "exactly the dropped day is synthesised, not everything after the \
             transition: {:?}",
            filled
                .substitutions
                .iter()
                .map(|e| e.interval.from)
                .collect::<Vec<_>>()
        );
        assert_eq!(filled.substitutions[0].interval.from, dropped.from);
        assert_eq!(filled.substitutions[0].interval.to, dropped.to);

        // The slots are calendar days, so the 23-hour one really is 23 hours.
        let short = filled
            .intervals
            .iter()
            .find(|iv| calendar::local_day(iv.from) == date!(2026 - 03 - 29))
            .expect("the spring-forward day is in the range");
        assert_eq!((short.to - short.from).whole_hours(), 23);

        // The same series on a fixed 24-hour grid desynchronises at the
        // transition and substitutes most of what follows.
        let fixed = fill_gaps(
            &series,
            &FillGapsConfig::new(IntervalResolution::Custom(86_400), period.0, period.1),
        );
        assert!(
            fixed.substituted_count() > 5,
            "a fixed 24-hour grid loses every day after the transition, \
             substituting {} of them",
            fixed.substituted_count()
        );
    }

    // ── degenerate input ─────────────────────────────────────────────────────

    #[test]
    fn degenerate_parameters_do_not_loop_or_panic() {
        let intervals = vec![iv(0, dec!(2.0))];

        // A resolution with no grid at all: the data comes back untouched.
        let no_grid = fill_gaps(
            &intervals,
            &FillGapsConfig::new(
                IntervalResolution::Custom(0),
                BASE,
                BASE + Duration::hours(1),
            ),
        );
        assert_eq!(no_grid.intervals.len(), 1);
        assert!(no_grid.substitutions.is_empty());

        // Inverted range.
        let inverted = fill_gaps(
            &intervals,
            &FillGapsConfig::new(
                IntervalResolution::QuarterHour,
                BASE + Duration::hours(1),
                BASE,
            ),
        );
        assert!(inverted.intervals.is_empty());
        assert!(inverted.substitutions.is_empty());

        // Empty range.
        let empty = fill_gaps(
            &intervals,
            &FillGapsConfig::new(IntervalResolution::QuarterHour, BASE, BASE),
        );
        assert!(empty.intervals.is_empty());
    }

    /// A long gap must not be truncated. The previous implementation capped its
    /// gap-length scan at 100 and silently changed method past that point.
    #[test]
    fn a_gap_longer_than_a_hundred_intervals_is_measured_in_full() {
        let intervals = vec![
            iv_at(BASE, dec!(10)),
            iv_at(BASE + Duration::days(2), dec!(10)),
        ];
        let filled = fill_gaps(
            &intervals,
            &FillGapsConfig::new(
                IntervalResolution::QuarterHour,
                BASE,
                BASE + Duration::days(2) + Duration::minutes(15),
            )
            .with_method(SubstituteMethod::ZeroFill)
            .short_gap_threshold(3),
        );
        assert_eq!(filled.substituted_count(), 191);
        assert!(
            filled
                .substitutions
                .iter()
                .all(|e| e.method == SubstituteMethod::ZeroFill),
            "a 191-interval gap is not a short gap at any point in it"
        );
    }

    #[test]
    fn enum_metadata_is_complete() {
        for m in SubstituteMethod::ALL {
            assert!(!m.description().is_empty(), "{m:?}");
        }
        for r in SubstitutionReason::ALL {
            assert!(!r.description().is_empty(), "{r:?}");
        }
        assert_eq!(SubstituteMethod::ALL.len(), 4);
        assert_eq!(SubstitutionReason::ALL.len(), 7);
    }
}
