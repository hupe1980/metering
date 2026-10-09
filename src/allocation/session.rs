//! Sessions and device logs → Lastgang: a span of register readings placed
//! on the metering grid.
//!
//! A charge point's CDR, a submetered heat pump or a device log reports
//! cumulative readings over a span; [`split_session`] places the energy on
//! grid slots and [`merge_sessions`] adds several sessions on one grid. The
//! readings are the only input, so the total cannot contradict them:
//!
//! ```text
//! Σ slot energy = last reading − first reading        exactly, always
//! ```
//!
//! | Basis | Where it comes from | Quality |
//! |---|---|---|
//! | **Metered** | readings bracketing every segment inside the slot | [`QualityFlag::Measured`] |
//! | **Pro rata** | a segment between two readings straddling a boundary, divided by wall-clock time | [`QualityFlag::Estimated`] |
//!
//! Pro rata is not a profile; supply more [`MeterSample`]s (e.g. OCPP
//! clock-aligned values) for a better shape.
//!
//! Slots come from [`DayBoundary::bucket`], the function
//! [`resample`](crate::series::resample::resample()) buckets with, so DST days
//! and the Gastag are laid out the same way. Dividing one slot among several
//! claims is [`crate::allocation::community::allocate`].

use std::collections::BTreeMap;

use rust_decimal::Decimal;
use time::OffsetDateTime;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use crate::ids::obis::ObisCode;
use crate::precision::{ALLOCATION_DP, ALLOCATION_STRATEGY};
use crate::series::interval::{MeterInterval, QualityFlag};
use crate::series::{Series, SeriesError};
use crate::time::calendar::DayBoundary;
use crate::time::resolution::Resolution;

// ── MeterSample ───────────────────────────────────────────────────────────────

/// A register reading taken during a session.
///
/// `reading` is **cumulative** (as OCPP `Energy.Active.Import.Register`), not
/// the energy since the last sample; only differences are used, so the offset
/// is irrelevant and a missing sample widens one segment. Unit: the series'
/// own — kWh for Strom (OCPP reports Wh; convert first).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct MeterSample {
    /// When the register was read (UTC).
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::rfc3339"))]
    pub at: OffsetDateTime,
    /// The cumulative register value at that instant.
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::decimal"))]
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
    /// The slot length.
    pub resolution: Resolution,
    /// Where a day is cut; every slot is laid out from the local day start.
    pub day_boundary: DayBoundary,
    /// The OBIS channel stamped on every emitted interval, so
    /// [`MeterInterval::direction`] answers — `1-0:1.8.0` for a charge,
    /// `1-0:2.8.0` for a V2G discharge.
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
            resolution: Resolution::QUARTER_HOUR,
            day_boundary: DayBoundary::Strom,
            obis_code: None,
            metered_quality: QualityFlag::Measured,
            prorated_quality: QualityFlag::Estimated,
        }
    }

    /// A different slot length (builder style).
    #[must_use]
    pub const fn at(mut self, resolution: Resolution) -> Self {
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
    /// [`Measured`](QualityFlag::Measured) (builder style) — e.g.
    /// [`Calculated`](QualityFlag::Calculated) for a device log that is not an
    /// eichrechtskonform measurement.
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
#[derive(Debug, thiserror::Error, PartialEq, Eq, Clone)]
#[non_exhaustive]
pub enum SessionError {
    /// Fewer than two distinct instants: no span, so no energy.
    #[error("a session needs readings at two instants at least")]
    TooFewSamples,

    /// A register reading went backwards. A V2G discharge is its own session
    /// on the export register ([`SessionSplitConfig::obis_code`]).
    #[error("register reading at {at} is below the previous one")]
    SamplesNotMonotonic {
        /// The instant at which the register decreased.
        at: OffsetDateTime,
    },

    /// Two different readings at one instant.
    #[error("two different readings at {at}")]
    ConflictingSamples {
        /// The instant.
        at: OffsetDateTime,
    },

    /// A slot lies outside the calendar's supported years.
    #[error("the slot at {at} lies outside the supported calendar years")]
    OutOfRange {
        /// The instant.
        at: OffsetDateTime,
    },

    /// [`merge_sessions`] was given a series on another grid or channel.
    #[error("series {index} is not on the configured grid and channel")]
    GridMismatch {
        /// Its position in the input.
        index: usize,
    },

    /// The result series could not be formed. Unreachable for slots this
    /// module lays out; carried so no path panics.
    #[error(transparent)]
    Series(#[from] SeriesError),
}

// ── split_session ─────────────────────────────────────────────────────────────

/// The span between two adjacent readings: its energy is known exactly.
struct Segment {
    start: OffsetDateTime,
    end: OffsetDateTime,
    energy: Decimal,
    /// Energy delivered *before* this segment began.
    cum_before: Decimal,
}

/// Place a session, given as register readings, on the metering grid.
///
/// `samples` are cumulative readings in any order. One interval per slot the
/// span touches, contiguous, each spanning its **whole** slot; a touched slot
/// with no energy is a zero, not a gap. Each slot is the difference of two
/// adjacent cumulatives cut to [`ALLOCATION_DP`] toward zero, so
/// `Σ slot = last − first` exactly and no slot is negative.
///
/// A slot is [`metered_quality`](SessionSplitConfig::metered_quality) when
/// no segment reaching into it crosses its boundaries, and
/// [`prorated_quality`](SessionSplitConfig::prorated_quality) otherwise.
///
/// # Errors
///
/// [`TooFewSamples`](SessionError::TooFewSamples),
/// [`SamplesNotMonotonic`](SessionError::SamplesNotMonotonic),
/// [`ConflictingSamples`](SessionError::ConflictingSamples) and
/// [`OutOfRange`](SessionError::OutOfRange).
///
/// ```rust
/// use metering::allocation::session::{MeterSample, SessionSplitConfig, split_session};
/// use metering::QualityFlag;
/// use rust_decimal::dec;
/// use time::macros::datetime;
///
/// // Meter start 12:07, clock-aligned values at 12:15 and 12:30, stop 12:37.
/// let samples = [
///     MeterSample::new(datetime!(2026-06-01 12:07 UTC), dec!(998)),
///     MeterSample::new(datetime!(2026-06-01 12:15 UTC), dec!(1000)),
///     MeterSample::new(datetime!(2026-06-01 12:30 UTC), dec!(1006)),
///     MeterSample::new(datetime!(2026-06-01 12:37 UTC), dec!(1008)),
/// ];
/// let series = split_session(&samples, &SessionSplitConfig::quarter_hourly())?;
/// let slots = series.as_slice();
///
/// // Every segment lies inside one slot, so every slot is measured.
/// assert_eq!(slots[1].value(), dec!(6));
/// assert!(slots.iter().all(|s| s.quality() == QualityFlag::Measured));
/// assert_eq!(series.billable_total(), Some(dec!(10)));
/// # Ok::<(), metering::allocation::session::SessionError>(())
/// ```
pub fn split_session(
    samples: &[MeterSample],
    config: &SessionSplitConfig,
) -> Result<Series, SessionError> {
    let segments = build_segments(samples)?;
    let (Some(first), Some(last)) = (segments.first(), segments.last()) else {
        return Err(SessionError::TooFewSamples);
    };
    let (from, to) = (first.start, last.end);
    let energy = last.cum_before + last.energy;

    let bucket = |at| {
        config
            .day_boundary
            .bucket(at, config.resolution)
            .map(|p| p.range())
            .ok_or(SessionError::OutOfRange { at })
    };
    let mut out = Vec::new();
    let mut slot_start = bucket(from)?.0;
    // Segments and slots both run forward: one cursor, one pass.
    let mut cursor = 0usize;
    let mut opening = Decimal::ZERO;

    while slot_start < to {
        let (bucket_from, bucket_to) = bucket(slot_start)?;
        if bucket_to <= bucket_from {
            return Err(SessionError::OutOfRange { at: bucket_from });
        }
        let covered_from = bucket_from.max(from);
        let covered_to = bucket_to.min(to);

        let closing = cumulative(&segments, cursor, from, to, energy, covered_to);
        let exact = segments
            .get(cursor..)
            .unwrap_or_default()
            .iter()
            .take_while(|s| s.start < covered_to)
            .all(|s| s.start >= covered_from && s.end <= covered_to);
        while cursor + 1 < segments.len()
            && segments.get(cursor).is_some_and(|s| s.end <= covered_to)
        {
            cursor += 1;
        }

        let quality = if exact {
            config.metered_quality
        } else {
            config.prorated_quality
        };
        out.push(
            MeterInterval::build(
                bucket_from,
                bucket_to,
                closing - opening,
                quality,
                config.obis_code,
            )
            .map_err(|_| SessionError::OutOfRange { at: bucket_from })?,
        );
        opening = closing;
        slot_start = bucket_to;
    }
    Ok(Series::new(config.resolution, config.day_boundary, out)?)
}

/// Sort the readings, refuse a contradiction, and cut the span into one
/// segment per pair of adjacent readings.
fn build_segments(samples: &[MeterSample]) -> Result<Vec<Segment>, SessionError> {
    let mut sorted: Vec<MeterSample> = samples.to_vec();
    sorted.sort_by_key(|s| s.at);
    let mut distinct: Vec<MeterSample> = Vec::with_capacity(sorted.len());
    for s in sorted {
        match distinct.last() {
            Some(prev) if prev.at == s.at && prev.reading != s.reading => {
                return Err(SessionError::ConflictingSamples { at: s.at });
            }
            Some(prev) if prev.at == s.at => {}
            Some(prev) if s.reading < prev.reading => {
                return Err(SessionError::SamplesNotMonotonic { at: s.at });
            }
            _ => distinct.push(s),
        }
    }
    let base = distinct.first().map_or(Decimal::ZERO, |s| s.reading);
    Ok(distinct
        .windows(2)
        .filter_map(|pair| match pair {
            [a, b] => Some(Segment {
                start: a.at,
                end: b.at,
                energy: b.reading - a.reading,
                cum_before: a.reading - base,
            }),
            _ => None,
        })
        .collect())
}

/// `x` cut to [`ALLOCATION_DP`] toward zero.
fn cut(x: Decimal) -> Decimal {
    x.round_dp_with_strategy(ALLOCATION_DP, ALLOCATION_STRATEGY)
}

/// Energy delivered from the session start up to `t`.
///
/// Pinned to `0` at `from` and the total at `to`; in between, whole segments
/// plus the wall-clock share of the one `t` falls in, cut to
/// [`ALLOCATION_DP`] (truncation keeps it monotone).
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
    // the one containing `t` is at or after it.
    let found = segments
        .get(hint..)
        .unwrap_or_default()
        .iter()
        .find(|s| s.start < t && t <= s.end)
        .or_else(|| segments.iter().find(|s| s.start < t && t <= s.end));
    let Some(seg) = found else {
        // Unreachable (the segments tile `[from, to]`); never return zero, or
        // the next slot would go negative.
        return segments
            .last()
            .map_or(Decimal::ZERO, |s| cut(s.cum_before + s.energy));
    };
    let span = (seg.end - seg.start).whole_seconds();
    if span <= 0 {
        return cut(seg.cum_before + seg.energy);
    }
    let elapsed = (t - seg.start).whole_seconds();
    cut(seg.cum_before + seg.energy * Decimal::from(elapsed) / Decimal::from(span))
}

// ── merge_sessions ────────────────────────────────────────────────────────────

/// Add several sessions on the configured grid, slot by slot.
///
/// A union of slots: an absent slot is zero energy, and only slots something
/// touched appear. Each slot carries the worst quality of its contributors.
///
/// # Errors
///
/// [`GridMismatch`](SessionError::GridMismatch) for a series on another
/// resolution, day boundary or OBIS channel than `config` names.
///
/// ```rust
/// use metering::allocation::session::{MeterSample, SessionSplitConfig, merge_sessions, split_session};
/// use rust_decimal::dec;
/// use time::macros::datetime;
///
/// let cfg = SessionSplitConfig::quarter_hourly();
/// let session = |from, to, kwh| {
///     split_session(&[MeterSample::new(from, dec!(0)), MeterSample::new(to, kwh)], &cfg)
/// };
/// // Two cars, overlapping in the 12:15 slot and nowhere else.
/// let a = session(datetime!(2026-06-01 12:00 UTC), datetime!(2026-06-01 12:30 UTC), dec!(8))?;
/// let b = session(datetime!(2026-06-01 12:15 UTC), datetime!(2026-06-01 12:45 UTC), dec!(4))?;
///
/// let merged = merge_sessions(&cfg, &[a, b])?;
/// assert_eq!(merged.len(), 3, "a union of the slots");
/// assert_eq!(merged.as_slice()[1].value(), dec!(6), "both cars charging");
/// assert_eq!(merged.billable_total(), Some(dec!(12)));
/// # Ok::<(), metering::allocation::session::SessionError>(())
/// ```
pub fn merge_sessions(
    config: &SessionSplitConfig,
    series: &[Series],
) -> Result<Series, SessionError> {
    let mut slots: BTreeMap<OffsetDateTime, (OffsetDateTime, Decimal, QualityFlag)> =
        BTreeMap::new();
    for (index, one) in series.iter().enumerate() {
        if one.resolution() != config.resolution
            || one.boundary() != config.day_boundary
            || (!one.is_empty() && one.channel() != config.obis_code)
        {
            return Err(SessionError::GridMismatch { index });
        }
        for iv in one {
            let entry =
                slots
                    .entry(iv.from())
                    .or_insert((iv.to(), Decimal::ZERO, QualityFlag::Measured));
            entry.1 += iv.value();
            entry.2 = entry.2.worse_of(iv.quality());
        }
    }
    let intervals = slots
        .into_iter()
        .map(|(from, (to, value, quality))| {
            MeterInterval::build(from, to, value, quality, config.obis_code)
                .map_err(|_| SessionError::OutOfRange { at: from })
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Series::new(
        config.resolution,
        config.day_boundary,
        intervals,
    )?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::series::interval::Direction;
    use rust_decimal::dec;
    use time::Duration;
    use time::macros::{date, datetime};

    fn cfg() -> SessionSplitConfig {
        SessionSplitConfig::quarter_hourly()
    }

    fn s(at: OffsetDateTime, reading: Decimal) -> MeterSample {
        MeterSample::new(at, reading)
    }

    fn values(series: &Series) -> Vec<Decimal> {
        series.iter().map(MeterInterval::value).collect()
    }

    #[test]
    fn a_span_inside_one_slot_is_that_slot_and_measured() {
        let out = split_session(
            &[
                s(datetime!(2026-06-01 12:02 UTC), dec!(10)),
                s(datetime!(2026-06-01 12:09 UTC), dec!(13.75)),
            ],
            &cfg(),
        )
        .unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out.as_slice()[0].from(), datetime!(2026-06-01 12:00 UTC));
        assert_eq!(out.as_slice()[0].value(), dec!(3.75));
        assert_eq!(out.as_slice()[0].quality(), QualityFlag::Measured);
    }

    #[test]
    fn a_segment_crossing_a_boundary_is_estimated_on_both_sides() {
        let out = split_session(
            &[
                s(datetime!(2026-06-01 12:10 UTC), dec!(0)),
                s(datetime!(2026-06-01 12:20 UTC), dec!(2)),
            ],
            &cfg(),
        )
        .unwrap();
        assert_eq!(values(&out), vec![dec!(1), dec!(1)]);
        assert!(out.iter().all(|iv| iv.quality() == QualityFlag::Estimated));
    }

    #[test]
    fn a_still_register_gives_zeros_not_gaps() {
        let t = datetime!(2026-06-01 12:00 UTC);
        let out = split_session(
            &[
                s(t, dec!(0)),
                s(t + Duration::minutes(15), dec!(4)),
                s(t + Duration::minutes(45), dec!(4)),
                s(t + Duration::minutes(60), dec!(6)),
            ],
            &cfg(),
        )
        .unwrap();
        assert_eq!(values(&out), vec![dec!(4), dec!(0), dec!(0), dec!(2)]);
        // The 12:15–12:45 segment crosses 12:30, so its two slots are pro rata.
        let flags: Vec<QualityFlag> = out.iter().map(MeterInterval::quality).collect();
        assert_eq!(flags[0], QualityFlag::Measured);
        assert_eq!(flags[1], QualityFlag::Estimated);
    }

    #[test]
    fn a_repeating_quotient_still_conserves_the_total() {
        let t = datetime!(2026-06-01 12:00 UTC);
        let out = split_session(
            &[s(t, dec!(0)), s(t + Duration::minutes(45), dec!(1))],
            &cfg(),
        )
        .unwrap();
        assert_eq!(out.len(), 3);
        assert_eq!(out.billable_total(), Some(dec!(1)));
    }

    #[test]
    fn the_autumn_long_day_gets_a_hundred_slots() {
        let day = DayBoundary::Strom.day(date!(2026 - 10 - 25)).unwrap();
        let out =
            split_session(&[s(day.start(), dec!(0)), s(day.end(), dec!(25))], &cfg()).unwrap();
        assert_eq!(out.len(), 100);
        assert_eq!(out.billable_total(), Some(dec!(25)));
    }

    #[test]
    fn a_gastag_grid_cuts_at_six() {
        let out = split_session(
            &[
                s(datetime!(2026-01-05 11:00 UTC), dec!(0)),
                s(datetime!(2026-01-06 11:00 UTC), dec!(24)),
            ],
            &cfg().at(Resolution::Day).on(DayBoundary::Gas),
        )
        .unwrap();
        assert_eq!(out.len(), 2);
        assert_eq!(
            out.as_slice()[1].from(),
            DayBoundary::Gas.day(date!(2026 - 01 - 06)).unwrap().start()
        );
        assert_eq!(out.billable_total(), Some(dec!(24)));
    }

    #[test]
    fn the_obis_channel_is_stamped_and_gives_the_direction() {
        let t = datetime!(2026-06-01 12:00 UTC);
        let out = split_session(
            &[s(t, dec!(0)), s(t + Duration::minutes(15), dec!(1))],
            &cfg().with_obis(ObisCode::STROM_EINSPEISUNG_LASTGANG),
        )
        .unwrap();
        assert_eq!(out.as_slice()[0].direction(), Some(Direction::Export));
    }

    /// The readings are the only input, so a total cannot contradict them;
    /// what is left to contradict is the readings themselves.
    #[test]
    fn contradictory_readings_are_refused() {
        let t = datetime!(2026-06-01 12:00 UTC);
        assert_eq!(
            split_session(&[s(t, dec!(1))], &cfg()),
            Err(SessionError::TooFewSamples)
        );
        assert_eq!(
            split_session(&[s(t, dec!(1)), s(t, dec!(1))], &cfg()),
            Err(SessionError::TooFewSamples)
        );
        let later = t + Duration::minutes(5);
        assert_eq!(
            split_session(&[s(t, dec!(5)), s(later, dec!(4))], &cfg()),
            Err(SessionError::SamplesNotMonotonic { at: later })
        );
        // Two readings at one instant: refused in either order.
        let a = [s(t, dec!(0)), s(later, dec!(1)), s(later, dec!(2))];
        let mut b = a;
        b.swap(1, 2);
        assert_eq!(
            split_session(&a, &cfg()),
            Err(SessionError::ConflictingSamples { at: later })
        );
        assert_eq!(
            split_session(&b, &cfg()),
            Err(SessionError::ConflictingSamples { at: later })
        );
    }

    #[test]
    fn merging_keeps_channels_apart_and_takes_the_worst_quality() {
        let t = datetime!(2026-06-01 12:00 UTC);
        let one = |q| {
            split_session(
                &[s(t, dec!(0)), s(t + Duration::minutes(15), dec!(2))],
                &cfg().metered_as(q),
            )
            .unwrap()
        };
        let merged = merge_sessions(
            &cfg(),
            &[one(QualityFlag::Measured), one(QualityFlag::Estimated)],
        )
        .unwrap();
        assert_eq!(values(&merged), vec![dec!(4)]);
        assert_eq!(merged.as_slice()[0].quality(), QualityFlag::Estimated);

        let export = split_session(
            &[s(t, dec!(0)), s(t + Duration::minutes(15), dec!(1))],
            &cfg().with_obis(ObisCode::STROM_EINSPEISUNG_LASTGANG),
        )
        .unwrap();
        assert_eq!(
            merge_sessions(&cfg(), &[one(QualityFlag::Measured), export]),
            Err(SessionError::GridMismatch { index: 1 })
        );
        assert!(merge_sessions(&cfg(), &[]).unwrap().is_empty());
    }

    #[test]
    fn a_minute_log_is_measured_in_every_whole_slot() {
        let day = DayBoundary::Strom.day(date!(2026 - 06 - 15)).unwrap();
        let samples: Vec<MeterSample> = (0..=1440)
            .map(|m| s(day.start() + Duration::minutes(m), Decimal::from(m)))
            .collect();
        let out = split_session(&samples, &cfg()).unwrap();
        assert_eq!(out.len(), 96);
        assert!(
            out.iter()
                .all(|iv| iv.value() == dec!(15) && iv.quality() == QualityFlag::Measured)
        );
    }
}
