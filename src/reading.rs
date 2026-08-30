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
//! ## Rollover belongs here
//!
//! A rollover (Überlauf) is a property of a **register** — a six-digit Zählwerk
//! wraps from 999 999 to 0 — so it can only be detected where readings live.
//! An interval value is not cumulative and has nothing to roll over, which is
//! why [`crate::validation`] leaves the number `V10` unused.
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
use crate::resolution::IntervalResolution;

/// The longest a period of `resolution` can be, in seconds.
///
/// Only used to derive a *ceiling*, where erring long is the safe direction: a
/// cap computed from 24 hours would reject the 25-hour Liefertag on a daily-read
/// meter every autumn.
const fn longest_seconds(resolution: IntervalResolution) -> i64 {
    match resolution {
        // 25 h, 31 days + 1 h, and a leap year.
        IntervalResolution::Day => 25 * 3600,
        IntervalResolution::Month => 31 * 86_400 + 3600,
        IntervalResolution::Year => 366 * 86_400,
        fixed => match fixed.fixed_seconds() {
            Some(s) => s as i64,
            // Unreachable: only the three calendar arms answer `None`.
            None => 0,
        },
    }
}

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
    /// Start of the span the wrap happened in (the earlier reading).
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::rfc3339"))]
    pub from: OffsetDateTime,
    /// End of it — the reading at which the wrap became visible.
    ///
    /// Paired with [`from`](Self::from) so a rollover and an [`Anomaly`]
    /// describe the same shape — *what happened between two readings* — and
    /// can share one audit table.
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

    /// Stable DB/wire label. Matches the `serde` tag and
    /// [`FromStr`](std::str::FromStr) input.
    ///
    /// An anomaly is the audit record for a span that could not be
    /// differenced, and § 146 Abs. 4 AO wants that trail intact — so the code
    /// is a contract rather than a `Debug` rendering.
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

crate::codes::string_codes! {
    AnomalyKind;
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

    /// How the derived intervals are labelled.
    ///
    /// A Lastgang is a different channel from the Zählerstand it came from:
    /// `1-0:1.8.0` is a Zählerstand (D = 8) and `1-0:1.29.0` is the Lastgang
    /// (D = 29). See [`ResultChannel`].
    pub result_channel: ResultChannel,
}

/// What OBIS code [`to_lastgang`] stamps on the intervals it derives.
///
/// The channel matters as much as the value: a **fixed** code turns a feed-in
/// Zählerstandsgang (`1-0:2.8.0`) into an import Lastgang (`1-0:1.29.0`) with
/// the values still right and nothing downstream able to tell.
/// [`Derived`](Self::Derived) reads the channel off the register instead.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum ResultChannel {
    /// Carry each reading's own OBIS code through unchanged.
    ///
    /// Convenient, and strictly speaking a mislabel: the result is a Lastgang
    /// wearing a Zählerstand's code. The default, because it invents nothing.
    #[default]
    Unchanged,

    /// Relabel each interval as the Lastgang of the register it came from —
    /// `1-0:1.8.0` → `1-0:1.29.0`, `1-0:2.8.0` → `1-0:2.29.0`.
    ///
    /// A reading whose code has no Lastgang — a tariff register, a gas code,
    /// none at all — keeps its own. See [`ObisCode::as_lastgang`].
    Derived,

    /// One fixed code on every derived interval, for readings that carry none.
    ///
    /// Prefer [`Derived`](Self::Derived) where the readings are labelled: this
    /// asserts a channel rather than reading it.
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
            result_channel: ResultChannel::Unchanged,
        }
    }
}

impl LastgangConfig {
    /// Electricity: results relabelled as the Lastgang of whichever register
    /// they came from — see [`ResultChannel`].
    ///
    /// No register width and no delta cap — both are properties of the specific
    /// device and connection, and a wrong default is worse than none. Add them
    /// with [`with_register_digits`](Self::with_register_digits) and
    /// [`with_capacity_kw`](Self::with_capacity_kw).
    #[must_use]
    pub const fn strom() -> Self {
        Self {
            register_digits: None,
            max_delta: None,
            result_channel: ResultChannel::Derived,
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

    /// Derive the delta cap from a connection capacity in kW and the reading
    /// **cadence**.
    ///
    /// `max_delta = capacity_kw × cadence_hours`, the most energy the
    /// connection can pass between two readings. This is the same physical
    /// ceiling [`crate::ValidationConfig::max_plant_power_kw`] applies to the
    /// resulting Lastgang, expressed at the point where it can *prevent* a bad
    /// value rather than merely flag one.
    ///
    /// The cadence is an [`IntervalResolution`], not raw seconds — obtain it
    /// from the readings themselves with
    /// [`detect_reading_cadence`].
    ///
    /// A **calendar** resolution has no single length, so the cap uses the
    /// longest that period can be — 25 hours for a day, 366 days for a year.
    /// A ceiling must not reject a legitimate 25-hour Liefertag.
    #[must_use]
    pub fn with_capacity_kw(mut self, capacity_kw: Decimal, cadence: IntervalResolution) -> Self {
        let hours = Decimal::from(longest_seconds(cadence)) / Decimal::from(3600u32);
        self.max_delta = Some(capacity_kw * hours);
        self
    }

    /// Label the derived intervals with one fixed `code`.
    ///
    /// Prefer [`ResultChannel::Derived`], which reads the channel off the
    /// register instead of asserting it.
    #[must_use]
    pub const fn labelled(mut self, code: ObisCode) -> Self {
        self.result_channel = ResultChannel::Fixed(code);
        self
    }

    /// Set how derived intervals are labelled.
    #[must_use]
    pub const fn on_channel(mut self, channel: ResultChannel) -> Self {
        self.result_channel = channel;
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
                from: prev.at,
                to: next.at,
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
            obis_code: config.result_channel.label(next.obis_code),
        });
    }

    Lastgang {
        intervals,
        rollovers,
        anomalies,
    }
}

// ── cadence ───────────────────────────────────────────────────────────────────

/// The reading cadence of a Zählerstandsgang — the spacing of its timestamps.
///
/// The counterpart of
/// [`detect_interval_length`](crate::classification::detect_interval_length),
/// which medians each interval's **duration** — a [`MeterReading`] is a point
/// and has none. The gap between consecutive `at`s is what a reading series has
/// instead, and the `cadence` [`LastgangConfig::with_capacity_kw`] needs.
///
/// Readings are sorted first and the **median** gap is taken, so neither a
/// missed transmission nor an out-of-order merge moves the answer, and the
/// result comes from [`IntervalResolution::from_observed_seconds`] — the same
/// tolerance table [`detect_interval_length`] uses, so a daily series is a
/// calendar [`Day`](IntervalResolution::Day) here and there alike rather than a
/// fixed 86 400 s window in one of them.
///
/// [`detect_interval_length`]: crate::classification::detect_interval_length
///
/// § 2 Satz 1 Nr. 27 MsbG names the two cadences the market defines:
/// *"viertelstündig ermittelter Zählerstände von elektrischer Arbeit und
/// stündlich ermittelter Zählerstände von Gasmengen"*.
///
/// `None` for fewer than two readings, or when every gap is zero.
///
/// ```rust
/// use metering::reading::{MeterReading, detect_reading_cadence};
/// use metering::IntervalResolution;
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
/// assert_eq!(detect_reading_cadence(&zsg), Some(IntervalResolution::QuarterHour));
/// assert_eq!(detect_reading_cadence(&zsg[..1]), None, "one point has no spacing");
/// ```
#[must_use]
pub fn detect_reading_cadence(readings: &[MeterReading]) -> Option<IntervalResolution> {
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
    IntervalResolution::from_observed_seconds(gaps[gaps.len() / 2])
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
        assert_eq!(rollover.from, datetime!(2026-06-01 0:00 UTC));
        assert_eq!(rollover.to, datetime!(2026-06-01 0:15 UTC));
        assert_eq!(rollover.duration(), Duration::minutes(15));
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
        let cfg =
            LastgangConfig::strom().with_capacity_kw(dec!(30), IntervalResolution::QuarterHour);
        assert_eq!(cfg.max_delta, Some(dec!(7.5)));

        // 30 kW over an hour is 30 kWh.
        let hourly = LastgangConfig::strom().with_capacity_kw(dec!(30), IntervalResolution::Hour);
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
        let cfg =
            LastgangConfig::strom().with_capacity_kw(dec!(30), IntervalResolution::QuarterHour);
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

    /// A feed-in Zählerstandsgang keeps its own channel. Stamping a fixed
    /// Bezug code on every derived interval leaves the values right and the
    /// channel a lie, which nothing downstream can detect.
    #[test]
    fn a_feed_in_series_is_not_relabelled_as_import() {
        let readings = zsg(
            datetime!(2026-06-01 0:00 UTC),
            Duration::minutes(15),
            4,
            Some(ObisCode::STROM_EINSPEISUNG_TOTAL),
        );
        let result = to_lastgang(&readings, &LastgangConfig::strom());

        for iv in &result.intervals {
            let code = iv.obis_code.expect("labelled");
            assert_eq!(code, ObisCode::STROM_EINSPEISUNG_LASTGANG);
            assert!(code.is_export() && !code.is_import());
            assert!(code.is_lastgang());
        }

        // ...and a Bezug series still becomes a Bezug Lastgang.
        let bezug = zsg(
            datetime!(2026-06-01 0:00 UTC),
            Duration::minutes(15),
            4,
            Some(ObisCode::STROM_BEZUG_TOTAL),
        );
        assert_eq!(
            to_lastgang(&bezug, &LastgangConfig::strom()).intervals[0].obis_code,
            Some(ObisCode::STROM_BEZUG_LASTGANG)
        );
    }

    /// A register with no Lastgang keeps its own code rather than being forced
    /// onto a channel that would misdescribe it.
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
            let result = to_lastgang(&readings, &LastgangConfig::strom());
            assert_eq!(result.intervals[0].obis_code, Some(code), "{code}");
        }

        // Unlabelled readings stay unlabelled — nothing is invented.
        let bare = zsg(
            datetime!(2026-06-01 0:00 UTC),
            Duration::minutes(15),
            3,
            None,
        );
        assert_eq!(
            to_lastgang(&bare, &LastgangConfig::strom()).intervals[0].obis_code,
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
        let label = |cfg: LastgangConfig| to_lastgang(&readings, &cfg).intervals[0].obis_code;

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
            label(LastgangConfig::default().labelled(ObisCode::GAS_VOLUME_M3)),
            Some(ObisCode::GAS_VOLUME_M3),
            "Fixed does exactly what it says, including when that is wrong"
        );
        assert_eq!(ResultChannel::default(), ResultChannel::Unchanged);
        assert_eq!(ResultChannel::Derived.label(None), None);
    }

    /// A reading is a point, so its series' cadence is the spacing of its
    /// timestamps — the number `with_capacity_kw` needs and that
    /// `detect_interval_length` cannot supply.
    #[test]
    fn the_cadence_comes_from_the_spacing_of_the_readings() {
        let base = datetime!(2026-06-01 0:00 UTC);
        for (step, expected) in [
            (Duration::minutes(15), IntervalResolution::QuarterHour),
            (Duration::minutes(30), IntervalResolution::HalfHour),
            (Duration::hours(1), IntervalResolution::Hour),
        ] {
            assert_eq!(
                detect_reading_cadence(&zsg(base, step, 8, None)),
                Some(expected),
                "{step:?}"
            );
        }

        // Fewer than two readings have no spacing, and duplicates no positive one.
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

    /// The median, so a missed transmission does not move the answer, and the
    /// sort, so an out-of-order series reports its real spacing.
    #[test]
    fn the_cadence_is_robust_to_gaps_and_disorder() {
        let base = datetime!(2026-06-01 0:00 UTC);
        let mut readings = zsg(base, Duration::minutes(15), 12, None);
        readings.remove(5); // one missed reading widens a single gap to 30 min
        assert_eq!(
            detect_reading_cadence(&readings),
            Some(IntervalResolution::QuarterHour)
        );

        readings.reverse();
        assert_eq!(
            detect_reading_cadence(&readings),
            Some(IntervalResolution::QuarterHour),
            "sorted before differencing"
        );
    }

    /// A daily Zählerstandsgang is a **calendar** day, so the cadence survives
    /// the 23- and 25-hour transitions and the derived cap does not reject the
    /// long one.
    #[test]
    fn a_daily_cadence_is_a_calendar_day() {
        let readings: Vec<MeterReading> = (0..6)
            .map(|i| {
                let day = time::macros::date!(2026 - 10 - 23)
                    .checked_add(Duration::days(i))
                    .unwrap();
                MeterReading::measured(
                    crate::calendar::day_start_utc(day),
                    dec!(1000) + Decimal::from(i),
                )
            })
            .collect();
        assert_eq!(
            detect_reading_cadence(&readings),
            Some(IntervalResolution::Day),
            "not a fixed 86 400 s window"
        );

        // The cap for a 30 kW connection on a daily grid allows the 25-hour day.
        let cfg = LastgangConfig::default().with_capacity_kw(dec!(30), IntervalResolution::Day);
        assert_eq!(cfg.max_delta, Some(dec!(750)), "30 kW × 25 h");

        // 24 h × 30 kW = 720 kWh would have rejected a legitimate long day.
        let long_day = vec![
            MeterReading::measured(
                crate::calendar::day_start_utc(time::macros::date!(2026 - 10 - 25)),
                dec!(0),
            ),
            MeterReading::measured(
                crate::calendar::day_end_utc(time::macros::date!(2026 - 10 - 25)),
                dec!(740),
            ),
        ];
        assert!(
            to_lastgang(&long_day, &cfg).is_clean(),
            "a 25-hour day at full load must not read as an anomaly"
        );
    }

    /// The ceiling and the validation rule describe the same physical fact at
    /// the two ends of the pipeline: one prevents, the other flags.
    #[test]
    fn the_two_capacity_ceilings_agree() {
        use crate::{ValidationConfig, ValidationRuleId, validate_intervals};

        let capacity = dec!(30);
        let cfg =
            LastgangConfig::default().with_capacity_kw(capacity, IntervalResolution::QuarterHour);
        assert_eq!(cfg.max_delta, Some(dec!(7.5)), "30 kW × 0.25 h");

        // A value just over the ceiling: refused at the difference...
        let over = vec![
            MeterReading::measured(datetime!(2026-06-01 0:00 UTC), dec!(0)),
            MeterReading::measured(datetime!(2026-06-01 0:15 UTC), dec!(8)),
        ];
        assert!(to_lastgang(&over, &cfg).intervals.is_empty());

        // ...and flagged if it reached the Lastgang some other way.
        let already_formed = to_lastgang(&over, &LastgangConfig::default()).intervals;
        let report = validate_intervals(
            &already_formed,
            &ValidationConfig::default().with_plant_capacity_kw(capacity),
        );
        assert_eq!(
            report.by_rule(ValidationRuleId::ImplausiblePower).count(),
            1,
            "{:?}",
            report.issues
        );
    }
}
