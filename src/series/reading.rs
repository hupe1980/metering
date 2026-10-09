//! Zählerstände and the Zählerstandsgang → Lastgang conversion
//! ([`to_lastgang`]; two readings: [`consumption_between`]).
//!
//! A register holds a cumulative **Zählerstand**; the interval energy
//! ([`MeterInterval`]) is the difference of two. Under BNetzA **BK6-24-174**
//! (*"Datenübermittlung ZSG"*) the MSB takes that difference. § 2 Satz 1
//! Nr. 27 MsbG defines the input: *"die Messung einer Reihe
//! **viertelstündig ermittelter Zählerstände** von elektrischer Arbeit und
//! **stündlich ermittelter Zählerstände** von Gasmengen"*.
//!
//! Register wraps (Überlauf) are reconstructed here, where readings live
//! ([`Rollover`]). Where a difference cannot be taken honestly, **no interval**
//! is emitted and an [`Anomaly`] is recorded instead; the hole is then a gap
//! for [`crate::vee::validation`] and [`crate::vee::substitute`].
//!
//! ```rust
//! use metering::series::reading::{LastgangConfig, MeterReading, to_lastgang};
//! use metering::QualityFlag;
//! use rust_decimal::dec;
//! use time::macros::datetime;
//!
//! // Four quarter-hourly Zählerstände.
//! let zsg: Vec<MeterReading> = [dec!(1000.0), dec!(1002.5), dec!(1004.8), dec!(1007.0)]
//!     .into_iter()
//!     .enumerate()
//!     .map(|(i, value)| MeterReading {
//!         at: datetime!(2026-06-01 0:00 UTC) + time::Duration::minutes(i as i64 * 15),
//!         value,
//!         quality: QualityFlag::Measured,
//!         obis_code: None,
//!     })
//!     .collect();
//!
//! let channels = to_lastgang(&zsg, &LastgangConfig::strom());
//! assert_eq!(channels.len(), 1, "one register, one Lastgang");
//! let lastgang = &channels[0];
//! assert_eq!(lastgang.intervals.len(), 3, "n readings give n−1 intervals");
//! assert_eq!(lastgang.intervals[0].value(), dec!(2.5));
//! assert_eq!(lastgang.intervals[1].value(), dec!(2.3));
//! assert!(lastgang.is_clean());
//! ```

use rust_decimal::Decimal;
use time::OffsetDateTime;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use crate::ids::obis::ObisCode;
use crate::series::interval::{MeterInterval, QualityFlag};
use crate::series::{Series, SeriesError};
use crate::time::calendar::DayBoundary;
use crate::time::resolution::Resolution;

/// The longest a period of `resolution` can be, in seconds — for a ceiling,
/// where erring long is safe.
const fn longest_seconds(resolution: Resolution) -> i64 {
    match resolution {
        Resolution::Day => 25 * 3600,
        Resolution::Month => 31 * 86_400 + 3600,
        Resolution::Year => 366 * 86_400,
        fixed => match fixed.fixed_seconds() {
            Some(s) => s as i64,
            // Unreachable: only the three calendar arms answer `None`.
            None => 0,
        },
    }
}

/// A cumulative meter reading (Zählerstand) at one instant, in the register's
/// unit ([`crate::Sparte::measured_unit`]); differencing is unit-agnostic.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct MeterReading {
    /// When the register held this value (UTC).
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::rfc3339"))]
    pub at: OffsetDateTime,
    /// The register value. Cumulative and, absent a rollover, non-decreasing.
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::decimal"))]
    pub value: Decimal,
    /// Quality of this reading.
    pub quality: QualityFlag,
    /// OBIS code of the register.
    pub obis_code: Option<ObisCode>,
}

impl MeterReading {
    /// A measured reading with no OBIS code.
    #[must_use]
    pub const fn measured(at: OffsetDateTime, value: Decimal) -> Self {
        Self {
            at,
            value,
            quality: QualityFlag::Measured,
            obis_code: None,
        }
    }

    /// Attach an OBIS code (builder style).
    #[must_use]
    pub const fn with_obis(mut self, code: ObisCode) -> Self {
        self.obis_code = Some(code);
        self
    }
}

/// A register wrap, reconstructed from the register width:
/// `delta = (10^digits − previous) + current`, accepted only within
/// [`LastgangConfig::max_delta`].
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[non_exhaustive]
pub struct Rollover {
    /// Start of the span the wrap happened in (the earlier reading).
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::rfc3339"))]
    pub from: OffsetDateTime,
    /// End of it — the reading at which the wrap became visible.
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::rfc3339"))]
    pub to: OffsetDateTime,
    /// Register value before the wrap.
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::decimal"))]
    pub previous: Decimal,
    /// Register value after it.
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::decimal"))]
    pub current: Decimal,
    /// The register's capacity, `10^digits`.
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::decimal"))]
    pub register_capacity: Decimal,
    /// Consumption across the wrap.
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::decimal"))]
    pub delta: Decimal,
}

impl Rollover {
    /// How long the span lasted.
    #[must_use]
    pub fn duration(&self) -> time::Duration {
        self.to - self.from
    }
}

/// A pair of readings that no honest difference could be taken from; the
/// corresponding interval is **absent** from [`Lastgang::intervals`].
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[non_exhaustive]
pub struct Anomaly {
    /// Start of the affected span (the earlier reading).
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::rfc3339"))]
    pub from: OffsetDateTime,
    /// End of it (the later reading).
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::rfc3339"))]
    pub to: OffsetDateTime,
    /// Why the difference was refused.
    pub kind: AnomalyKind,
    /// The earlier register value.
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::decimal"))]
    pub previous: Decimal,
    /// The later register value.
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::decimal"))]
    pub current: Decimal,
}

/// Why a difference between two readings was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum AnomalyKind {
    /// The register went backwards and no (usable) register width was
    /// configured to reconstruct a wrap.
    BackwardsWithoutRegisterWidth,
    /// The register went backwards and no plausible wrap explains it: the
    /// reconstructed delta exceeds [`LastgangConfig::max_delta`], or the
    /// earlier reading is at or above the register capacity.
    ImplausibleRollover,
    /// The forward difference exceeds [`LastgangConfig::max_delta`] (or
    /// overflows).
    ImplausibleDelta,
    /// The two readings carry the same timestamp.
    ZeroLengthSpan,
    /// One of the two readings is not billable.
    NonBillableEndpoint,
}

impl AnomalyKind {
    /// Every kind, in declaration order.
    pub const ALL: [Self; 5] = [
        Self::BackwardsWithoutRegisterWidth,
        Self::ImplausibleRollover,
        Self::ImplausibleDelta,
        Self::ZeroLengthSpan,
        Self::NonBillableEndpoint,
    ];

    /// Stable DB/wire label. Matches the `serde` tag and
    /// [`FromStr`](std::str::FromStr) input — an audit code (§ 146 Abs. 4 AO).
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::BackwardsWithoutRegisterWidth => "BACKWARDS_WITHOUT_REGISTER_WIDTH",
            Self::ImplausibleRollover => "IMPLAUSIBLE_ROLLOVER",
            Self::ImplausibleDelta => "IMPLAUSIBLE_DELTA",
            Self::ZeroLengthSpan => "ZERO_LENGTH_SPAN",
            Self::NonBillableEndpoint => "NON_BILLABLE_ENDPOINT",
        }
    }

    /// A short explanation, for a log line or an operator UI.
    #[must_use]
    pub const fn description(self) -> &'static str {
        match self {
            Self::BackwardsWithoutRegisterWidth => {
                "register decreased and no register width was configured to explain a wrap"
            }
            Self::ImplausibleRollover => {
                "register decreased, but reconstructing a wrap implies an implausible consumption"
            }
            Self::ImplausibleDelta => "the forward difference exceeds the plausible maximum",
            Self::ZeroLengthSpan => "two readings share a timestamp, so there is no span",
            Self::NonBillableEndpoint => "one endpoint is not billable",
        }
    }
}

crate::ids::codes::string_codes! {
    AnomalyKind;
}

/// How to turn a Zählerstandsgang into a Lastgang; build with
/// [`strom`](Self::strom) or [`default`](Self::default) and the setters.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct LastgangConfig {
    register_digits: Option<u32>,
    max_delta: Option<Decimal>,
    result_channel: ResultChannel,
}

/// What OBIS code [`to_lastgang`] stamps on the intervals it derives.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "SCREAMING_SNAKE_CASE"))]
#[non_exhaustive]
pub enum ResultChannel {
    /// Carry each reading's own (Zählerstand) OBIS code through unchanged.
    #[default]
    Unchanged,

    /// Relabel each interval as the Lastgang of its register
    /// ([`ObisCode::as_lastgang`]: `1-0:1.8.0` → `1-0:1.29.0`); a code without
    /// one keeps its own.
    Derived,

    /// One fixed code on every derived interval, for unlabelled readings. On a
    /// labelled feed-in series it would mislabel the direction; prefer
    /// [`Derived`](Self::Derived) there.
    Fixed(ObisCode),
}

impl ResultChannel {
    /// The code to stamp on an interval derived from a reading labelled
    /// `source`.
    #[must_use]
    pub fn label(self, source: Option<ObisCode>) -> Option<ObisCode> {
        match self {
            Self::Unchanged => source,
            Self::Derived => source.map(|c| c.as_lastgang().unwrap_or(c)),
            Self::Fixed(code) => Some(code),
        }
    }
}

impl Default for LastgangConfig {
    /// No wrap reconstruction, no plausibility cap, and
    /// [`ResultChannel::Unchanged`].
    fn default() -> Self {
        Self {
            register_digits: None,
            max_delta: None,
            result_channel: ResultChannel::Unchanged,
        }
    }
}

impl LastgangConfig {
    /// Electricity: [`ResultChannel::Derived`], no register width and no delta
    /// cap — add those per device with
    /// [`register_digits`](Self::register_digits) and
    /// [`capacity_kw`](Self::capacity_kw).
    #[must_use]
    pub const fn strom() -> Self {
        Self {
            register_digits: None,
            max_delta: None,
            result_channel: ResultChannel::Derived,
        }
    }

    /// Set the register width — digits before the point, so it wraps at
    /// `10^digits` — enabling wrap reconstruction. A width over 28 disables it.
    ///
    /// Leave it unset unless known: a wrong width turns a meter exchange into
    /// consumption.
    #[must_use]
    pub const fn register_digits(mut self, digits: u32) -> Self {
        self.register_digits = Some(digits);
        self
    }

    /// Set the largest plausible difference between consecutive readings, per
    /// reading interval (7.5 kWh for a quarter-hourly ZSG on 30 kW). It is what
    /// tells a wrap from a meter exchange.
    #[must_use]
    pub const fn max_delta(mut self, max: Decimal) -> Self {
        self.max_delta = Some(max);
        self
    }

    /// Derive the delta cap as `capacity_kw × cadence hours` — the ceiling the
    /// capacity rule of [`crate::vee::validation::Rules`] flags, applied before
    /// the value exists.
    ///
    /// A calendar `cadence` uses its longest period (25 h for a day, 31 d + 1 h
    /// for a month, 366 d for a year); [`detect_reading_cadence`] supplies it.
    /// `None` on overflow.
    #[must_use]
    pub fn capacity_kw(mut self, capacity_kw: Decimal, cadence: Resolution) -> Option<Self> {
        // Multiply first: one rounding, at the end.
        let max = capacity_kw
            .checked_mul(Decimal::from(longest_seconds(cadence)))?
            .checked_div(Decimal::from(3600u32))?;
        self.max_delta = Some(max);
        Some(self)
    }

    /// Label the derived intervals with one fixed `code`
    /// ([`ResultChannel::Fixed`]).
    #[must_use]
    pub const fn label(mut self, code: ObisCode) -> Self {
        self.result_channel = ResultChannel::Fixed(code);
        self
    }

    /// Set how derived intervals are labelled.
    #[must_use]
    pub const fn channel(mut self, channel: ResultChannel) -> Self {
        self.result_channel = channel;
        self
    }

    /// The register capacity, `10^digits`; `None` without a width or for one
    /// over 28 (beyond `Decimal`), which disables wrap reconstruction.
    fn capacity(&self) -> Option<Decimal> {
        let digits = self.register_digits?;
        if digits > 28 {
            return None;
        }
        // Not `Decimal::powu`: that needs rust_decimal's `maths` feature.
        (0..digits).try_fold(Decimal::ONE, |acc, _| acc.checked_mul(Decimal::TEN))
    }
}

/// The result of differencing the Zählerstandsgang of **one register**.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct Lastgang {
    /// The readings' OBIS code; the intervals may carry another
    /// ([`ResultChannel`]).
    pub register: Option<ObisCode>,
    /// The derived intervals, ascending — one per usable consecutive pair.
    pub intervals: Vec<MeterInterval>,
    /// Reconstructed register wraps; their intervals **are** in
    /// [`intervals`](Self::intervals).
    pub rollovers: Vec<Rollover>,
    /// Pairs no difference could be taken from; their intervals are **absent**.
    pub anomalies: Vec<Anomaly>,
}

impl Lastgang {
    /// `true` when every consecutive pair yielded an interval and none needed a
    /// wrap reconstructed.
    #[must_use]
    pub fn is_clean(&self) -> bool {
        self.anomalies.is_empty() && self.rollovers.is_empty()
    }

    /// Total across the derived intervals — last minus first reading only when
    /// there are no anomalies. `None` on overflow.
    #[must_use]
    pub fn total(&self) -> Option<Decimal> {
        self.intervals
            .iter()
            .try_fold(Decimal::ZERO, |sum, iv| sum.checked_add(iv.value()))
    }

    /// The intervals as a validated [`Series`] on `resolution`.
    ///
    /// # Errors
    ///
    /// A [`SeriesError`] when the readings were not taken on that grid.
    pub fn to_series(
        &self,
        resolution: Resolution,
        boundary: DayBoundary,
    ) -> Result<Series, SeriesError> {
        Series::new(resolution, boundary, self.intervals.clone())
    }
}

/// Difference a Zählerstandsgang into one Lastgang **per register**.
///
/// Readings are grouped by OBIS code and sorted by timestamp; the result is
/// ordered by register. Each interval takes the **worse** endpoint quality
/// ([`QualityFlag::worse_of`]); a refused pair yields an [`Anomaly`] instead.
///
/// ## Example — a six-digit register wrapping
///
/// ```rust
/// use metering::series::reading::{LastgangConfig, MeterReading, to_lastgang};
/// use rust_decimal::dec;
/// use time::macros::datetime;
///
/// let zsg = vec![
///     MeterReading::measured(datetime!(2026-06-01 0:00 UTC), dec!(999998.5)),
///     MeterReading::measured(datetime!(2026-06-01 0:15 UTC), dec!(1.5)), // wrapped
/// ];
///
/// // Without a register width the drop is an anomaly.
/// let blind = &to_lastgang(&zsg, &LastgangConfig::strom())[0];
/// assert!(blind.intervals.is_empty());
/// assert_eq!(blind.anomalies.len(), 1);
///
/// // With one: (1 000 000 − 999 998.5) + 1.5 = 3.
/// let cfg = LastgangConfig::strom().register_digits(6);
/// let wrapped = &to_lastgang(&zsg, &cfg)[0];
/// assert_eq!(wrapped.intervals[0].value(), dec!(3.0));
/// assert_eq!(wrapped.rollovers.len(), 1);
/// assert!(wrapped.anomalies.is_empty());
/// ```
#[must_use]
pub fn to_lastgang(readings: &[MeterReading], config: &LastgangConfig) -> Vec<Lastgang> {
    let mut registers: std::collections::BTreeMap<Option<ObisCode>, Vec<&MeterReading>> =
        std::collections::BTreeMap::new();
    for r in readings {
        registers.entry(r.obis_code).or_default().push(r);
    }
    registers
        .into_iter()
        .map(|(register, ordered)| difference(register, ordered, config))
        .collect()
}

/// Difference one register's readings.
fn difference(
    register: Option<ObisCode>,
    mut ordered: Vec<&MeterReading>,
    config: &LastgangConfig,
) -> Lastgang {
    ordered.sort_by_key(|r| r.at);

    let capacity = config.capacity();
    let mut intervals = Vec::with_capacity(ordered.len().saturating_sub(1));
    let mut rollovers = Vec::new();
    let mut anomalies = Vec::new();

    for pair in ordered.windows(2) {
        let (prev, next) = (pair[0], pair[1]);
        let anomaly = |kind| Anomaly {
            from: prev.at,
            to: next.at,
            kind,
            previous: prev.value,
            current: next.value,
        };

        if next.at == prev.at {
            anomalies.push(anomaly(AnomalyKind::ZeroLengthSpan));
            continue;
        }
        if !prev.quality.is_billable() || !next.quality.is_billable() {
            anomalies.push(anomaly(AnomalyKind::NonBillableEndpoint));
            continue;
        }

        let Some(straight) = next.value.checked_sub(prev.value) else {
            anomalies.push(anomaly(AnomalyKind::ImplausibleDelta));
            continue;
        };
        let (delta, wrapped) = if straight >= Decimal::ZERO {
            (straight, None)
        } else {
            let Some(cap) = capacity else {
                anomalies.push(anomaly(AnomalyKind::BackwardsWithoutRegisterWidth));
                continue;
            };
            let Some(reconstructed) = cap
                .checked_sub(prev.value)
                .and_then(|d| d.checked_add(next.value))
            else {
                anomalies.push(anomaly(AnomalyKind::ImplausibleRollover));
                continue;
            };
            // A reading above the register's own capacity is not a wrap at all.
            if reconstructed < Decimal::ZERO || prev.value >= cap {
                anomalies.push(anomaly(AnomalyKind::ImplausibleRollover));
                continue;
            }
            (reconstructed, Some(cap))
        };

        if let Some(max) = config.max_delta
            && delta > max
        {
            anomalies.push(anomaly(if wrapped.is_some() {
                AnomalyKind::ImplausibleRollover
            } else {
                AnomalyKind::ImplausibleDelta
            }));
            continue;
        }

        if let Some(register_capacity) = wrapped {
            rollovers.push(Rollover {
                from: prev.at,
                to: next.at,
                previous: prev.value,
                current: next.value,
                register_capacity,
                delta,
            });
        }

        // Sorted and equal instants refused above, so `build` cannot fail.
        intervals.extend(
            MeterInterval::build(
                prev.at,
                next.at,
                delta,
                prev.quality.worse_of(next.quality),
                config.result_channel.label(next.obis_code),
            )
            .ok(),
        );
    }

    Lastgang {
        register,
        intervals,
        rollovers,
        anomalies,
    }
}

/// The reading cadence of a Zählerstandsgang — the `cadence`
/// [`LastgangConfig::capacity_kw`] needs.
///
/// The **median** positive gap of the sorted timestamps, mapped by
/// [`Resolution::from_observed_seconds`] (so a daily series is a calendar
/// [`Day`](Resolution::Day)). `None` for fewer than two readings, when every
/// gap is zero, or when the gap maps to no resolution.
///
/// ```rust
/// use metering::series::reading::{MeterReading, detect_reading_cadence};
/// use metering::Resolution;
/// use rust_decimal::dec;
/// use time::{Duration, macros::datetime};
///
/// let zsg: Vec<MeterReading> = (0..8)
///     .map(|i| MeterReading::measured(
///         datetime!(2026-06-01 0:00 UTC) + Duration::minutes(15 * i),
///         dec!(1000) + rust_decimal::Decimal::from(i),
///     ))
///     .collect();
///
/// assert_eq!(detect_reading_cadence(&zsg), Some(Resolution::QUARTER_HOUR));
/// assert_eq!(detect_reading_cadence(&zsg[..1]), None, "one point has no spacing");
/// ```
#[must_use]
pub fn detect_reading_cadence(readings: &[MeterReading]) -> Option<Resolution> {
    if readings.len() < 2 {
        return None;
    }
    let mut instants: Vec<OffsetDateTime> = readings.iter().map(|r| r.at).collect();
    instants.sort_unstable();

    let mut gaps: Vec<i64> = instants
        .windows(2)
        .map(|w| (w[1] - w[0]).whole_seconds())
        .filter(|&g| g > 0)
        .collect();
    if gaps.is_empty() {
        return None;
    }
    gaps.sort_unstable();
    Resolution::from_observed_seconds(gaps[gaps.len() / 2])
}

/// Why [`consumption_between`] could not answer.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum ReadingError {
    /// `end` is not after `start`.
    #[error("the end reading at {end} is not after the start reading at {start}")]
    Reversed {
        /// When the start reading was taken.
        start: OffsetDateTime,
        /// When the end reading was taken.
        end: OffsetDateTime,
    },
    /// The two readings are on different registers.
    #[error("the readings are on different registers ({start:?} and {end:?})")]
    ChannelMismatch {
        /// The start reading's register.
        start: Option<ObisCode>,
        /// The end reading's register.
        end: Option<ObisCode>,
    },
    /// The difference cannot be taken honestly — see [`Anomaly`].
    #[error(transparent)]
    Anomaly(Anomaly),
}

/// Consumption between two readings of one register — the SLP
/// Jahresabrechnung path.
///
/// # Errors
///
/// [`ReadingError::Reversed`] when `end` is not after `start` (never swapped);
/// [`ReadingError::ChannelMismatch`] for two different registers;
/// [`ReadingError::Anomaly`] when the difference is refused.
///
/// ```rust
/// use metering::series::reading::{LastgangConfig, MeterReading, ReadingError, consumption_between};
/// use rust_decimal::dec;
/// use time::macros::datetime;
///
/// let start = MeterReading::measured(datetime!(2025-01-01 0:00 UTC), dec!(14_230));
/// let end   = MeterReading::measured(datetime!(2026-01-01 0:00 UTC), dec!(17_845));
/// assert_eq!(consumption_between(&start, &end, &LastgangConfig::default())?, dec!(3615));
/// assert!(matches!(
///     consumption_between(&end, &start, &LastgangConfig::default()),
///     Err(ReadingError::Reversed { .. })
/// ));
/// # Ok::<(), ReadingError>(())
/// ```
pub fn consumption_between(
    start: &MeterReading,
    end: &MeterReading,
    config: &LastgangConfig,
) -> Result<Decimal, ReadingError> {
    if end.at <= start.at {
        return Err(ReadingError::Reversed {
            start: start.at,
            end: end.at,
        });
    }
    if start.obis_code != end.obis_code {
        return Err(ReadingError::ChannelMismatch {
            start: start.obis_code,
            end: end.obis_code,
        });
    }
    let mut lastgang = difference(start.obis_code, vec![start, end], config);
    match lastgang.anomalies.pop() {
        Some(anomaly) => Err(ReadingError::Anomaly(anomaly)),
        None => Ok(lastgang
            .intervals
            .first()
            .map_or(Decimal::ZERO, MeterInterval::value)),
    }
}

impl std::fmt::Display for Anomaly {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} between {} ({}) and {} ({})",
            self.kind.description(),
            self.from,
            self.previous,
            self.to,
            self.current
        )
    }
}

impl std::error::Error for Anomaly {}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal::dec;
    use time::{Duration, macros::datetime};

    fn zsg(values: &[Decimal]) -> Vec<MeterReading> {
        values
            .iter()
            .enumerate()
            .map(|(i, &value)| {
                MeterReading::measured(
                    datetime!(2026-06-01 0:00 UTC) + Duration::minutes(i as i64 * 15),
                    value,
                )
            })
            .collect()
    }

    /// An HT minus an NT reading is refused.
    #[test]
    fn consumption_between_refuses_two_registers() {
        let ht: ObisCode = "1-0:1.8.1".parse().unwrap();
        let nt: ObisCode = "1-0:1.8.2".parse().unwrap();
        let t = datetime!(2026-06-01 0:00 UTC);
        let start = MeterReading::measured(t, dec!(1000)).with_obis(ht);
        let end = MeterReading::measured(t + Duration::days(30), dec!(1200)).with_obis(nt);
        assert_eq!(
            consumption_between(&start, &end, &LastgangConfig::strom()),
            Err(ReadingError::ChannelMismatch {
                start: Some(ht),
                end: Some(nt),
            })
        );
        let end_ht = MeterReading::measured(t + Duration::days(30), dec!(1200)).with_obis(ht);
        assert_eq!(
            consumption_between(&start, &end_ht, &LastgangConfig::strom()),
            Ok(dec!(200))
        );
    }

    /// The Lastgang sums to the difference of the outer Zählerstände.
    #[test]
    fn the_lastgang_sums_to_the_register_difference() {
        let readings = zsg(&[dec!(1000), dec!(1002.5), dec!(1004.8), dec!(1007)]);
        let result = to_lastgang(&readings, &LastgangConfig::strom()).remove(0);

        assert_eq!(result.intervals.len(), 3, "n readings give n−1 intervals");
        assert_eq!(result.total(), Some(dec!(7)), "1007 − 1000");
        assert_eq!(result.intervals[0].value(), dec!(2.5));
        assert_eq!(result.intervals[1].value(), dec!(2.3));
        assert_eq!(result.intervals[2].value(), dec!(2.2));
        assert!(result.is_clean());
    }

    #[test]
    fn intervals_tile_the_reading_timestamps() {
        let readings = zsg(&[dec!(0), dec!(1), dec!(2), dec!(3)]);
        let result = to_lastgang(&readings, &LastgangConfig::strom()).remove(0);
        assert_eq!(result.intervals[0].from(), readings[0].at);
        for pair in result.intervals.windows(2) {
            assert_eq!(pair[0].to(), pair[1].from());
        }
        assert_eq!(result.intervals.last().unwrap().to(), readings[3].at);
    }

    #[test]
    fn fewer_than_two_readings_yield_nothing() {
        assert!(to_lastgang(&[], &LastgangConfig::strom()).is_empty());
        let one = zsg(&[dec!(1000)]);
        let result = to_lastgang(&one, &LastgangConfig::strom()).remove(0);
        assert!(result.intervals.is_empty());
        assert!(result.is_clean(), "one reading is short, not corrupt");
    }

    #[test]
    fn readings_are_sorted_before_differencing() {
        let ordered = zsg(&[dec!(1000), dec!(1002.5), dec!(1004.8)]);
        let mut shuffled = ordered.clone();
        shuffled.reverse();

        let a = to_lastgang(&ordered, &LastgangConfig::strom()).remove(0);
        let b = to_lastgang(&shuffled, &LastgangConfig::strom()).remove(0);
        assert_eq!(a.intervals, b.intervals);
        assert!(
            b.is_clean(),
            "reordering is not corruption: {:?}",
            b.anomalies
        );
    }

    #[test]
    fn a_six_digit_register_wrap_is_reconstructed() {
        let readings = vec![
            MeterReading::measured(datetime!(2026-06-01 0:00 UTC), dec!(999998.5)),
            MeterReading::measured(datetime!(2026-06-01 0:15 UTC), dec!(1.5)),
        ];
        let cfg = LastgangConfig::strom().register_digits(6);
        let result = to_lastgang(&readings, &cfg).remove(0);

        assert_eq!(result.intervals[0].value(), dec!(3.0));
        assert_eq!(result.rollovers.len(), 1);
        assert!(result.anomalies.is_empty());

        let rollover = &result.rollovers[0];
        assert_eq!(rollover.register_capacity, dec!(1000000));
        assert_eq!(rollover.delta, dec!(3.0));
        assert_eq!(rollover.from, datetime!(2026-06-01 0:00 UTC));
        assert_eq!(rollover.to, datetime!(2026-06-01 0:15 UTC));
        assert_eq!(rollover.duration(), Duration::minutes(15));
        assert!(
            !result.is_clean(),
            "a wrap is explained, but still reported"
        );
    }

    #[test]
    fn a_backwards_step_without_a_width_is_an_anomaly() {
        let readings = zsg(&[dec!(999998.5), dec!(1.5)]);
        let result = to_lastgang(&readings, &LastgangConfig::strom()).remove(0);
        assert!(result.intervals.is_empty(), "no value is invented");
        assert_eq!(
            result.anomalies[0].kind,
            AnomalyKind::BackwardsWithoutRegisterWidth
        );
    }

    /// A meter replaced at 800 000 by one starting at 0 is an exchange, not a
    /// wrap; the delta cap tells them apart.
    #[test]
    fn an_implausible_wrap_is_rejected_rather_than_billed() {
        let readings = vec![
            MeterReading::measured(datetime!(2026-06-01 0:00 UTC), dec!(800000)),
            MeterReading::measured(datetime!(2026-06-01 0:15 UTC), dec!(0)),
        ];
        let cfg = LastgangConfig::strom()
            .register_digits(6)
            .max_delta(dec!(7.5)); // a 30 kW connection, quarter-hourly

        let result = to_lastgang(&readings, &cfg).remove(0);
        assert!(result.intervals.is_empty());
        assert!(result.rollovers.is_empty());
        assert_eq!(result.anomalies[0].kind, AnomalyKind::ImplausibleRollover);

        let uncapped =
            to_lastgang(&readings, &LastgangConfig::strom().register_digits(6)).remove(0);
        assert_eq!(uncapped.intervals[0].value(), dec!(200000));
    }

    #[test]
    fn an_implausible_forward_delta_is_refused() {
        let readings = zsg(&[dec!(1000), dec!(9000)]);
        let cfg = LastgangConfig::strom().max_delta(dec!(7.5));
        let result = to_lastgang(&readings, &cfg).remove(0);
        assert!(result.intervals.is_empty());
        assert_eq!(result.anomalies[0].kind, AnomalyKind::ImplausibleDelta);
    }

    #[test]
    fn the_delta_cap_can_come_from_a_connection_capacity() {
        let cfg = LastgangConfig::strom()
            .capacity_kw(dec!(30), Resolution::QUARTER_HOUR)
            .expect("no overflow");
        assert_eq!(cfg.max_delta, Some(dec!(7.5)));

        let hourly = LastgangConfig::strom()
            .capacity_kw(dec!(30), Resolution::Hour)
            .expect("no overflow");
        assert_eq!(hourly.max_delta, Some(dec!(30)));

        // 8 kWh in a quarter-hour is 32 kW — over the ceiling.
        let readings = zsg(&[dec!(0), dec!(8)]);
        assert!(!to_lastgang(&readings, &cfg).remove(0).anomalies.is_empty());
        assert!(to_lastgang(&readings, &hourly).remove(0).is_clean());
    }

    #[test]
    fn a_reading_wider_than_the_register_is_not_a_wrap() {
        let readings = zsg(&[dec!(50000), dec!(10)]);
        let cfg = LastgangConfig::strom().register_digits(4); // wraps at 10 000
        let result = to_lastgang(&readings, &cfg).remove(0);
        assert_eq!(result.anomalies[0].kind, AnomalyKind::ImplausibleRollover);
    }

    #[test]
    fn the_worse_endpoint_quality_wins() {
        let mut readings = zsg(&[dec!(0), dec!(2), dec!(4)]);
        readings[1].quality = QualityFlag::Estimated;
        let result = to_lastgang(&readings, &LastgangConfig::strom()).remove(0);
        assert_eq!(result.intervals[0].quality(), QualityFlag::Estimated);
        assert_eq!(result.intervals[1].quality(), QualityFlag::Estimated);
    }

    #[test]
    fn a_non_billable_endpoint_yields_no_interval() {
        let mut readings = zsg(&[dec!(0), dec!(2), dec!(4)]);
        readings[1].quality = QualityFlag::Faulty;
        let result = to_lastgang(&readings, &LastgangConfig::strom()).remove(0);
        assert!(
            result.intervals.is_empty(),
            "both spans touch the bad reading"
        );
        assert_eq!(result.anomalies.len(), 2);
        assert!(
            result
                .anomalies
                .iter()
                .all(|a| a.kind == AnomalyKind::NonBillableEndpoint)
        );
    }

    #[test]
    fn the_result_is_labelled_as_a_lastgang() {
        let readings = zsg(&[dec!(0), dec!(2)])
            .into_iter()
            .map(|r| r.with_obis(ObisCode::STROM_BEZUG_TOTAL))
            .collect::<Vec<_>>();

        let labelled = to_lastgang(&readings, &LastgangConfig::strom()).remove(0);
        assert_eq!(
            labelled.intervals[0].obis(),
            Some(ObisCode::STROM_BEZUG_LASTGANG)
        );
        assert!(labelled.intervals[0].obis().unwrap().is_lastgang());

        let passthrough = to_lastgang(&readings, &LastgangConfig::default()).remove(0);
        assert_eq!(
            passthrough.intervals[0].obis(),
            Some(ObisCode::STROM_BEZUG_TOTAL)
        );
    }

    #[test]
    fn duplicate_timestamps_are_a_zero_length_span() {
        let at = datetime!(2026-06-01 0:00 UTC);
        let readings = vec![
            MeterReading::measured(at, dec!(100)),
            MeterReading::measured(at, dec!(102)),
        ];
        let result = to_lastgang(&readings, &LastgangConfig::strom()).remove(0);
        assert!(result.intervals.is_empty());
        assert_eq!(result.anomalies[0].kind, AnomalyKind::ZeroLengthSpan);
    }

    #[test]
    fn an_unchanged_register_is_zero_consumption() {
        let readings = zsg(&[dec!(1000), dec!(1000), dec!(1000)]);
        let result = to_lastgang(&readings, &LastgangConfig::strom()).remove(0);
        assert!(result.is_clean());
        assert_eq!(result.total(), Some(Decimal::ZERO));
        assert!(result.intervals.iter().all(|iv| iv.value().is_zero()));
    }

    #[test]
    fn an_absurd_register_width_disables_reconstruction() {
        let readings = zsg(&[dec!(100), dec!(10)]);
        let cfg = LastgangConfig::strom().register_digits(99);
        let result = to_lastgang(&readings, &cfg).remove(0);
        assert_eq!(
            result.anomalies[0].kind,
            AnomalyKind::BackwardsWithoutRegisterWidth
        );
    }

    #[test]
    fn a_jahresabrechnung_is_two_readings() {
        let start = MeterReading::measured(datetime!(2025-01-01 0:00 UTC), dec!(14_230));
        let end = MeterReading::measured(datetime!(2026-01-01 0:00 UTC), dec!(17_845));
        assert_eq!(
            consumption_between(&start, &end, &LastgangConfig::default()).unwrap(),
            dec!(3615)
        );
    }

    #[test]
    fn consumption_between_reports_the_anomaly_rather_than_a_number() {
        let start = MeterReading::measured(datetime!(2025-01-01 0:00 UTC), dec!(17_845));
        let end = MeterReading::measured(datetime!(2026-01-01 0:00 UTC), dec!(14_230));
        let err = consumption_between(&start, &end, &LastgangConfig::default()).unwrap_err();
        assert!(matches!(
            &err,
            ReadingError::Anomaly(a) if a.kind == AnomalyKind::BackwardsWithoutRegisterWidth
        ));
        assert!(err.to_string().contains("register decreased"), "{err}");
    }

    /// The hole an anomaly leaves is an ordinary gap that Ersatzwertbildung
    /// fills.
    #[test]
    fn an_anomalous_span_becomes_a_gap_the_substitute_engine_can_fill() {
        use crate::time::holiday::Bundesland;
        use crate::vee::substitute::{Method, Policy, SubstitutionReason, substitute};
        use crate::vee::validation::{Rule, Rules, validate};
        use crate::{DayBoundary, Resolution};

        // One corrupt value: the step down is backwards, the step back up is
        // 506 kWh — far over the 30 kW cap.
        let readings = zsg(&[dec!(1000), dec!(1002), dec!(500), dec!(1006), dec!(1008)]);
        let cfg = LastgangConfig::strom()
            .capacity_kw(dec!(30), Resolution::QUARTER_HOUR)
            .expect("no overflow");
        let result = to_lastgang(&readings, &cfg).remove(0);
        assert_eq!(
            result.anomalies.len(),
            2,
            "both spans touching the bad value"
        );
        assert_eq!(
            result.anomalies[1].kind,
            AnomalyKind::ImplausibleDelta,
            "the recovery step is as implausible as the drop"
        );
        assert_eq!(result.intervals.len(), 2);

        let series = result
            .to_series(Resolution::QUARTER_HOUR, DayBoundary::Strom)
            .unwrap();
        let hour = DayBoundary::Strom
            .bucket(datetime!(2026-06-01 0:00 UTC), Resolution::Hour)
            .unwrap();
        let report = validate(
            &series,
            &Rules::strom(hour, datetime!(2026-07-01 0:00 UTC), None),
        );
        assert_eq!(
            report.by_rule(Rule::Gap).count(),
            1,
            "{:?}",
            report.findings
        );

        let policy = Policy::strom(Bundesland::Be, SubstitutionReason::ImplausibleValue);
        let filled = substitute(&series, &report, &policy).unwrap();
        assert_eq!(filled.series.len(), 4);
        assert_eq!(filled.substitutes.len(), 2);
        assert!(
            filled
                .substitutes
                .iter()
                .all(|s| s.method == Method::Interpolation)
        );
    }

    #[test]
    fn anomaly_metadata_is_complete() {
        for kind in AnomalyKind::ALL {
            assert!(!kind.description().is_empty(), "{kind:?}");
        }
    }
}

#[cfg(test)]
mod channel_and_cadence_tests {
    use super::*;
    use rust_decimal::dec;
    use time::{Duration, macros::datetime};

    fn zsg(
        start: OffsetDateTime,
        step: Duration,
        n: i64,
        code: Option<ObisCode>,
    ) -> Vec<MeterReading> {
        (0..n)
            .map(|i| {
                let r = MeterReading::measured(
                    start + step * (i as i32),
                    dec!(1000) + Decimal::from(i),
                );
                match code {
                    Some(c) => r.with_obis(c),
                    None => r,
                }
            })
            .collect()
    }

    /// A feed-in Zählerstandsgang keeps its direction.
    #[test]
    fn a_feed_in_series_is_not_relabelled_as_import() {
        let readings = zsg(
            datetime!(2026-06-01 0:00 UTC),
            Duration::minutes(15),
            4,
            Some(ObisCode::STROM_EINSPEISUNG_TOTAL),
        );
        let result = to_lastgang(&readings, &LastgangConfig::strom()).remove(0);

        for iv in &result.intervals {
            let code = iv.obis().expect("labelled");
            assert_eq!(code, ObisCode::STROM_EINSPEISUNG_LASTGANG);
            assert!(code.is_export() && !code.is_import());
            assert!(code.is_lastgang());
        }

        let bezug = zsg(
            datetime!(2026-06-01 0:00 UTC),
            Duration::minutes(15),
            4,
            Some(ObisCode::STROM_BEZUG_TOTAL),
        );
        assert_eq!(
            to_lastgang(&bezug, &LastgangConfig::strom())
                .remove(0)
                .intervals[0]
                .obis(),
            Some(ObisCode::STROM_BEZUG_LASTGANG)
        );
    }

    #[test]
    fn a_register_without_a_lastgang_keeps_its_code() {
        for code in [
            "1-0:1.8.1".parse::<ObisCode>().unwrap(), // HT — no tariff Lastgang exists
            ObisCode::GAS_VOLUME_M3,                  // gas — D = 29 means nothing there
        ] {
            let readings = zsg(
                datetime!(2026-06-01 0:00 UTC),
                Duration::minutes(15),
                3,
                Some(code),
            );
            let result = to_lastgang(&readings, &LastgangConfig::strom()).remove(0);
            assert_eq!(result.intervals[0].obis(), Some(code), "{code}");
        }

        let bare = zsg(
            datetime!(2026-06-01 0:00 UTC),
            Duration::minutes(15),
            3,
            None,
        );
        assert_eq!(
            to_lastgang(&bare, &LastgangConfig::strom())
                .remove(0)
                .intervals[0]
                .obis(),
            None
        );
    }

    #[test]
    fn the_three_result_channels_do_what_they_say() {
        let readings = zsg(
            datetime!(2026-06-01 0:00 UTC),
            Duration::minutes(15),
            3,
            Some(ObisCode::STROM_BEZUG_TOTAL),
        );
        let label =
            |cfg: LastgangConfig| to_lastgang(&readings, &cfg).remove(0).intervals[0].obis();

        assert_eq!(
            label(LastgangConfig::default()),
            Some(ObisCode::STROM_BEZUG_TOTAL),
            "Unchanged carries the reading's own code"
        );
        assert_eq!(
            label(LastgangConfig::strom()),
            Some(ObisCode::STROM_BEZUG_LASTGANG)
        );
        assert_eq!(
            label(LastgangConfig::default().label(ObisCode::GAS_VOLUME_M3)),
            Some(ObisCode::GAS_VOLUME_M3),
            "Fixed does exactly what it says, including when that is wrong"
        );
        assert_eq!(ResultChannel::default(), ResultChannel::Unchanged);
        assert_eq!(ResultChannel::Derived.label(None), None);
    }

    #[test]
    fn the_cadence_comes_from_the_spacing_of_the_readings() {
        let base = datetime!(2026-06-01 0:00 UTC);
        for (step, expected) in [
            (Duration::minutes(15), Resolution::QUARTER_HOUR),
            (Duration::minutes(30), Resolution::HALF_HOUR),
            (Duration::hours(1), Resolution::Hour),
        ] {
            assert_eq!(
                detect_reading_cadence(&zsg(base, step, 8, None)),
                Some(expected),
                "{step:?}"
            );
        }

        assert_eq!(detect_reading_cadence(&[]), None);
        assert_eq!(
            detect_reading_cadence(&zsg(base, Duration::ZERO, 1, None)),
            None
        );
        assert_eq!(
            detect_reading_cadence(&zsg(base, Duration::ZERO, 4, None)),
            None
        );
    }

    #[test]
    fn the_cadence_is_robust_to_gaps_and_disorder() {
        let base = datetime!(2026-06-01 0:00 UTC);
        let mut readings = zsg(base, Duration::minutes(15), 12, None);
        readings.remove(5); // one missed reading widens a single gap to 30 min
        assert_eq!(
            detect_reading_cadence(&readings),
            Some(Resolution::QUARTER_HOUR)
        );

        readings.reverse();
        assert_eq!(
            detect_reading_cadence(&readings),
            Some(Resolution::QUARTER_HOUR),
            "sorted before differencing"
        );
    }

    /// A daily cadence is a calendar day, so the derived cap admits the 25-hour
    /// day.
    #[test]
    fn a_daily_cadence_is_a_calendar_day() {
        let readings: Vec<MeterReading> = (0..6)
            .map(|i| {
                let day = time::macros::date!(2026 - 10 - 23)
                    .checked_add(Duration::days(i))
                    .unwrap();
                MeterReading::measured(
                    crate::time::calendar::DayBoundary::Strom
                        .day(day)
                        .unwrap()
                        .start(),
                    dec!(1000) + Decimal::from(i),
                )
            })
            .collect();
        assert_eq!(
            detect_reading_cadence(&readings),
            Some(Resolution::Day),
            "not a fixed 86 400 s window"
        );

        let cfg = LastgangConfig::default()
            .capacity_kw(dec!(30), Resolution::Day)
            .expect("no overflow");
        assert_eq!(cfg.max_delta, Some(dec!(750)), "30 kW × 25 h");

        // 740 kWh: over a 24 h cap (720), under the 25 h one.
        let long_day = vec![
            MeterReading::measured(
                crate::time::calendar::DayBoundary::Strom
                    .day(time::macros::date!(2026 - 10 - 25))
                    .unwrap()
                    .start(),
                dec!(0),
            ),
            MeterReading::measured(
                crate::time::calendar::DayBoundary::Strom
                    .day(time::macros::date!(2026 - 10 - 25))
                    .unwrap()
                    .end(),
                dec!(740),
            ),
        ];
        assert!(
            to_lastgang(&long_day, &cfg).remove(0).is_clean(),
            "a 25-hour day at full load must not read as an anomaly"
        );
    }

    /// The delta cap refuses what the validation capacity rule flags.
    #[test]
    fn the_two_capacity_ceilings_agree() {
        use crate::vee::validation::{Rule, Rules, validate};
        use crate::{DayBoundary, Series};

        let capacity = dec!(30);
        let cfg = LastgangConfig::default()
            .capacity_kw(capacity, Resolution::QUARTER_HOUR)
            .expect("no overflow");
        assert_eq!(cfg.max_delta, Some(dec!(7.5)), "30 kW × 0.25 h");

        let over = vec![
            MeterReading::measured(datetime!(2026-06-01 0:00 UTC), dec!(0)),
            MeterReading::measured(datetime!(2026-06-01 0:15 UTC), dec!(8)),
        ];
        assert!(to_lastgang(&over, &cfg).remove(0).intervals.is_empty());

        let already_formed = to_lastgang(&over, &LastgangConfig::default())
            .remove(0)
            .intervals;
        let series =
            Series::new(Resolution::QUARTER_HOUR, DayBoundary::Strom, already_formed).unwrap();
        let day = DayBoundary::Strom
            .day(time::macros::date!(2026 - 06 - 01))
            .unwrap();
        let report = validate(
            &series,
            &Rules::strom(day, datetime!(2026-07-01 0:00 UTC), Some(capacity)),
        );
        assert_eq!(
            report.by_rule(Rule::Capacity).count(),
            1,
            "{:?}",
            report.findings
        );
    }
}
