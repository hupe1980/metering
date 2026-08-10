//! Zählerstände and the Zählerstandsgang → Lastgang conversion.
//!
//! ## Why this module exists
//!
//! A meter counts upwards. Everything else in this crate works on
//! [`MeterInterval`] — the energy *in* a period — but that is a derived
//! quantity: what a register actually holds is a cumulative **Zählerstand**,
//! and the interval energy is the difference between two of them.
//!
//! Since 6 June 2025 that difference is the Messstellenbetreiber's job by
//! regulation. BNetzA **BK6-24-174** is titled *"Datenübermittlung ZSG"*, and
//! the model it implements is:
//!
//! ```text
//! Smart-Meter-Gateway ──Zählerstandsgang──► MSB ──Lastgang──► NB, Lieferant
//!                        (this module)      └── differencing happens here
//! ```
//!
//! § 2 Satz 1 Nr. 27 MsbG defines the input verbatim: *"die Messung einer Reihe
//! **viertelstündig ermittelter Zählerstände** von elektrischer Arbeit und
//! **stündlich ermittelter Zählerstände** von Gasmengen"*. Note the two
//! resolutions — electricity is quarter-hourly, gas hourly.
//!
//! ## Rollover belongs here, not in the validation engine
//!
//! [`crate::validation`] used to carry a V10 "register rollover" rule that
//! compared consecutive `MeterInterval::value` for a large drop. That could
//! not work: an interval value is not cumulative, so it has nothing to roll
//! over. A rollover (Überlauf) is a property of a **register** — a six-digit
//! Zählwerk wraps from 999 999 to 0 — and can only be detected where readings
//! live. That is here, and [`Rollover`] is what V10 was trying to be.
//!
//! ## The conversion never invents a value
//!
//! Where a difference cannot be taken honestly — an implausible jump, a
//! backwards step that no register width explains — this module emits **no
//! interval** and records an [`Anomaly`]. The hole then shows up as a V01 gap
//! in validation and is filled, with an audit trail, by
//! [`crate::substitute`]. Guessing here would bury the problem inside a value
//! that looks measured.
//!
//! ## Example
//!
//! ```rust
//! use metering::reading::{LastgangConfig, MeterReading, to_lastgang};
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
//! let lastgang = to_lastgang(&zsg, &LastgangConfig::strom());
//! assert_eq!(lastgang.intervals.len(), 3, "n readings give n−1 intervals");
//! assert_eq!(lastgang.intervals[0].value, dec!(2.5));
//! assert_eq!(lastgang.intervals[1].value, dec!(2.3));
//! assert!(lastgang.is_clean());
//! ```

use rust_decimal::Decimal;
use time::OffsetDateTime;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use crate::interval::{MeterInterval, QualityFlag};
use crate::obis::ObisCode;

// ── MeterReading ──────────────────────────────────────────────────────────────

/// A cumulative meter reading (Zählerstand) at one instant.
///
/// The unit is whatever the register counts: kWh for electricity and heat, m³
/// for gas and water. This type does not name it — [`crate::MeasurementUnit`]
/// and the OBIS medium do — because the differencing below is unit-agnostic and
/// converting the *difference* is [`crate::conversion`]'s job.
///
/// `at` is a UTC instant, like every timestamp in this crate.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct MeterReading {
    /// When the register held this value (UTC).
    pub at: OffsetDateTime,
    /// The register value. Cumulative and, absent a rollover, non-decreasing.
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

// ── Rollover ──────────────────────────────────────────────────────────────────

/// A register wrap, reconstructed from the register width.
///
/// A Zählwerk with `digits` decimal places before the point counts to
/// `10^digits − 1` and then returns to zero. The consumption across the wrap is
///
/// ```text
/// delta = (10^digits − previous) + current
/// ```
///
/// which is what a technician computes by hand and what this module computes
/// automatically — but only when it is *plausible*. See
/// [`LastgangConfig::max_delta`].
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct Rollover {
    /// Instant the wrap was detected at (the later reading).
    pub at: OffsetDateTime,
    /// Register value before the wrap.
    pub previous: Decimal,
    /// Register value after it.
    pub current: Decimal,
    /// The register's capacity, `10^digits`.
    pub register_capacity: Decimal,
    /// Consumption across the wrap.
    pub delta: Decimal,
}

// ── Anomaly ───────────────────────────────────────────────────────────────────

/// A pair of readings that no honest difference could be taken from.
///
/// The corresponding interval is **absent** from
/// [`Lastgang::intervals`] — see the
/// [module docs](self#the-conversion-never-invents-a-value).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct Anomaly {
    /// Start of the affected span (the earlier reading).
    pub from: OffsetDateTime,
    /// End of it (the later reading).
    pub to: OffsetDateTime,
    /// Why the difference was refused.
    pub kind: AnomalyKind,
    /// The earlier register value.
    pub previous: Decimal,
    /// The later register value.
    pub current: Decimal,
}

/// Why a difference between two readings was refused.
///
/// `#[non_exhaustive]`: a caller that wildcards an unfamiliar kind still does
/// the right thing — it treats the span as unusable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "SCREAMING_SNAKE_CASE"))]
#[non_exhaustive]
pub enum AnomalyKind {
    /// The register went backwards and no register width was configured, so a
    /// wrap could not be reconstructed.
    ///
    /// The usual causes are an undocumented meter exchange, a reading entered
    /// against the wrong register, or a series merged out of order.
    BackwardsWithoutRegisterWidth,
    /// The register went backwards, and reconstructing a wrap would imply a
    /// consumption above [`LastgangConfig::max_delta`].
    ///
    /// A wrap is one explanation for a backwards step and a meter exchange is
    /// another; when the wrap reading is implausible, the exchange is likelier.
    ImplausibleRollover,
    /// The forward difference exceeds [`LastgangConfig::max_delta`].
    ImplausibleDelta,
    /// The two readings carry the same timestamp, so there is no span between
    /// them.
    ZeroLengthSpan,
    /// One of the two readings is not billable, so the difference would not be
    /// either.
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

// ── LastgangConfig ────────────────────────────────────────────────────────────

/// How to turn a Zählerstandsgang into a Lastgang.
#[derive(Debug, Clone, PartialEq)]
pub struct LastgangConfig {
    /// Decimal places **before** the point on the register, so it wraps at
    /// `10^digits`.
    ///
    /// German electricity meters are typically six-digit (999 999 kWh); gas
    /// meters five or six. `None` disables wrap reconstruction, and a backwards
    /// step then becomes an [`AnomalyKind::BackwardsWithoutRegisterWidth`]
    /// rather than a silently invented value.
    ///
    /// Leave it unset unless you know the width. Guessing it wrong turns a
    /// meter exchange into a million kWh of consumption.
    pub register_digits: Option<u32>,

    /// Largest difference to accept between two consecutive readings.
    ///
    /// This is the plausibility check that makes wrap reconstruction safe: a
    /// backwards step has two explanations, a wrap and an exchange, and the cap
    /// is what tells them apart. `None` accepts any forward difference and any
    /// reconstructable wrap.
    ///
    /// Express it for the *reading interval*, not per hour — for a quarter-hour
    /// ZSG on a 30 kW connection that is 7.5 kWh.
    pub max_delta: Option<Decimal>,

    /// The OBIS code to stamp on the derived intervals.
    ///
    /// A Lastgang is a different channel from the Zählerstand it came from:
    /// `1-0:1.8.0` is a Zählerstand (D = 8) and `1-0:1.29.0` is the Lastgang
    /// (D = 29). `None` carries the readings' own code through unchanged, which
    /// is convenient but strictly speaking mislabels the result.
    pub result_obis: Option<ObisCode>,
}

impl Default for LastgangConfig {
    /// The most conservative configuration there is: no wrap reconstruction, no
    /// plausibility cap, and the readings' own OBIS code carried through.
    ///
    /// Written out rather than derived, because every field's default is a
    /// deliberate refusal to guess a device property, and `#[derive(Default)]`
    /// would make that look incidental.
    fn default() -> Self {
        Self {
            register_digits: None,
            max_delta: None,
            result_obis: None,
        }
    }
}

impl LastgangConfig {
    /// Electricity: quarter-hourly readings, results labelled `1-0:1.29.0`.
    ///
    /// No register width and no delta cap — both are properties of the specific
    /// device and connection, and a wrong default is worse than none. Add them
    /// with [`with_register_digits`](Self::with_register_digits) and
    /// [`with_max_delta`](Self::with_max_delta).
    #[must_use]
    pub const fn strom() -> Self {
        Self {
            register_digits: None,
            max_delta: None,
            result_obis: Some(ObisCode::STROM_BEZUG_LASTGANG),
        }
    }

    /// Set the register width, enabling wrap reconstruction.
    #[must_use]
    pub const fn with_register_digits(mut self, digits: u32) -> Self {
        self.register_digits = Some(digits);
        self
    }

    /// Set the largest plausible difference between consecutive readings.
    #[must_use]
    pub const fn with_max_delta(mut self, max: Decimal) -> Self {
        self.max_delta = Some(max);
        self
    }

    /// Derive the delta cap from a connection capacity in kW.
    ///
    /// `max_delta = capacity_kw × interval_seconds / 3600`, the most energy the
    /// connection can pass in one reading interval. This is the same ceiling
    /// [`crate::ValidationConfig::max_plant_power_kw`] applies to the resulting
    /// Lastgang, expressed at the point where it can prevent a bad value rather
    /// than merely flag one.
    #[must_use]
    pub fn with_capacity_kw(mut self, capacity_kw: Decimal, interval_secs: u32) -> Self {
        let hours = Decimal::from(interval_secs) / Decimal::from(3600u32);
        self.max_delta = Some(capacity_kw * hours);
        self
    }

    /// Label the derived intervals with `code`.
    #[must_use]
    pub const fn labelled(mut self, code: ObisCode) -> Self {
        self.result_obis = Some(code);
        self
    }

    /// The register capacity, `10^digits`, when a width is configured.
    ///
    /// `None` for an absurd width: a `Decimal` holds 28–29 significant digits,
    /// and a register wider than that is a typo rather than a meter. Returning
    /// `None` disables wrap reconstruction, which fails safe — the backwards
    /// step becomes an anomaly instead of an overflowed capacity.
    fn capacity(&self) -> Option<Decimal> {
        let digits = self.register_digits?;
        if digits > 28 {
            return None;
        }
        // Repeated multiplication rather than `Decimal::powu`, which lives
        // behind rust_decimal's `maths` feature — one dependency feature is not
        // worth twenty-eight multiplications that happen once per call.
        (0..digits).try_fold(Decimal::ONE, |acc, _| acc.checked_mul(Decimal::TEN))
    }
}

// ── Lastgang ──────────────────────────────────────────────────────────────────

/// The result of differencing a Zählerstandsgang.
#[derive(Debug, Clone, PartialEq)]
pub struct Lastgang {
    /// The derived intervals, ascending. One per *usable* pair of consecutive
    /// readings, so `n` clean readings give `n − 1` intervals.
    pub intervals: Vec<MeterInterval>,
    /// Register wraps that were reconstructed. The corresponding intervals
    /// **are** in [`intervals`](Self::intervals) — a wrap is explained, not an
    /// error.
    pub rollovers: Vec<Rollover>,
    /// Pairs no difference could be taken from. The corresponding intervals are
    /// **absent**.
    pub anomalies: Vec<Anomaly>,
}

impl Lastgang {
    /// `true` when every consecutive pair yielded an interval and none needed a
    /// wrap reconstructed.
    #[must_use]
    pub fn is_clean(&self) -> bool {
        self.anomalies.is_empty() && self.rollovers.is_empty()
    }

    /// Total energy across the derived intervals.
    ///
    /// Equal to the last reading minus the first **only when there are no
    /// anomalies** — an omitted span is energy this total does not contain.
    #[must_use]
    pub fn total(&self) -> Decimal {
        self.intervals.iter().map(|iv| iv.value).sum()
    }
}

// ── to_lastgang ───────────────────────────────────────────────────────────────

/// Difference a Zählerstandsgang into a Lastgang.
///
/// Readings are sorted by timestamp first, so a series merged out of order
/// converts correctly rather than producing a run of negative differences.
/// Duplicate timestamps yield an [`AnomalyKind::ZeroLengthSpan`].
///
/// The quality of each derived interval is the **worse** of its two endpoints,
/// by [`QualityFlag::severity_rank`]: a difference is only as good as the
/// readings it came from. A pair with a non-billable endpoint produces no
/// interval at all.
///
/// ## Example — a six-digit register wrapping
///
/// ```rust
/// use metering::reading::{LastgangConfig, MeterReading, to_lastgang};
/// use rust_decimal::dec;
/// use time::macros::datetime;
///
/// let zsg = vec![
///     MeterReading::measured(datetime!(2026-06-01 0:00 UTC), dec!(999998.5)),
///     MeterReading::measured(datetime!(2026-06-01 0:15 UTC), dec!(1.5)), // wrapped
/// ];
///
/// // Without a register width the drop is unexplainable, so nothing is invented.
/// let blind = to_lastgang(&zsg, &LastgangConfig::strom());
/// assert!(blind.intervals.is_empty());
/// assert_eq!(blind.anomalies.len(), 1);
///
/// // With one, the wrap is reconstructed: (1 000 000 − 999 998.5) + 1.5 = 3.
/// let cfg = LastgangConfig::strom().with_register_digits(6);
/// let wrapped = to_lastgang(&zsg, &cfg);
/// assert_eq!(wrapped.intervals[0].value, dec!(3.0));
/// assert_eq!(wrapped.rollovers.len(), 1);
/// assert!(wrapped.anomalies.is_empty());
/// ```
#[must_use]
pub fn to_lastgang(readings: &[MeterReading], config: &LastgangConfig) -> Lastgang {
    let mut ordered: Vec<&MeterReading> = readings.iter().collect();
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

        let straight = next.value - prev.value;
        let (delta, wrapped) = if straight >= Decimal::ZERO {
            (straight, false)
        } else {
            // The register went backwards. A wrap is the only explanation this
            // module can reconstruct, and only when the width says how wide the
            // register is.
            let Some(cap) = capacity else {
                anomalies.push(anomaly(AnomalyKind::BackwardsWithoutRegisterWidth));
                continue;
            };
            let reconstructed = (cap - prev.value) + next.value;
            // A reading above the register's own capacity is not a wrap at all.
            if reconstructed < Decimal::ZERO || prev.value >= cap {
                anomalies.push(anomaly(AnomalyKind::ImplausibleRollover));
                continue;
            }
            (reconstructed, true)
        };

        if let Some(max) = config.max_delta
            && delta > max
        {
            anomalies.push(anomaly(if wrapped {
                AnomalyKind::ImplausibleRollover
            } else {
                AnomalyKind::ImplausibleDelta
            }));
            continue;
        }

        if wrapped {
            rollovers.push(Rollover {
                at: next.at,
                previous: prev.value,
                current: next.value,
                register_capacity: capacity.unwrap_or(Decimal::ZERO),
                delta,
            });
        }

        intervals.push(MeterInterval {
            from: prev.at,
            to: next.at,
            value: delta,
            quality: prev.quality.worse_of(next.quality),
            obis_code: config.result_obis.or(next.obis_code),
        });
    }

    Lastgang {
        intervals,
        rollovers,
        anomalies,
    }
}

// ── consumption between two readings ──────────────────────────────────────────

/// Consumption between two readings, the way a Jahresabrechnung computes it.
///
/// This is [`to_lastgang`] for the two-reading case, returning just the
/// quantity. It is the ordinary billing path for an SLP delivery point, where
/// there is no interval series at all — only a Jahresablesung a year apart.
///
/// # Errors
///
/// Returns the [`Anomaly`] rather than a number when the difference cannot be
/// taken honestly.
///
/// ```rust
/// use metering::reading::{LastgangConfig, MeterReading, consumption_between};
/// use rust_decimal::dec;
/// use time::macros::datetime;
///
/// let start = MeterReading::measured(datetime!(2025-01-01 0:00 UTC), dec!(14_230));
/// let end   = MeterReading::measured(datetime!(2026-01-01 0:00 UTC), dec!(17_845));
/// let kwh = consumption_between(&start, &end, &LastgangConfig::default())?;
/// assert_eq!(kwh, dec!(3615));
/// # Ok::<(), metering::reading::Anomaly>(())
/// ```
pub fn consumption_between(
    start: &MeterReading,
    end: &MeterReading,
    config: &LastgangConfig,
) -> Result<Decimal, Anomaly> {
    let pair = [start.clone(), end.clone()];
    let mut lastgang = to_lastgang(&pair, config);
    match lastgang.anomalies.pop() {
        Some(anomaly) => Err(anomaly),
        // Two readings with no anomaly always yield exactly one interval.
        None => Ok(lastgang
            .intervals
            .first()
            .map_or(Decimal::ZERO, |iv| iv.value)),
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

// ── tests ─────────────────────────────────────────────────────────────────────

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

    // ── the basic difference ─────────────────────────────────────────────────

    /// The identity the whole module rests on: the Lastgang sums to the
    /// difference of the outer Zählerstände.
    #[test]
    fn the_lastgang_sums_to_the_register_difference() {
        let readings = zsg(&[dec!(1000), dec!(1002.5), dec!(1004.8), dec!(1007)]);
        let result = to_lastgang(&readings, &LastgangConfig::strom());

        assert_eq!(result.intervals.len(), 3, "n readings give n−1 intervals");
        assert_eq!(result.total(), dec!(7), "1007 − 1000");
        assert_eq!(result.intervals[0].value, dec!(2.5));
        assert_eq!(result.intervals[1].value, dec!(2.3));
        assert_eq!(result.intervals[2].value, dec!(2.2));
        assert!(result.is_clean());
    }

    /// Each derived interval spans exactly the two readings it came from, and
    /// they tile without gaps.
    #[test]
    fn intervals_tile_the_reading_timestamps() {
        let readings = zsg(&[dec!(0), dec!(1), dec!(2), dec!(3)]);
        let result = to_lastgang(&readings, &LastgangConfig::strom());
        assert_eq!(result.intervals[0].from, readings[0].at);
        for pair in result.intervals.windows(2) {
            assert_eq!(pair[0].to, pair[1].from);
        }
        assert_eq!(result.intervals.last().unwrap().to, readings[3].at);
    }

    /// A single reading is not a series; two are the minimum.
    #[test]
    fn fewer_than_two_readings_yield_nothing() {
        assert!(
            to_lastgang(&[], &LastgangConfig::strom())
                .intervals
                .is_empty()
        );
        let one = zsg(&[dec!(1000)]);
        let result = to_lastgang(&one, &LastgangConfig::strom());
        assert!(result.intervals.is_empty());
        assert!(result.is_clean(), "one reading is short, not corrupt");
    }

    /// A shuffled series must convert correctly rather than producing a run of
    /// negative differences — the failure mode a naive `windows(2)` has.
    #[test]
    fn readings_are_sorted_before_differencing() {
        let ordered = zsg(&[dec!(1000), dec!(1002.5), dec!(1004.8)]);
        let mut shuffled = ordered.clone();
        shuffled.reverse();

        let a = to_lastgang(&ordered, &LastgangConfig::strom());
        let b = to_lastgang(&shuffled, &LastgangConfig::strom());
        assert_eq!(a.intervals, b.intervals);
        assert!(
            b.is_clean(),
            "reordering is not corruption: {:?}",
            b.anomalies
        );
    }

    // ── rollover ─────────────────────────────────────────────────────────────

    /// The rule V10 was trying to be, in the place it can actually work.
    #[test]
    fn a_six_digit_register_wrap_is_reconstructed() {
        let readings = vec![
            MeterReading::measured(datetime!(2026-06-01 0:00 UTC), dec!(999998.5)),
            MeterReading::measured(datetime!(2026-06-01 0:15 UTC), dec!(1.5)),
        ];
        let cfg = LastgangConfig::strom().with_register_digits(6);
        let result = to_lastgang(&readings, &cfg);

        // (1 000 000 − 999 998.5) + 1.5 = 3
        assert_eq!(result.intervals[0].value, dec!(3.0));
        assert_eq!(result.rollovers.len(), 1);
        assert!(result.anomalies.is_empty());

        let rollover = &result.rollovers[0];
        assert_eq!(rollover.register_capacity, dec!(1000000));
        assert_eq!(rollover.delta, dec!(3.0));
        assert_eq!(rollover.at, datetime!(2026-06-01 0:15 UTC));
        assert!(
            !result.is_clean(),
            "a wrap is explained, but still reported"
        );
    }

    /// Without a configured width there is nothing to reconstruct from, and the
    /// module refuses to guess one.
    #[test]
    fn a_backwards_step_without_a_width_is_an_anomaly() {
        let readings = zsg(&[dec!(999998.5), dec!(1.5)]);
        let result = to_lastgang(&readings, &LastgangConfig::strom());
        assert!(result.intervals.is_empty(), "no value is invented");
        assert_eq!(
            result.anomalies[0].kind,
            AnomalyKind::BackwardsWithoutRegisterWidth
        );
    }

    /// A backwards step has two explanations — a wrap and a meter exchange —
    /// and the delta cap is what tells them apart. A meter replaced at 800 000
    /// by one starting at 0 is not 200 000 kWh of consumption in a quarter-hour.
    #[test]
    fn an_implausible_wrap_is_rejected_rather_than_billed() {
        let readings = vec![
            MeterReading::measured(datetime!(2026-06-01 0:00 UTC), dec!(800000)),
            MeterReading::measured(datetime!(2026-06-01 0:15 UTC), dec!(0)),
        ];
        let cfg = LastgangConfig::strom()
            .with_register_digits(6)
            .with_max_delta(dec!(7.5)); // a 30 kW connection, quarter-hourly

        let result = to_lastgang(&readings, &cfg);
        assert!(result.intervals.is_empty());
        assert!(result.rollovers.is_empty());
        assert_eq!(result.anomalies[0].kind, AnomalyKind::ImplausibleRollover);

        // Without the cap the same data yields 200 000 kWh — which is exactly
        // why the cap exists.
        let uncapped = to_lastgang(&readings, &LastgangConfig::strom().with_register_digits(6));
        assert_eq!(uncapped.intervals[0].value, dec!(200000));
    }

    /// A forward jump beyond the connection's capacity is refused too.
    #[test]
    fn an_implausible_forward_delta_is_refused() {
        let readings = zsg(&[dec!(1000), dec!(9000)]);
        let cfg = LastgangConfig::strom().with_max_delta(dec!(7.5));
        let result = to_lastgang(&readings, &cfg);
        assert!(result.intervals.is_empty());
        assert_eq!(result.anomalies[0].kind, AnomalyKind::ImplausibleDelta);
    }

    /// The cap can be derived from the connection capacity, which is how an
    /// operator actually knows it.
    #[test]
    fn the_delta_cap_can_come_from_a_connection_capacity() {
        // 30 kW over a quarter-hour is 7.5 kWh.
        let cfg = LastgangConfig::strom().with_capacity_kw(dec!(30), 900);
        assert_eq!(cfg.max_delta, Some(dec!(7.5)));

        // 30 kW over an hour is 30 kWh.
        let hourly = LastgangConfig::strom().with_capacity_kw(dec!(30), 3600);
        assert_eq!(hourly.max_delta, Some(dec!(30)));

        // 8 kWh in a quarter-hour is 32 kW — over the ceiling.
        let readings = zsg(&[dec!(0), dec!(8)]);
        assert!(!to_lastgang(&readings, &cfg).anomalies.is_empty());
        assert!(to_lastgang(&readings, &hourly).is_clean());
    }

    /// A reading at or above the register's own capacity is not a wrap; the
    /// configured width is simply wrong.
    #[test]
    fn a_reading_wider_than_the_register_is_not_a_wrap() {
        let readings = zsg(&[dec!(50000), dec!(10)]);
        let cfg = LastgangConfig::strom().with_register_digits(4); // wraps at 10 000
        let result = to_lastgang(&readings, &cfg);
        assert_eq!(result.anomalies[0].kind, AnomalyKind::ImplausibleRollover);
    }

    // ── quality and labelling ────────────────────────────────────────────────

    /// A difference is only as good as the readings it came from.
    #[test]
    fn the_worse_endpoint_quality_wins() {
        let mut readings = zsg(&[dec!(0), dec!(2), dec!(4)]);
        readings[1].quality = QualityFlag::Estimated;
        let result = to_lastgang(&readings, &LastgangConfig::strom());
        assert_eq!(result.intervals[0].quality, QualityFlag::Estimated);
        assert_eq!(result.intervals[1].quality, QualityFlag::Estimated);
    }

    #[test]
    fn a_non_billable_endpoint_yields_no_interval() {
        let mut readings = zsg(&[dec!(0), dec!(2), dec!(4)]);
        readings[1].quality = QualityFlag::Faulty;
        let result = to_lastgang(&readings, &LastgangConfig::strom());
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

    /// A Lastgang is a different OBIS channel from the Zählerstand it came
    /// from: D = 29, not D = 8.
    #[test]
    fn the_result_is_labelled_as_a_lastgang() {
        let readings = zsg(&[dec!(0), dec!(2)])
            .into_iter()
            .map(|r| r.with_obis(ObisCode::STROM_BEZUG_TOTAL))
            .collect::<Vec<_>>();

        let labelled = to_lastgang(&readings, &LastgangConfig::strom());
        assert_eq!(
            labelled.intervals[0].obis_code,
            Some(ObisCode::STROM_BEZUG_LASTGANG)
        );
        assert!(labelled.intervals[0].obis_code.unwrap().is_lastgang());

        // ...unless the caller asks for the readings' own code to carry through.
        let passthrough = to_lastgang(&readings, &LastgangConfig::default());
        assert_eq!(
            passthrough.intervals[0].obis_code,
            Some(ObisCode::STROM_BEZUG_TOTAL)
        );
    }

    // ── degenerate input ─────────────────────────────────────────────────────

    #[test]
    fn duplicate_timestamps_are_a_zero_length_span() {
        let at = datetime!(2026-06-01 0:00 UTC);
        let readings = vec![
            MeterReading::measured(at, dec!(100)),
            MeterReading::measured(at, dec!(102)),
        ];
        let result = to_lastgang(&readings, &LastgangConfig::strom());
        assert!(result.intervals.is_empty());
        assert_eq!(result.anomalies[0].kind, AnomalyKind::ZeroLengthSpan);
    }

    /// A flat register is zero consumption, not an anomaly.
    #[test]
    fn an_unchanged_register_is_zero_consumption() {
        let readings = zsg(&[dec!(1000), dec!(1000), dec!(1000)]);
        let result = to_lastgang(&readings, &LastgangConfig::strom());
        assert!(result.is_clean());
        assert_eq!(result.total(), Decimal::ZERO);
        assert!(result.intervals.iter().all(|iv| iv.value.is_zero()));
    }

    /// An absurd register width must not panic or produce a nonsense capacity.
    #[test]
    fn an_absurd_register_width_disables_reconstruction() {
        let readings = zsg(&[dec!(100), dec!(10)]);
        let cfg = LastgangConfig::strom().with_register_digits(99);
        let result = to_lastgang(&readings, &cfg);
        assert_eq!(
            result.anomalies[0].kind,
            AnomalyKind::BackwardsWithoutRegisterWidth
        );
    }

    // ── consumption_between ──────────────────────────────────────────────────

    /// The SLP billing path: two Jahresablesungen a year apart.
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
        assert_eq!(err.kind, AnomalyKind::BackwardsWithoutRegisterWidth);
        assert!(err.to_string().contains("register decreased"), "{err}");
    }

    // ── composition with the rest of the crate ───────────────────────────────

    /// The point of emitting nothing for a bad span: the hole becomes an
    /// ordinary V01 gap, which Ersatzwertbildung then fills with an audit
    /// trail. Nothing has to know it came from a register anomaly.
    #[test]
    fn an_anomalous_span_becomes_a_gap_the_substitute_engine_can_fill() {
        use crate::{
            FillGapsConfig, IntervalResolution, ValidationConfig, fill_gaps, validate_intervals,
        };

        // Five quarter-hourly readings with one corrupt value in the middle.
        // The step down to it is backwards, and the step back up is a 506 kWh
        // jump — 2 MW on a 30 kW connection — so the cap catches both sides.
        let readings = zsg(&[dec!(1000), dec!(1002), dec!(500), dec!(1006), dec!(1008)]);
        let cfg = LastgangConfig::strom().with_capacity_kw(dec!(30), 900);
        let result = to_lastgang(&readings, &cfg);
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

        // Validation sees an ordinary gap.
        let from = datetime!(2026-06-01 0:00 UTC);
        let to = from + Duration::hours(1);
        let cfg = ValidationConfig::default().over_period(from, to);
        let report = validate_intervals(&result.intervals, &cfg);
        assert!(
            report
                .by_rule(crate::ValidationRuleId::GapDetected)
                .next()
                .is_some(),
            "{:?}",
            report.issues
        );

        // ...and Ersatzwertbildung closes it.
        let filled = fill_gaps(
            &result.intervals,
            &FillGapsConfig::new(IntervalResolution::QuarterHour, from, to)
                .because(crate::SubstitutionReason::PlausibilityCheckFailed),
        );
        assert_eq!(filled.intervals.len(), 4);
        assert_eq!(filled.substituted_count(), 2);
    }

    #[test]
    fn anomaly_metadata_is_complete() {
        for kind in AnomalyKind::ALL {
            assert!(!kind.description().is_empty(), "{kind:?}");
        }
    }
}
