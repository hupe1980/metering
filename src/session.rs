//! Sessions and device logs → Lastgang: distributing a total across the
//! metering grid.
//!
//! Some energy is measured as a **total over a span** rather than per slot — a
//! charge point's Charge Detail Record, a submetered heat pump between two
//! visits, a device log for a day. The market settles on slots, so that energy
//! has to be placed on the grid before it can be allocated, balanced or
//! invoiced. [`split_session`] places it; [`merge_sessions`] adds several
//! sessions already on the same grid.
//!
//! ```text
//! Σ slot energy = the session total        exactly, always
//! ```
//!
//! # Two ways a slot's energy can be known
//!
//! | Basis | Where it comes from | Quality |
//! |---|---|---|
//! | **Metered** | the difference of two register readings on this slot's own boundaries | [`QualityFlag::Measured`] |
//! | **Pro rata** | a span straddling a boundary, divided by wall-clock time | [`QualityFlag::Estimated`] |
//!
//! The distinction is not cosmetic: a supplier settling a § 42b allocation and
//! a customer disputing a peak both need to know which quarter-hours were
//! measured and which were inferred from a constant-power assumption the
//! session did not obey. OCPP's clock-aligned meter values exist so the first
//! row can be filled in, and this module uses them where they land on the grid.
//!
//! Pro rata is not a **profile**. If a better shape is known, express it by
//! supplying more [`MeterSample`]s.
//!
//! # The grid is the calendar's
//!
//! Slots come from [`DayBoundary::bucket_bounds`], the same function
//! [`crate::resample()`] buckets with — so a session across the October Sunday
//! sees the repeated hour, and a daily or Gastag grid gets 23, 24 or 25 hours
//! rather than a flat 86 400 s. A second implementation would drift, and the
//! two would then disagree about which slot a kWh belongs to.
//!
//! Dividing one slot's energy between several claims on it is the other
//! direction: [`crate::allocation::allocate`].

use std::collections::BTreeMap;

use rust_decimal::Decimal;
use time::OffsetDateTime;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use crate::allocation::allocation_share;
use crate::calendar::DayBoundary;
use crate::interval::{MeterInterval, QualityFlag};
use crate::obis::ObisCode;
use crate::resolution::IntervalResolution;

// ── MeterSample ───────────────────────────────────────────────────────────────

/// A register reading taken during a session.
///
/// `reading` is a **cumulative** register value — what the meter displayed at
/// `at` — not the energy since the last sample. That is what OCPP
/// `MeterValues` carry in `Energy.Active.Import.Register` and what any
/// Eichrecht-conformant charge point signs, and it is the form that makes a
/// missing sample harmless: only the *difference* between two readings is ever
/// used, so a gap in the middle of a session widens one segment rather than
/// losing energy.
///
/// The unit is the session's own — see [`split_session`]. Only differences are
/// taken, so a register offset is irrelevant and the reading need not start at
/// zero.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct MeterSample {
    /// When the register was read (UTC).
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::rfc3339"))]
    pub at: OffsetDateTime,
    /// The cumulative register value at that instant.
    pub reading: Decimal,
}

impl MeterSample {
    /// A reading of `reading` at `at`.
    #[must_use]
    pub const fn new(at: OffsetDateTime, reading: Decimal) -> Self {
        Self { at, reading }
    }
}

// ── SessionSplitConfig ────────────────────────────────────────────────────────

/// Which grid to place a session on, and how to flag what lands there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionSplitConfig {
    /// The slot length. [`QuarterHour`](IntervalResolution::QuarterHour) is
    /// the German settlement grid for electricity.
    pub resolution: IntervalResolution,
    /// Where a day is cut, for the calendar resolutions.
    ///
    /// Irrelevant for sub-daily slots, which snap in UTC.
    pub day_boundary: DayBoundary,
    /// The OBIS channel to stamp on every emitted interval.
    ///
    /// Stamp it and [`MeterInterval::direction`] answers, which is what makes
    /// a bidirectional charge point's import and export series distinguishable
    /// downstream — `1-0:1.8.0` for a charge, `1-0:2.8.0` for a V2G discharge.
    pub obis_code: Option<ObisCode>,
    /// The flag for a slot whose energy came from register readings on its own
    /// boundaries.
    pub metered_quality: QualityFlag,
    /// The flag for a slot that had to be pro-rated across a boundary.
    pub prorated_quality: QualityFlag,
}

impl SessionSplitConfig {
    /// The German electricity settlement grid: quarter-hours cut at midnight,
    /// pro-rated slots flagged [`Estimated`](QualityFlag::Estimated).
    #[must_use]
    pub const fn quarter_hourly() -> Self {
        Self {
            resolution: IntervalResolution::QuarterHour,
            day_boundary: DayBoundary::Midnight,
            obis_code: None,
            metered_quality: QualityFlag::Measured,
            prorated_quality: QualityFlag::Estimated,
        }
    }

    /// A different slot length (builder style).
    #[must_use]
    pub const fn at(mut self, resolution: IntervalResolution) -> Self {
        self.resolution = resolution;
        self
    }

    /// Cut days on this boundary (builder style) — the Gastag, for gas.
    #[must_use]
    pub const fn on(mut self, boundary: DayBoundary) -> Self {
        self.day_boundary = boundary;
        self
    }

    /// Stamp this OBIS channel on every emitted interval (builder style).
    #[must_use]
    pub const fn with_obis(mut self, code: ObisCode) -> Self {
        self.obis_code = Some(code);
        self
    }

    /// Flag pro-rated slots with something other than
    /// [`Estimated`](QualityFlag::Estimated) (builder style).
    #[must_use]
    pub const fn prorated_as(mut self, flag: QualityFlag) -> Self {
        self.prorated_quality = flag;
        self
    }

    /// Flag exactly-known slots with something other than
    /// [`Measured`](QualityFlag::Measured) (builder style).
    ///
    /// The pair matters for a source that is not the Messstellenbetreiber's
    /// meter: a heat pump's own log is a real register difference, so the slot
    /// value is exact, but it is not an eichrechtskonform measurement and
    /// stamping it `Measured` would claim more than the device can support.
    /// [`Calculated`](QualityFlag::Calculated) is the usual answer there.
    #[must_use]
    pub const fn metered_as(mut self, flag: QualityFlag) -> Self {
        self.metered_quality = flag;
        self
    }
}

impl Default for SessionSplitConfig {
    fn default() -> Self {
        Self::quarter_hourly()
    }
}

// ── SessionError ──────────────────────────────────────────────────────────────

/// Why a session could not be placed on the grid.
///
/// `#[non_exhaustive]`: a caller that wildcards an unrecognised variant still
/// behaves correctly — it reports a failure.
#[derive(Debug, thiserror::Error, PartialEq, Eq, Clone)]
#[non_exhaustive]
pub enum SessionError {
    /// The span is empty or runs backwards.
    #[error("session span {from} → {to} is empty or reversed")]
    EmptySpan {
        /// The supplied start.
        from: OffsetDateTime,
        /// The supplied end.
        to: OffsetDateTime,
    },

    /// The session total is below zero.
    ///
    /// A session register counts up. A negative total is a sign error or a
    /// swapped pair of readings, and pro-rating it would spread that error
    /// silently across the grid. A V2G discharge is not a negative charge: it
    /// is its own session on the export register — see
    /// [`SessionSplitConfig::obis_code`].
    #[error("session energy {energy} is negative")]
    NegativeEnergy {
        /// The supplied total.
        energy: Decimal,
    },

    /// A sample was taken outside the session span.
    #[error("sample at {at} falls outside the session span")]
    SampleOutsideSpan {
        /// The offending sample's instant.
        at: OffsetDateTime,
    },

    /// A register reading went backwards.
    #[error("register reading at {at} is below the previous one")]
    SamplesNotMonotonic {
        /// The instant at which the register decreased.
        at: OffsetDateTime,
    },

    /// The samples and the session total cannot both be true.
    ///
    /// Either the samples account for more energy than the session claims, or
    /// they account for less and there is no uncovered span left to put the
    /// difference in — a first sample at the session start and a last one at
    /// its end leave nowhere for a discrepancy to go.
    #[error(
        "samples account for {sampled} of a {session} session, and the difference has nowhere to go"
    )]
    SampleTotalMismatch {
        /// The session total that was supplied.
        session: Decimal,
        /// What the register readings span.
        sampled: Decimal,
    },
}

// ── split_session ─────────────────────────────────────────────────────────────

/// A span of the session over which the energy is known and constant-rate.
struct Segment {
    start: OffsetDateTime,
    end: OffsetDateTime,
    energy: Decimal,
    /// Cumulative energy delivered *before* this segment began.
    cum_before: Decimal,
    /// `true` when this segment's energy is a measured difference rather than
    /// a share of an unmeasured remainder.
    exact: bool,
}

/// Place a session's energy on the metering grid.
///
/// `energy` is the session total in the series' own unit — kWh for
/// [`Sparte::Strom`](crate::Sparte::Strom). OCPP reports Wh; convert at the
/// boundary. `samples` are cumulative register readings taken during the
/// session, in any order, and may be empty — then the whole total is spread
/// pro rata.
///
/// One [`MeterInterval`] per grid slot the session touches, contiguous and
/// ascending, each spanning its **whole** slot rather than only the part the
/// session occupied: that is what makes two sessions addable and lets the
/// output go straight into [`aggregate`](crate::aggregate),
/// [`resample`](crate::resample()) or
/// [`validate_intervals`](crate::validate_intervals). A slot the session
/// touched but delivered nothing in is a zero, not a gap.
///
/// `Σ slot = energy`, exactly. Each slot is the **difference of two adjacent
/// cumulatives**, so the series telescopes to `cum(end) − cum(start)` whatever
/// rounding the cumulative itself needed; the cut is
/// [`ALLOCATION_DP`](crate::ALLOCATION_DP), because a session split is an
/// allocation of a total across slots. Truncation toward zero keeps the
/// cumulative monotone, so no slot comes back negative.
///
/// A slot is [`Measured`](QualityFlag::Measured) when every segment reaching
/// into it is a register difference lying wholly inside it, and
/// [`Estimated`](QualityFlag::Estimated) otherwise — see the
/// [module docs](self).
///
/// # Errors
///
/// See [`SessionError`]. All of them are contradictions in the input — an
/// empty span, a register that ran backwards, a total the samples disagree
/// with — never a shape this function merely does not handle.
///
/// ```rust
/// use metering::session::{MeterSample, SessionSplitConfig, split_session};
/// use metering::QualityFlag;
/// use rust_decimal::dec;
/// use time::macros::datetime;
///
/// // The charge point sent a clock-aligned reading at 12:15 and 12:30.
/// let samples = [
///     MeterSample::new(datetime!(2026-06-01 12:15 UTC), dec!(1000)),
///     MeterSample::new(datetime!(2026-06-01 12:30 UTC), dec!(1006)),
/// ];
///
/// let slots = split_session(
///     datetime!(2026-06-01 12:07 UTC),
///     datetime!(2026-06-01 12:37 UTC),
///     dec!(10),
///     &samples,
///     &SessionSplitConfig::quarter_hourly(),
/// )?;
///
/// // Bounded by two readings, so measured; the two ends were pro-rated.
/// assert_eq!(slots[1].value, dec!(6));
/// assert_eq!(slots[1].quality, QualityFlag::Measured);
/// assert_eq!(slots[0].quality, QualityFlag::Estimated);
///
/// assert_eq!(slots.iter().map(|s| s.value).sum::<rust_decimal::Decimal>(), dec!(10));
/// # Ok::<(), metering::session::SessionError>(())
/// ```
pub fn split_session(
    from: OffsetDateTime,
    to: OffsetDateTime,
    energy: Decimal,
    samples: &[MeterSample],
    config: &SessionSplitConfig,
) -> Result<Vec<MeterInterval>, SessionError> {
    if to <= from {
        return Err(SessionError::EmptySpan { from, to });
    }
    if energy < Decimal::ZERO {
        return Err(SessionError::NegativeEnergy { energy });
    }

    let segments = build_segments(from, to, energy, samples)?;

    let mut out = Vec::new();
    let mut slot_start = config.day_boundary.bucket_bounds(from, config.resolution).0;
    // Segments and slots both run forward, so one index walks the segment list
    // once across the session instead of it being rescanned per slot — which
    // for a day-long log sampled every minute is 1 440 × 96 comparisons.
    let mut cursor = 0usize;
    let mut carried = Decimal::ZERO;

    while slot_start < to {
        let (bucket_from, bucket_to) = config
            .day_boundary
            .bucket_bounds(slot_start, config.resolution);
        // A resolution that failed to advance would loop forever. It cannot —
        // every fixed length is at least a second and every calendar period at
        // least a day — but the loop is not the place to rely on that.
        if bucket_to <= bucket_from {
            break;
        }

        let covered_from = bucket_from.max(from);
        let covered_to = bucket_to.min(to);

        // The cumulative at the slot's start is the one computed for the
        // previous slot's end, so each boundary is evaluated once.
        let opening = if out.is_empty() {
            cumulative(&segments, cursor, from, to, energy, covered_from)
        } else {
            carried
        };

        // `cursor` indexes the first segment still open at `covered_from`:
        // everything before it ended at or before this slot began, so neither
        // the cumulative nor the quality needs to look at it again.
        let closing = cumulative(&segments, cursor, from, to, energy, covered_to);
        carried = closing;

        // Estimated unless every segment that reaches into this slot is a
        // measured difference *and* lies wholly inside it — a segment cut by a
        // slot boundary was divided by wall-clock time, whatever its own
        // energy is worth.
        let exact = segments[cursor..]
            .iter()
            .take_while(|s| s.start < covered_to)
            .all(|s| s.exact && s.start >= covered_from && s.end <= covered_to);

        // Only now advance, so the cursor keeps pointing at the first segment
        // open at the *next* slot's start.
        while cursor + 1 < segments.len() && segments[cursor].end <= covered_to {
            cursor += 1;
        }

        out.push(MeterInterval {
            from: bucket_from,
            to: bucket_to,
            value: closing - opening,
            quality: if exact {
                config.metered_quality
            } else {
                config.prorated_quality
            },
            obis_code: config.obis_code,
        });
        slot_start = bucket_to;
    }
    Ok(out)
}

/// Split the span into constant-rate segments, one per gap between anchors.
///
/// Anchors are the session's two ends and every sample instant. Between two
/// samples the energy is their register difference and is *exact*; the head
/// before the first sample and the tail after the last share whatever the
/// samples did not account for, and are exact only when there is just one of
/// them to give it all to.
fn build_segments(
    from: OffsetDateTime,
    to: OffsetDateTime,
    energy: Decimal,
    samples: &[MeterSample],
) -> Result<Vec<Segment>, SessionError> {
    for s in samples {
        if s.at < from || s.at > to {
            return Err(SessionError::SampleOutsideSpan { at: s.at });
        }
    }

    // Sorted, and one reading per instant: two readings at the same instant
    // describe a step, and the later one is the state of the register from
    // then on. Folding the step into the preceding segment is the only
    // placement that does not invent a zero-length rate.
    let mut sorted: Vec<MeterSample> = samples.to_vec();
    sorted.sort_by_key(|s| s.at);
    sorted.dedup_by(|later, earlier| {
        if later.at == earlier.at {
            earlier.reading = later.reading;
            true
        } else {
            false
        }
    });

    for pair in sorted.windows(2) {
        if pair[1].reading < pair[0].reading {
            return Err(SessionError::SamplesNotMonotonic { at: pair[1].at });
        }
    }

    let sampled = match (sorted.first(), sorted.last()) {
        (Some(first), Some(last)) => last.reading - first.reading,
        _ => Decimal::ZERO,
    };
    let remainder = energy - sampled;
    if remainder < Decimal::ZERO {
        return Err(SessionError::SampleTotalMismatch {
            session: energy,
            sampled,
        });
    }

    let head = sorted.first().map_or(to, |s| s.at); // no samples → one span
    let tail = sorted.last().map_or(to, |s| s.at);
    let head_secs = (head - from).whole_seconds().max(0);
    let tail_secs = (to - tail).whole_seconds().max(0);
    let uncovered = head_secs + tail_secs;

    if uncovered == 0 && !remainder.is_zero() {
        return Err(SessionError::SampleTotalMismatch {
            session: energy,
            sampled,
        });
    }

    // The remainder is split between the two uncovered ends by wall-clock
    // time, and the tail takes the difference so the two add back exactly.
    let head_energy = if uncovered == 0 {
        Decimal::ZERO
    } else {
        allocation_share(remainder * Decimal::from(head_secs) / Decimal::from(uncovered))
    };
    let tail_energy = remainder - head_energy;
    // Exact when it is the whole remainder rather than a share of it.
    let ends_are_exact = head_secs == 0 || tail_secs == 0;

    let mut segments: Vec<Segment> = Vec::with_capacity(sorted.len() + 1);
    let mut cum_before = Decimal::ZERO;
    let mut push = |start: OffsetDateTime, end: OffsetDateTime, e: Decimal, exact: bool| {
        if end > start {
            segments.push(Segment {
                start,
                end,
                energy: e,
                cum_before,
                exact,
            });
            cum_before += e;
        }
    };

    if head_secs > 0 {
        push(from, head, head_energy, ends_are_exact);
    }
    for pair in sorted.windows(2) {
        push(
            pair[0].at,
            pair[1].at,
            pair[1].reading - pair[0].reading,
            true,
        );
    }
    if tail_secs > 0 {
        push(tail, to, tail_energy, ends_are_exact);
    }
    Ok(segments)
}

/// Energy delivered from the session start up to `t`.
///
/// Pinned at both ends — `0` at `from` and the session total at `to` — so the
/// slot differences telescope to exactly the total. In between it is the
/// running sum of whole segments plus the wall-clock share of the segment `t`
/// falls in, cut to [`ALLOCATION_DP`](crate::ALLOCATION_DP).
///
/// Monotonic, because truncation toward zero preserves the order of a
/// non-decreasing function, which is why no slot can come back negative.
fn cumulative(
    segments: &[Segment],
    hint: usize,
    from: OffsetDateTime,
    to: OffsetDateTime,
    energy: Decimal,
    t: OffsetDateTime,
) -> Decimal {
    if t <= from {
        return Decimal::ZERO;
    }
    if t >= to {
        return energy;
    }
    // `hint` is the caller's cursor: the segments tile the span in order, so
    // the one containing `t` is at or after it. The scan from there is bounded
    // by how far the cursor has yet to advance, and across a whole session it
    // walks the list once.
    let Some(seg) = segments[hint.min(segments.len())..]
        .iter()
        .find(|s| s.start < t && t <= s.end)
    else {
        return Decimal::ZERO;
    };
    let span = (seg.end - seg.start).whole_seconds();
    if span <= 0 {
        return allocation_share(seg.cum_before + seg.energy);
    }
    let elapsed = (t - seg.start).whole_seconds();
    allocation_share(seg.cum_before + seg.energy * Decimal::from(elapsed) / Decimal::from(span))
}

// ── merge_sessions ────────────────────────────────────────────────────────────

/// Add several series that share a grid, slot by slot.
///
/// The step after [`split_session`]: a Übergabestelle has many sessions behind
/// it, and what a Bilanzkreis settles is their sum.
///
/// **Union, not intersection.**
/// [`compute_virtual_meter`](crate::compute_virtual_meter) with
/// [`Sum`](crate::AggregationRule::Sum) keeps only the timestamps present in
/// *all* its sources, because a missing source there means the total would be
/// wrong. A charge point that was idle contributes no intervals, and that
/// absence **is** zero energy. It is a separate function rather than a flag
/// because choosing the wrong one silently produces a plausible number.
///
/// Grouped by `(from, to, obis_code)`, so a bidirectional point's import and
/// export series do not collapse into one meaningless total. Sorted by `from`,
/// then by channel; each slot carries the **worst** quality among its
/// contributors, and is order-independent for the same reason.
///
/// Only slots something touched appear: filling an idle hour with zeros is
/// [`fill_gaps`](crate::fill_gaps), which records what it invented. Series on
/// different grids are not reconciled either — the result then overlaps
/// itself, which [`validate_intervals`](crate::validate_intervals) reports as
/// V02 and [`resample`](crate::resample()) is the fix for.
///
/// ```rust
/// use metering::session::{SessionSplitConfig, merge_sessions, split_session};
/// use rust_decimal::dec;
/// use time::macros::datetime;
///
/// let cfg = SessionSplitConfig::quarter_hourly();
///
/// // Two cars, overlapping in the 12:15 slot and nowhere else.
/// let a = split_session(
///     datetime!(2026-06-01 12:00 UTC), datetime!(2026-06-01 12:30 UTC),
///     dec!(8), &[], &cfg,
/// )?;
/// let b = split_session(
///     datetime!(2026-06-01 12:15 UTC), datetime!(2026-06-01 12:45 UTC),
///     dec!(4), &[], &cfg,
/// )?;
///
/// let merged = merge_sessions(&[a, b]);
///
/// assert_eq!(merged.len(), 3, "a union of the slots");
/// assert_eq!(merged[1].value, dec!(6), "both cars charging");
/// assert_eq!(merged.iter().map(|s| s.value).sum::<rust_decimal::Decimal>(), dec!(12));
/// # Ok::<(), metering::session::SessionError>(())
/// ```
#[must_use]
pub fn merge_sessions(series: &[Vec<MeterInterval>]) -> Vec<MeterInterval> {
    let mut slots: BTreeMap<
        (OffsetDateTime, OffsetDateTime, Option<ObisCode>),
        (Decimal, QualityFlag),
    > = BTreeMap::new();

    for one in series {
        for iv in one {
            let entry = slots
                .entry((iv.from, iv.to, iv.obis_code))
                .or_insert((Decimal::ZERO, QualityFlag::Measured));
            entry.0 += iv.value;
            entry.1 = entry.1.worse_of(iv.quality);
        }
    }

    slots
        .into_iter()
        .map(|((from, to, obis_code), (value, quality))| MeterInterval {
            from,
            to,
            value,
            quality,
            obis_code,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::calendar;
    use rust_decimal::dec;
    use time::Duration;
    use time::macros::{date, datetime};

    fn cfg() -> SessionSplitConfig {
        SessionSplitConfig::quarter_hourly()
    }

    fn total(slots: &[MeterInterval]) -> Decimal {
        slots.iter().map(|s| s.value).sum()
    }

    #[test]
    fn a_session_inside_one_slot_stays_in_it() {
        let slots = split_session(
            datetime!(2026-06-01 12:02 UTC),
            datetime!(2026-06-01 12:09 UTC),
            dec!(3.75),
            &[],
            &cfg(),
        )
        .unwrap();
        assert_eq!(slots.len(), 1);
        assert_eq!(slots[0].from, datetime!(2026-06-01 12:00 UTC));
        assert_eq!(slots[0].to, datetime!(2026-06-01 12:15 UTC));
        assert_eq!(slots[0].value, dec!(3.75));
    }

    /// A span wholly inside one slot was never divided, so the slot's value is
    /// the session total unchanged and nothing about it is an estimate — even
    /// though no register was read on the slot's own boundaries. What the flag
    /// reports is whether the *value* was inferred, not whether a reading
    /// happened to land on a quarter-hour.
    #[test]
    fn a_span_wholly_inside_one_slot_is_measured() {
        let slots = split_session(
            datetime!(2026-06-01 12:02 UTC),
            datetime!(2026-06-01 12:09 UTC),
            dec!(3.75),
            &[],
            &cfg(),
        )
        .unwrap();
        assert_eq!(slots[0].quality, QualityFlag::Measured);
    }

    /// ...and the moment the span crosses a boundary, every slot it touches is
    /// an estimate, because the division across that boundary assumed a
    /// constant power the session never promised.
    #[test]
    fn a_span_that_crosses_a_boundary_is_estimated_on_both_sides() {
        let slots = split_session(
            datetime!(2026-06-01 12:10 UTC),
            datetime!(2026-06-01 12:20 UTC),
            dec!(2),
            &[],
            &cfg(),
        )
        .unwrap();
        assert_eq!(slots.len(), 2);
        assert!(slots.iter().all(|s| s.quality == QualityFlag::Estimated));
        assert_eq!(slots[0].value, dec!(1));
        assert_eq!(slots[1].value, dec!(1));
    }

    #[test]
    fn samples_on_both_ends_make_every_whole_slot_measured() {
        let samples = [
            MeterSample::new(datetime!(2026-06-01 12:00 UTC), dec!(100)),
            MeterSample::new(datetime!(2026-06-01 12:15 UTC), dec!(103)),
            MeterSample::new(datetime!(2026-06-01 12:30 UTC), dec!(110)),
        ];
        let slots = split_session(
            datetime!(2026-06-01 12:00 UTC),
            datetime!(2026-06-01 12:30 UTC),
            dec!(10),
            &samples,
            &cfg(),
        )
        .unwrap();
        assert_eq!(slots.len(), 2);
        assert_eq!(slots[0].value, dec!(3));
        assert_eq!(slots[1].value, dec!(7));
        assert!(slots.iter().all(|s| s.quality == QualityFlag::Measured));
        assert_eq!(total(&slots), dec!(10));
    }

    #[test]
    fn an_untouched_slot_inside_the_span_comes_back_as_a_zero() {
        // Nothing flowed between 12:15 and 12:45: the register stands still.
        let samples = [
            MeterSample::new(datetime!(2026-06-01 12:00 UTC), dec!(0)),
            MeterSample::new(datetime!(2026-06-01 12:15 UTC), dec!(4)),
            MeterSample::new(datetime!(2026-06-01 12:45 UTC), dec!(4)),
            MeterSample::new(datetime!(2026-06-01 13:00 UTC), dec!(6)),
        ];
        let slots = split_session(
            datetime!(2026-06-01 12:00 UTC),
            datetime!(2026-06-01 13:00 UTC),
            dec!(6),
            &samples,
            &cfg(),
        )
        .unwrap();
        assert_eq!(slots.len(), 4);
        assert_eq!(
            slots.iter().map(|s| s.value).collect::<Vec<_>>(),
            vec![dec!(4), dec!(0), dec!(0), dec!(2)]
        );
        assert_eq!(total(&slots), dec!(6));
    }

    /// A repeating quotient is where a naive "sum the slots" loses energy.
    #[test]
    fn a_repeating_quotient_still_conserves_the_total() {
        let slots = split_session(
            datetime!(2026-06-01 12:00 UTC),
            datetime!(2026-06-01 12:45 UTC),
            dec!(1),
            &[],
            &cfg(),
        )
        .unwrap();
        assert_eq!(slots.len(), 3);
        assert_eq!(total(&slots), dec!(1));
    }

    /// Berlin's long day has 100 quarter-hours, and a session that runs the
    /// whole of it must produce exactly that many.
    #[test]
    fn the_autumn_long_day_gets_a_hundred_slots() {
        let day = date!(2026 - 10 - 25);
        let slots = split_session(
            calendar::day_start_utc(day),
            calendar::day_end_utc(day),
            dec!(100),
            &[],
            &cfg(),
        )
        .unwrap();
        assert_eq!(slots.len(), 100);
        assert_eq!(total(&slots), dec!(100));
        assert!(slots.iter().all(|s| s.value == dec!(1)));
    }

    #[test]
    fn the_spring_short_day_gets_ninety_two_slots() {
        let day = date!(2026 - 03 - 29);
        let slots = split_session(
            calendar::day_start_utc(day),
            calendar::day_end_utc(day),
            dec!(92),
            &[],
            &cfg(),
        )
        .unwrap();
        assert_eq!(slots.len(), 92);
        assert_eq!(total(&slots), dec!(92));
    }

    /// A daily grid on the Gastag cuts at 06:00, so a session running from
    /// Monday noon to Tuesday noon straddles exactly two gas days.
    #[test]
    fn a_gastag_grid_cuts_at_six() {
        let slots = split_session(
            datetime!(2026-01-05 11:00 UTC),
            datetime!(2026-01-06 11:00 UTC),
            dec!(24),
            &[],
            &cfg().at(IntervalResolution::Day).on(DayBoundary::Gastag),
        )
        .unwrap();
        assert_eq!(slots.len(), 2);
        assert_eq!(
            slots[0].from,
            calendar::gas_day_start_utc(date!(2026 - 01 - 05))
        );
        assert_eq!(
            slots[1].from,
            calendar::gas_day_start_utc(date!(2026 - 01 - 06))
        );
        assert_eq!(total(&slots), dec!(24));
    }

    #[test]
    fn the_obis_channel_is_stamped_and_gives_the_direction() {
        let slots = split_session(
            datetime!(2026-06-01 12:00 UTC),
            datetime!(2026-06-01 12:15 UTC),
            dec!(1),
            &[],
            &cfg().with_obis("1-0:2.8.0".parse().unwrap()),
        )
        .unwrap();
        assert_eq!(
            slots[0].direction(),
            Some(crate::interval::Direction::Export)
        );
    }

    #[test]
    fn an_empty_span_is_an_error() {
        let t = datetime!(2026-06-01 12:00 UTC);
        assert_eq!(
            split_session(t, t, dec!(1), &[], &cfg()),
            Err(SessionError::EmptySpan { from: t, to: t })
        );
    }

    #[test]
    fn a_negative_total_is_an_error() {
        assert!(matches!(
            split_session(
                datetime!(2026-06-01 12:00 UTC),
                datetime!(2026-06-01 12:15 UTC),
                dec!(-1),
                &[],
                &cfg(),
            ),
            Err(SessionError::NegativeEnergy { .. })
        ));
    }

    #[test]
    fn a_sample_outside_the_span_is_an_error() {
        assert!(matches!(
            split_session(
                datetime!(2026-06-01 12:00 UTC),
                datetime!(2026-06-01 12:15 UTC),
                dec!(1),
                &[MeterSample::new(datetime!(2026-06-01 13:00 UTC), dec!(1))],
                &cfg(),
            ),
            Err(SessionError::SampleOutsideSpan { .. })
        ));
    }

    #[test]
    fn a_register_that_runs_backwards_is_an_error() {
        assert!(matches!(
            split_session(
                datetime!(2026-06-01 12:00 UTC),
                datetime!(2026-06-01 12:30 UTC),
                dec!(1),
                &[
                    MeterSample::new(datetime!(2026-06-01 12:05 UTC), dec!(9)),
                    MeterSample::new(datetime!(2026-06-01 12:10 UTC), dec!(8)),
                ],
                &cfg(),
            ),
            Err(SessionError::SamplesNotMonotonic { .. })
        ));
    }

    #[test]
    fn samples_that_exceed_the_session_total_are_an_error() {
        assert!(matches!(
            split_session(
                datetime!(2026-06-01 12:00 UTC),
                datetime!(2026-06-01 12:30 UTC),
                dec!(1),
                &[
                    MeterSample::new(datetime!(2026-06-01 12:05 UTC), dec!(0)),
                    MeterSample::new(datetime!(2026-06-01 12:10 UTC), dec!(8)),
                ],
                &cfg(),
            ),
            Err(SessionError::SampleTotalMismatch { .. })
        ));
    }

    /// Samples pinned to both ends leave nowhere for a discrepancy, so one is
    /// reported rather than absorbed into the last slot.
    #[test]
    fn a_discrepancy_with_no_uncovered_end_is_an_error() {
        assert!(matches!(
            split_session(
                datetime!(2026-06-01 12:00 UTC),
                datetime!(2026-06-01 12:30 UTC),
                dec!(10),
                &[
                    MeterSample::new(datetime!(2026-06-01 12:00 UTC), dec!(0)),
                    MeterSample::new(datetime!(2026-06-01 12:30 UTC), dec!(8)),
                ],
                &cfg(),
            ),
            Err(SessionError::SampleTotalMismatch { .. })
        ));
    }

    #[test]
    fn samples_may_arrive_in_any_order() {
        let mut samples = vec![
            MeterSample::new(datetime!(2026-06-01 12:30 UTC), dec!(110)),
            MeterSample::new(datetime!(2026-06-01 12:00 UTC), dec!(100)),
            MeterSample::new(datetime!(2026-06-01 12:15 UTC), dec!(103)),
        ];
        let forward = split_session(
            datetime!(2026-06-01 12:00 UTC),
            datetime!(2026-06-01 12:30 UTC),
            dec!(10),
            &samples,
            &cfg(),
        )
        .unwrap();
        samples.reverse();
        let backward = split_session(
            datetime!(2026-06-01 12:00 UTC),
            datetime!(2026-06-01 12:30 UTC),
            dec!(10),
            &samples,
            &cfg(),
        )
        .unwrap();
        assert_eq!(forward, backward);
    }

    /// Two readings at one instant describe a step. The later one is the state
    /// of the register from then on, and the step joins the segment before it.
    #[test]
    fn two_readings_at_one_instant_do_not_divide_by_zero() {
        let slots = split_session(
            datetime!(2026-06-01 12:00 UTC),
            datetime!(2026-06-01 12:30 UTC),
            dec!(10),
            &[
                MeterSample::new(datetime!(2026-06-01 12:00 UTC), dec!(0)),
                MeterSample::new(datetime!(2026-06-01 12:15 UTC), dec!(3)),
                MeterSample::new(datetime!(2026-06-01 12:15 UTC), dec!(4)),
                MeterSample::new(datetime!(2026-06-01 12:30 UTC), dec!(10)),
            ],
            &cfg(),
        )
        .unwrap();
        assert_eq!(slots[0].value, dec!(4));
        assert_eq!(slots[1].value, dec!(6));
        assert_eq!(total(&slots), dec!(10));
    }

    #[test]
    fn no_slot_is_ever_negative() {
        let slots = split_session(
            datetime!(2026-06-01 12:07 UTC),
            datetime!(2026-06-01 14:23 UTC),
            dec!(37.771),
            &[MeterSample::new(datetime!(2026-06-01 13:11 UTC), dec!(5.5))],
            &cfg(),
        )
        .unwrap();
        assert!(slots.iter().all(|s| s.value >= Decimal::ZERO));
        assert_eq!(total(&slots), dec!(37.771));
    }

    #[test]
    fn merging_keeps_import_and_export_apart() {
        let cfg = SessionSplitConfig::quarter_hourly();
        let charge = split_session(
            datetime!(2026-06-01 12:00 UTC),
            datetime!(2026-06-01 12:15 UTC),
            dec!(5),
            &[],
            &cfg.with_obis("1-0:1.8.0".parse().unwrap()),
        )
        .unwrap();
        let discharge = split_session(
            datetime!(2026-06-01 12:00 UTC),
            datetime!(2026-06-01 12:15 UTC),
            dec!(2),
            &[],
            &cfg.with_obis("1-0:2.8.0".parse().unwrap()),
        )
        .unwrap();

        let merged = merge_sessions(&[charge, discharge]);
        assert_eq!(merged.len(), 2, "one slot, two registers");
        let balance = crate::aggregation::sum_by_direction(&merged);
        assert_eq!(balance.import, dec!(5));
        assert_eq!(balance.export, dec!(2));
        assert_eq!(balance.net(), dec!(3));
    }

    #[test]
    fn merging_takes_the_worst_quality_and_does_not_depend_on_order() {
        let cfg = SessionSplitConfig::quarter_hourly();
        let measured = split_session(
            datetime!(2026-06-01 12:00 UTC),
            datetime!(2026-06-01 12:15 UTC),
            dec!(3),
            &[
                MeterSample::new(datetime!(2026-06-01 12:00 UTC), dec!(0)),
                MeterSample::new(datetime!(2026-06-01 12:15 UTC), dec!(3)),
            ],
            &cfg,
        )
        .unwrap();
        assert_eq!(measured[0].quality, QualityFlag::Measured);

        let estimated = split_session(
            datetime!(2026-06-01 12:05 UTC),
            datetime!(2026-06-01 12:20 UTC),
            dec!(3),
            &[],
            &cfg,
        )
        .unwrap();

        let forward = merge_sessions(&[measured.clone(), estimated.clone()]);
        let backward = merge_sessions(&[estimated, measured]);
        assert_eq!(forward, backward);
        assert!(forward.iter().all(|s| s.quality == QualityFlag::Estimated));
    }

    #[test]
    fn merging_nothing_is_nothing() {
        assert!(merge_sessions(&[]).is_empty());
        assert!(merge_sessions(&[Vec::new(), Vec::new()]).is_empty());
    }

    /// A long log sampled far more often than the grid: the cursor walks both
    /// lists once, and the answer is the same one a rescan would give.
    #[test]
    fn a_densely_sampled_day_still_conserves_and_stays_ordered() {
        let day = date!(2026 - 06 - 15);
        let start = calendar::day_start_utc(day);
        let samples: Vec<MeterSample> = (1..1440)
            .map(|m| MeterSample::new(start + Duration::minutes(m), Decimal::from(m)))
            .collect();

        let slots = split_session(
            start,
            calendar::day_end_utc(day),
            dec!(1440),
            &samples,
            &cfg(),
        )
        .unwrap();

        assert_eq!(slots.len(), 96);
        assert_eq!(total(&slots), dec!(1440));
        for pair in slots.windows(2) {
            assert_eq!(pair[0].to, pair[1].from);
        }
        // Every interior quarter-hour is bounded by minute readings, so it is
        // measured — 15 minutes at 1 unit each.
        assert!(slots[1..95].iter().all(|s| s.value == dec!(15)));
        assert!(
            slots[1..95]
                .iter()
                .all(|s| s.quality == QualityFlag::Measured)
        );
    }
}
