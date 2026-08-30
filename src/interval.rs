//! Core metering types: [`MeterInterval`], [`Sparte`], [`QualityFlag`].
//!
//! ## String forms
//!
//! [`Sparte`], [`MeasurementUnit`] and [`QualityFlag`] each have a stable
//! SCREAMING_SNAKE_CASE code — `as_str`, [`std::fmt::Display`] and
//! [`FromStr`] all agree on it, and it is the same string the
//! `serde` feature emits. Round-tripping is verified for every variant, so a
//! value written to a log, a CLI argument or a database column reads back as
//! itself.

use std::fmt;
use std::str::FromStr;

use rust_decimal::Decimal;
use time::OffsetDateTime;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use crate::error::ParseError;
use crate::obis::ObisCode;

/// Energy commodity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "SCREAMING_SNAKE_CASE"))]
pub enum Sparte {
    /// Electricity.
    #[default]
    Strom,
    /// Natural gas.
    Gas,
    /// Heat (Fern-/Nahwärme, Wärmemengenzähler per EN 1434 / MID MI-004).
    ///
    /// A heat meter integrates flow against the supply/return temperature
    /// difference on-device, so its register holds thermal kWh directly.
    /// Governed by **HeizkostenV**, not MsbG.
    Waerme,
    /// Water (Kalt-/Warmwasser).
    ///
    /// Metered **and billed** in m³ — the only Sparte whose billing unit is a
    /// volume. Water has no calorific value, so the gas m³→kWh path does not
    /// apply to it. For the heat share of warm water see
    /// [`crate::warm_water_heat_kwh`] (HeizkostenV §9 Abs. 2).
    Wasser,
}

impl Sparte {
    /// The unit this Sparte's meter register advances in.
    ///
    /// A gas meter registers m³ of Betriebsvolumen; its energy content is
    /// derived from Brennwert and Zustandszahl. Electricity and heat meters
    /// register energy directly.
    #[must_use]
    pub const fn measured_unit(self) -> MeasurementUnit {
        match self {
            Self::Strom | Self::Waerme => MeasurementUnit::KiloWattHour,
            Self::Gas | Self::Wasser => MeasurementUnit::CubicMetre,
        }
    }

    /// The unit this Sparte is settled and invoiced in.
    ///
    /// Differs from [`Sparte::measured_unit`] only for gas, which is metered in
    /// m³ and billed in kWh.
    #[must_use]
    pub const fn billing_unit(self) -> MeasurementUnit {
        match self {
            Self::Strom | Self::Gas | Self::Waerme => MeasurementUnit::KiloWattHour,
            Self::Wasser => MeasurementUnit::CubicMetre,
        }
    }

    /// `true` when the measured unit differs from the billing unit, so a
    /// reading must be converted before it can be settled. Gas only.
    ///
    /// [`crate::gas_m3_to_kwh_hs`] performs the conversion. An ingest path uses
    /// this to require the conversion parameters up front.
    #[must_use]
    pub const fn requires_conversion(self) -> bool {
        matches!(
            (self.measured_unit(), self.billing_unit()),
            (MeasurementUnit::KiloWattHour, MeasurementUnit::CubicMetre)
                | (MeasurementUnit::CubicMetre, MeasurementUnit::KiloWattHour)
        )
    }

    /// Stable DB/wire label. Matches the `serde` tag and [`FromStr`] input.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Strom => "STROM",
            Self::Gas => "GAS",
            Self::Waerme => "WAERME",
            Self::Wasser => "WASSER",
        }
    }

    /// Every variant, in declaration order.
    pub const ALL: [Self; 4] = [Self::Strom, Self::Gas, Self::Waerme, Self::Wasser];
}

crate::codes::string_codes! {
    // German callers type the umlaut. `WÄRME` is accepted on the way in;
    // `WAERME` stays the one spelling that comes out, because a code that has
    // to survive a database column, a CLI argument and a URL path segment is
    // not the place for a character with three plausible encodings.
    Sparte, aliases = [("WÄRME", Self::Waerme)];
}

/// Unit a meter reading is expressed in.
///
/// Electricity, gas and heat settle in kWh; water settles in m³. Carrying the
/// unit alongside the value keeps the two dimensions distinguishable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum MeasurementUnit {
    /// Kilowatt-hour. Electricity and heat as measured; gas only after
    /// conversion.
    #[default]
    #[cfg_attr(feature = "serde", serde(rename = "KWH"))]
    KiloWattHour,
    /// Cubic metre. Water as measured and billed; gas as measured, before
    /// conversion.
    #[cfg_attr(feature = "serde", serde(rename = "M3"))]
    CubicMetre,
}

impl MeasurementUnit {
    /// Stable DB/wire label. Matches the `serde` tag and [`FromStr`] input.
    ///
    /// Note this is the *canonical storage* code, deliberately narrower than
    /// what [`parse_scaled`](Self::parse_scaled) accepts off the wire.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::KiloWattHour => "KWH",
            Self::CubicMetre => "M3",
        }
    }

    /// Every variant, in declaration order.
    pub const ALL: [Self; 2] = [Self::KiloWattHour, Self::CubicMetre];

    /// Parse a unit string that is already canonical.
    ///
    /// Accepts the superscript `m³` as well as `m3`. Units that need rescaling
    /// (MWh, GJ, litres) are rejected here; use
    /// [`MeasurementUnit::parse_scaled`] for those.
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        let scaled = Self::parse_scaled(s)?;
        scaled.is_canonical().then_some(scaled.unit)
    }

    /// Parse any accepted unit, returning the canonical unit plus the exact
    /// factor that converts a value into it.
    ///
    /// EN 1434-1 cl. 6.3.1 permits heat meters to register in Joules or
    /// Watt-hours and any decimal multiple, so kWh, MWh and GJ registers are all
    /// in service; water submeters commonly report litres. Callers rescale to
    /// the canonical unit before storing, keeping exactly two units in the
    /// persisted data.
    #[must_use]
    pub fn parse_scaled(s: &str) -> Option<UnitScale> {
        let (unit, num, den) = match s.trim().to_lowercase().as_str() {
            // ── Human/device unit symbols, as printed on a meter display ────
            "kwh" | "kwh_th" | "kwh_hs" => (Self::KiloWattHour, 1, 1),
            "wh" => (Self::KiloWattHour, 1, 1_000),
            "mwh" => (Self::KiloWattHour, 1_000, 1),
            "gwh" => (Self::KiloWattHour, 1_000_000, 1),
            // 1 GJ = 1e9 J and 1 kWh = 3.6e6 J, so the factor is 1000/3.6 —
            // a repeating decimal. Held as the exact rational 2500/9 so the
            // conversion does not lose precision on every single reading.
            "gj" => (Self::KiloWattHour, 2_500, 9),
            "mj" => (Self::KiloWattHour, 5, 18),
            "m3" | "m³" | "cbm" => (Self::CubicMetre, 1, 1),
            "l" | "ltr" | "liter" | "litre" => (Self::CubicMetre, 1, 1_000),

            // ── UN/ECE Recommendation 20 codes ──────────────────────────────
            // Used by UTILMD DE6411 and EN 16931/PEPPOL. The codes are not the
            // unit symbols: megajoule is `3B`, gigajoule is `GV`, cubic metre is
            // `MTQ`. Rec 20 also assigns `GJ` to gram per millilitre; the symbol
            // reading wins here because no Sparte modelled in this crate carries
            // a density. Callers emitting Rec 20 should send `GV`.
            "mtq" => (Self::CubicMetre, 1, 1),
            "whr" => (Self::KiloWattHour, 1, 1_000),
            "gv" => (Self::KiloWattHour, 2_500, 9),
            "3b" => (Self::KiloWattHour, 5, 18),
            "jou" => (Self::KiloWattHour, 1, 3_600_000),
            "kjo" => (Self::KiloWattHour, 1, 3_600),
            _ => return None,
        };
        Some(UnitScale { unit, num, den })
    }
}

impl fmt::Display for MeasurementUnit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.pad(self.as_str())
    }
}

impl MeasurementUnit {
    /// The canonical codes [`Display`](fmt::Display) writes.
    ///
    /// Deliberately narrower than what [`FromStr`] accepts: `m³`, `kWh_th` and
    /// the UN/ECE Rec 20 codes all read, and exactly two spellings write.
    pub const CODES: &'static [&'static str] = &["KWH", "M3"];
}

impl FromStr for MeasurementUnit {
    type Err = ParseError;

    /// Parses the canonical codes **and** every unit symbol
    /// [`parse`](Self::parse) accepts, so `"kWh"`, `"KWH"` and `"m³"` all work.
    /// Units needing a rescale (MWh, GJ, litres) are rejected — use
    /// [`parse_scaled`](Self::parse_scaled), which also returns the factor.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s).ok_or_else(|| ParseError::one_of("MeasurementUnit", s, Self::CODES))
    }
}

/// A parsed unit together with the exact rational factor converting a value in
/// that unit into the canonical [`MeasurementUnit`].
///
/// The factor is kept as a numerator/denominator pair rather than a single
/// `Decimal` because the useful ones repeat: 1 GJ is 277.7… kWh. Storing
/// `277.777…8` and multiplying by it would round **twice** — once when the
/// factor was written down and once per reading — and the error would be
/// systematic rather than symmetric.
///
/// Multiplying before dividing rounds **once**, at the end. That makes the
/// conversion exact wherever the quotient terminates, which covers every
/// decimal-power unit (Wh, MWh, GWh, litres) and the identities the rationals
/// are chosen to satisfy — 3.6 GJ is exactly 1 000 kWh, 18 MJ exactly 5 kWh,
/// 3.6 × 10⁶ J exactly 1 kWh. For an input whose quotient does not terminate
/// the result is correctly rounded to `Decimal`'s width, once, and `apply(v) ×
/// den` will not equal `v × num` digit for digit. See the crate-level
/// **What "exact" means here**.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnitScale {
    /// The canonical unit the value converts into.
    pub unit: MeasurementUnit,
    /// Conversion numerator.
    pub num: i64,
    /// Conversion denominator.
    pub den: i64,
}

impl UnitScale {
    /// `true` when the source unit is already canonical (factor 1:1).
    #[must_use]
    pub const fn is_canonical(self) -> bool {
        self.num == self.den
    }

    /// Convert `value` into [`UnitScale::unit`].
    ///
    /// Multiplies before dividing so that a repeating factor such as GJ→kWh
    /// (2500/9) rounds once, at the end, rather than once per operand.
    #[must_use]
    pub fn apply(self, value: Decimal) -> Decimal {
        if self.is_canonical() {
            return value;
        }
        value * Decimal::from(self.num) / Decimal::from(self.den)
    }
}

/// BDEW / MSCONS quality flag.
///
/// Maps to the `MESSWERTSTATUS` field in MSCONS and
/// the BO4E `Messwertstatus` enum.
///
/// ## Billability
///
/// Only `Faulty` and `Unknown` block billing. Everything else is a value
/// somebody stands behind, including the derived ones: an Ersatzwert exists
/// precisely so that a missing measurement does not stop an invoice, and a
/// Prognosewert is the ordinary basis of an Abschlagsrechnung.
///
/// | Flag | Billable | Notes |
/// |---|---|---|
/// | `Measured` | ✓ | Actual reading — highest confidence |
/// | `Estimated` | ✓ | Prognosewert — the basis of an Abschlagsrechnung |
/// | `Substituted` | ✓ | Ersatzwert — see [`crate::substitute`] |
/// | `Calculated` | ✓ | Derived from other measurements (e.g. Residuallast) |
/// | `Corrected` | ✓ | Nachbearbeitet — corrected from an earlier value |
/// | `Preliminary` | ✓ | Vorläufiger Wert — may be revised later |
/// | `Faulty` | ✗ | Fehlerhaft — measurement error, must not be billed |
/// | `Unknown` | ✗ | Quality not determinable — do not bill |
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "SCREAMING_SNAKE_CASE"))]
pub enum QualityFlag {
    /// Reading as measured (Abgelesen / Messwert).
    Measured,
    /// Estimated value (Prognosewert) — the basis of an Abschlagsrechnung
    /// before the final annual read exists.
    /// Also generated by SLP profiling for customers without interval metering.
    Estimated,
    /// Substituted / replaced value (Ersatzwert).
    ///
    /// Generated by the Messstellenbetreiber when measurement failed; see
    /// [`crate::substitute`] for how, and for the statutory basis.
    Substituted,
    /// Calculated / derived value (Vorlaeufiger Wert / Rechenwert).
    ///
    /// Derived from other meter readings (e.g. Residuallast = Bezug − Einspeisung).
    Calculated,
    /// Corrected value (Nachbearbeitungswert).
    ///
    /// Originally measured/estimated, subsequently corrected by MSB.
    /// Replaces a prior value — billable, supersedes earlier reading.
    Corrected,
    /// Preliminary value (Vorläufiger Wert).
    ///
    /// Valid for billing but may be revised. Record as provisional in invoice.
    Preliminary,
    /// Faulty measurement (Fehlerhaft / Unplausibel).
    ///
    /// Must NOT be used for billing. Requires substitute value generation.
    Faulty,
    /// Quality not known.
    #[default]
    Unknown,
}

impl QualityFlag {
    /// `true` when this flag indicates the value is reliable for billing.
    ///
    /// `Estimated` (Prognosewert) is billable: it is the ordinary basis for an
    /// Abschlagsrechnung before the annual read exists. Excluding it would
    /// produce a zero Arbeitsmenge for every SLP delivery point and for every
    /// measurement outage, which is not a conservative result — it is a wrong
    /// one. Only `Faulty` and `Unknown` block billing.
    #[must_use]
    pub const fn is_billable(self) -> bool {
        matches!(
            self,
            Self::Measured
                | Self::Estimated
                | Self::Substituted
                | Self::Calculated
                | Self::Corrected
                | Self::Preliminary
        )
    }

    /// `true` when this value should be flagged as provisional in invoices.
    ///
    /// Preliminary values are billable but the invoice should note they may be revised.
    #[must_use]
    pub const fn is_provisional(self) -> bool {
        matches!(self, Self::Preliminary | Self::Estimated)
    }

    /// How far this flag is from a plain measured value, on a **strict** total
    /// order from `0` ([`Measured`](Self::Measured)) to `7`
    /// ([`Unknown`](Self::Unknown)).
    ///
    /// Aggregating a set of intervals into one bucket has to give the bucket a
    /// single flag, and the only defensible choice is the worst contributor —
    /// a daily total containing one substitute value is not a measured daily
    /// total. The ranking is public so that every aggregation in and outside
    /// this crate reaches the same verdict.
    ///
    /// **No two flags share a rank**, and that is load-bearing rather than
    /// tidy. [`worse_of`](Self::worse_of) keeps `self` on a tie, so a shared
    /// rank would make [`worst_of`](Self::worst_of) — and with it every bucket
    /// quality in [`mod@crate::resample`] and [`crate::virtual_meter`] — depend
    /// on the order the caller supplied the intervals in.
    ///
    /// The order is by how far the value is from a measurement, not by
    /// billability:
    ///
    /// | Rank | Flag | Why here |
    /// |---|---|---|
    /// | 0 | `Measured` | the measurement itself |
    /// | 1 | `Calculated` | derived from measurements, arithmetically |
    /// | 2 | `Corrected` | measured, then revised — a measurement stands behind it |
    /// | 3 | `Substituted` | never measured; reconstructed from other values |
    /// | 4 | `Estimated` | a forecast, not a reconstruction |
    /// | 5 | `Preliminary` | billable, and explicitly subject to revision |
    /// | 6 | `Faulty` | known bad |
    /// | 7 | `Unknown` | not even known to be bad |
    ///
    /// ```rust
    /// use metering::QualityFlag;
    ///
    /// // Distinct ranks, so the worst of a set does not depend on its order.
    /// let a = [QualityFlag::Corrected, QualityFlag::Substituted];
    /// let b = [QualityFlag::Substituted, QualityFlag::Corrected];
    /// assert_eq!(QualityFlag::worst_of(a), QualityFlag::worst_of(b));
    /// ```
    #[must_use]
    pub const fn severity_rank(self) -> u8 {
        match self {
            Self::Measured => 0,
            Self::Calculated => 1,
            Self::Corrected => 2,
            Self::Substituted => 3,
            Self::Estimated => 4,
            Self::Preliminary => 5,
            Self::Faulty => 6,
            Self::Unknown => 7,
        }
    }

    /// The worse of two flags, by [`severity_rank`](Self::severity_rank).
    ///
    /// Commutative, associative and idempotent — the ranks are distinct — so
    /// folding it over a set is order-independent.
    #[must_use]
    pub const fn worse_of(self, other: Self) -> Self {
        if other.severity_rank() > self.severity_rank() {
            other
        } else {
            self
        }
    }

    /// The worst flag across an iterator, or [`Unknown`](Self::Unknown) when it
    /// is empty.
    ///
    /// An empty set has no measurement to speak for it, so the neutral answer
    /// is "not known" rather than "measured" — and `Unknown` being the maximum
    /// rank makes that the same answer either way.
    #[must_use]
    pub fn worst_of(flags: impl IntoIterator<Item = Self>) -> Self {
        flags
            .into_iter()
            .reduce(Self::worse_of)
            .unwrap_or(Self::Unknown)
    }

    /// Stable DB/wire label. Matches the `serde` tag and [`FromStr`] input.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Measured => "MEASURED",
            Self::Estimated => "ESTIMATED",
            Self::Substituted => "SUBSTITUTED",
            Self::Calculated => "CALCULATED",
            Self::Corrected => "CORRECTED",
            Self::Preliminary => "PRELIMINARY",
            Self::Faulty => "FAULTY",
            Self::Unknown => "UNKNOWN",
        }
    }

    /// Every variant, in declaration order.
    pub const ALL: [Self; 8] = [
        Self::Measured,
        Self::Estimated,
        Self::Substituted,
        Self::Calculated,
        Self::Corrected,
        Self::Preliminary,
        Self::Faulty,
        Self::Unknown,
    ];
}

crate::codes::string_codes! {
    // Deliberately no aliases: an unrecognised status is not `UNKNOWN`, which
    // is a statement about the measurement, but a parse failure, which is a
    // statement about the message. Mapping one to the other silently would
    // turn a malformed MSCONS field into an unbillable reading with no error.
    QualityFlag;
}

// ── Direction ─────────────────────────────────────────────────────────────────

/// Which way energy crossed the measurement point.
///
/// ## Why this is derived and not a field
///
/// A bidirectional Zählpunkt — a charge point that also discharges, a battery,
/// a PV roof behind the same meter — delivers import *and* export for the same
/// quarter-hour, and a settlement has to keep the two apart to the kWh. The
/// market already keeps them apart, in value group C of the OBIS code:
/// `1-0:1.8.x` counts Bezug and `1-0:2.8.x` counts Lieferung, on two registers,
/// and MSCONS carries one time series per register.
///
/// So a `direction` field on [`MeterInterval`] would be a second, separately
/// mutable copy of something [`obis_code`](MeterInterval::obis_code) already
/// states — the same objection that keeps the *unit* off that type. Two copies
/// of one fact disagree eventually, and nothing reports it. The direction is
/// therefore read off the code, by [`ObisCode::direction`] and
/// [`MeterInterval::direction`], and a **signed** interval is not offered at
/// all: a negative kWh in this crate means a Korrekturenergiemenge
/// (EDI@Energy *Codeliste* v2.5c §2.1), not a reversed flow, and overloading
/// the sign would make those two indistinguishable.
///
/// What a caller actually needs from a bidirectional point is the balance, and
/// that is [`sum_by_direction`](crate::aggregation::sum_by_direction).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "SCREAMING_SNAKE_CASE"))]
pub enum Direction {
    /// Bezug — energy drawn *from* the grid. OBIS value group C = 1.
    Import,
    /// Einspeisung / Rücklieferung — energy fed *into* the grid. C = 2.
    Export,
}

impl Direction {
    /// Every variant, in declaration order.
    pub const ALL: [Self; 2] = [Self::Import, Self::Export];

    /// Stable DB/wire label. Matches the `serde` tag and [`FromStr`] input.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Import => "IMPORT",
            Self::Export => "EXPORT",
        }
    }

    /// The German market term: *Bezug* and *Einspeisung*.
    ///
    /// A description, not a code — [`as_str`](Self::as_str) is what gets
    /// stored, this is what gets printed on a German-language report.
    #[must_use]
    pub const fn bezeichnung(self) -> &'static str {
        match self {
            Self::Import => "Bezug",
            Self::Export => "Einspeisung",
        }
    }

    /// The other direction.
    #[must_use]
    pub const fn reversed(self) -> Self {
        match self {
            Self::Import => Self::Export,
            Self::Export => Self::Import,
        }
    }
}

crate::codes::string_codes! {
    // `BEZUG` / `EINSPEISUNG` are the market's own words and are read on the
    // way in; `IMPORT` / `EXPORT` are what this crate writes, so that a stored
    // direction needs no umlaut-free transliteration and matches the OBIS
    // helpers `is_import` / `is_export` that answer the same question.
    Direction, aliases = [("BEZUG", Self::Import), ("EINSPEISUNG", Self::Export)];
}

/// A single metered interval — the energy or volume *in* a period.
///
/// This is the Lastgang. For the cumulative Zählerstand it is derived from, and
/// the conversion between them, see [`crate::reading`].
///
/// ## The unit is the Sparte's, and this type does not carry it
///
/// [`value`](Self::value) is kWh for Strom, kWh_Hs for Gas *after* conversion,
/// kWh_th for Wärme and **m³ for Wasser** — whatever
/// [`Sparte::billing_unit`] says. The field is named `value` rather than
/// `value_kwh` for exactly that reason: `Sparte::Wasser` is a supported
/// Sparte, water is billed in cubic metres, and a field named `_kwh` holding
/// m³ is a lie the compiler cannot catch.
///
/// The unit lives on the [`MeasurementPoint`](crate::MeasurementPoint) or the
/// OBIS medium, not on every interval — carrying it here would put a
/// redundant, separately-mutable copy of it on the hottest type in the crate.
/// Use [`crate::conversion::gas_m3_to_kwh_hs`] to convert a gas volume before
/// building intervals from it.
///
/// One consequence worth naming: [`demand_kw`](Self::demand_kw) is only
/// meaningful where the unit is energy. Dividing cubic metres by hours gives
/// m³/h, which is a flow rate, not a power.
///
/// `obis_code` is a parsed [`ObisCode`], not a string: the same channel
/// identifier must have the same type and the same value wherever it appears,
/// so that `MeterInterval` and [`crate::MeasurementSeries`] can be compared and
/// stored without a parse that might fail on data already accepted. Parse at
/// the boundary — `"1-0:1.8.0".parse()` — and an unparseable code is
/// rejected there, where the message is still available to report.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct MeterInterval {
    /// Interval start (UTC, inclusive).
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::rfc3339"))]
    pub from: OffsetDateTime,
    /// Interval end (UTC, exclusive).
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::rfc3339"))]
    pub to: OffsetDateTime,
    /// The metered quantity, in the Sparte's own unit — see the type docs.
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::decimal"))]
    pub value: Decimal,
    /// Reading quality.
    pub quality: QualityFlag,
    /// OBIS-Kennzahl identifying the measurement channel.
    /// `None` when not provided by MSCONS.
    pub obis_code: Option<ObisCode>,
}

impl MeterInterval {
    /// Duration in whole seconds.
    #[must_use]
    pub fn duration_secs(&self) -> i64 {
        (self.to - self.from).whole_seconds()
    }

    /// The Europe/Berlin calendar day this interval starts on.
    ///
    /// The correct grouping key for daily aggregation — see
    /// [`crate::calendar`] for why `self.from.date()` is not.
    #[must_use]
    pub fn berlin_day(&self) -> time::Date {
        crate::calendar::local_day(self.from)
    }

    /// Duration in minutes.
    #[must_use]
    pub fn duration_minutes(&self) -> i64 {
        (self.to - self.from).whole_minutes()
    }

    /// Instantaneous demand in kW, computed as `kWh ÷ (duration_h)`.
    ///
    /// Only meaningful for RLM intervals (15-min or 60-min).
    /// For a 15-min interval carrying 2.5 kWh: demand = 2.5 × 4 = 10 kW.
    #[must_use]
    pub fn demand_kw(&self) -> Option<Decimal> {
        let h = Decimal::from(self.duration_secs()) / Decimal::from(3600u32);
        if h.is_zero() {
            None
        } else {
            Some(self.value / h)
        }
    }

    /// Which way the energy flowed, read off the OBIS code.
    ///
    /// `None` when the interval carries no OBIS code, and when the code it
    /// carries has no direction — a reactive register, a gas volume, a
    /// Zustandszahl. Three answers, not two: "not stated" and "neither" are
    /// both real, and a pair of booleans that are both `false` cannot say
    /// which.
    ///
    /// ## Example
    ///
    /// ```rust
    /// use metering::{Direction, MeterInterval, QualityFlag};
    /// use metering::obis::ObisCode;
    /// use rust_decimal::dec;
    /// use time::macros::datetime;
    ///
    /// let iv = MeterInterval {
    ///     from: datetime!(2026-01-01 0:00 UTC),
    ///     to:   datetime!(2026-01-01 0:15 UTC),
    ///     value: dec!(2.5),
    ///     quality: QualityFlag::Measured,
    ///     obis_code: Some("1-0:1.8.0".parse().unwrap()),
    /// };
    /// assert_eq!(iv.obis_code, Some(ObisCode::STROM_BEZUG_TOTAL));
    /// assert_eq!(iv.direction(), Some(Direction::Import));
    /// ```
    #[must_use]
    pub fn direction(&self) -> Option<Direction> {
        self.obis_code.and_then(ObisCode::direction)
    }

    /// Tariff register number from the OBIS code (`None` = total, `Some(1)` = HT, `Some(2)` = NT).
    #[must_use]
    pub fn tariff_register(&self) -> Option<u8> {
        self.obis_code?.tariff_register()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal::dec;
    use time::macros::datetime;

    #[test]
    fn demand_kw_15min_interval() {
        let iv = MeterInterval {
            from: datetime!(2026-01-01 0:00 UTC),
            to: datetime!(2026-01-01 0:15 UTC),
            value: dec!(2.5),
            quality: QualityFlag::Measured,
            obis_code: None,
        };
        // 2.5 kWh in 15 min = 10 kW
        assert_eq!(iv.demand_kw(), Some(dec!(10)));
    }

    #[test]
    fn demand_kw_hourly() {
        let iv = MeterInterval {
            from: datetime!(2026-01-01 0:00 UTC),
            to: datetime!(2026-01-01 1:00 UTC),
            value: dec!(5.0),
            quality: QualityFlag::Measured,
            obis_code: None,
        };
        // 5.0 kWh in 60 min = 5 kW
        assert_eq!(iv.demand_kw(), Some(dec!(5)));
    }

    #[test]
    fn quality_flag_billable() {
        assert!(QualityFlag::Measured.is_billable());
        assert!(QualityFlag::Substituted.is_billable());
        // Estimated (Prognosewert) is billable — it is what an Abschlag rests on.
        assert!(
            QualityFlag::Estimated.is_billable(),
            "Estimated must be billable"
        );
        assert!(QualityFlag::Corrected.is_billable());
        assert!(QualityFlag::Preliminary.is_billable());
        // Only Faulty and Unknown block billing
        assert!(!QualityFlag::Faulty.is_billable());
        assert!(!QualityFlag::Unknown.is_billable());
    }

    /// Distinct ranks are what make `worst_of` order-independent — the
    /// property every bucket quality in the crate rests on.
    #[test]
    fn severity_ranks_are_a_strict_total_order() {
        let ranks: Vec<u8> = QualityFlag::ALL.iter().map(|q| q.severity_rank()).collect();
        let unique: std::collections::BTreeSet<u8> = ranks.iter().copied().collect();
        assert_eq!(
            unique.len(),
            QualityFlag::ALL.len(),
            "every flag needs its own rank, or worse_of breaks ties by argument order: {ranks:?}"
        );

        // Commutative over every pair, which is exactly what a tie would break.
        for a in QualityFlag::ALL {
            for b in QualityFlag::ALL {
                assert_eq!(a.worse_of(b), b.worse_of(a), "{a} vs {b}");
            }
        }

        // ...so shuffling a set cannot change its worst flag.
        let set = QualityFlag::ALL;
        let mut reversed = set;
        reversed.reverse();
        assert_eq!(QualityFlag::worst_of(set), QualityFlag::worst_of(reversed));
        assert_eq!(QualityFlag::worst_of(set), QualityFlag::Unknown);
        assert_eq!(QualityFlag::worst_of([]), QualityFlag::Unknown);

        // A measurement outranks everything derived from one.
        assert_eq!(QualityFlag::Measured.severity_rank(), 0);
        assert!(
            QualityFlag::Corrected.severity_rank() < QualityFlag::Substituted.severity_rank(),
            "a corrected value has a measurement behind it; a substitute does not"
        );
    }

    #[test]
    fn quality_flag_provisional() {
        assert!(QualityFlag::Estimated.is_provisional());
        assert!(QualityFlag::Preliminary.is_provisional());
        assert!(!QualityFlag::Measured.is_provisional());
        assert!(!QualityFlag::Substituted.is_provisional());
    }
}

#[cfg(test)]
mod media_tests {
    use super::*;

    /// Water is m³ and heat is kWh_th — the distinction the `unit` column exists
    /// to preserve.
    #[test]
    fn measured_unit_is_what_the_register_counts() {
        // Gas and water meters register a volume.
        assert_eq!(Sparte::Gas.measured_unit(), MeasurementUnit::CubicMetre);
        assert_eq!(Sparte::Wasser.measured_unit(), MeasurementUnit::CubicMetre);
        // A heat meter integrates flow × ΔT on-device, so its register is kWh_th.
        assert_eq!(
            Sparte::Waerme.measured_unit(),
            MeasurementUnit::KiloWattHour
        );
        assert_eq!(Sparte::Strom.measured_unit(), MeasurementUnit::KiloWattHour);
    }

    #[test]
    fn billing_unit_diverges_from_measured_unit_only_for_gas() {
        for sparte in [Sparte::Strom, Sparte::Gas, Sparte::Waerme, Sparte::Wasser] {
            assert_eq!(
                sparte.requires_conversion(),
                sparte == Sparte::Gas,
                "{} conversion requirement",
                sparte.as_str()
            );
            assert_eq!(
                sparte.measured_unit() != sparte.billing_unit(),
                sparte.requires_conversion(),
                "{}: requires_conversion must track the unit divergence",
                sparte.as_str()
            );
        }

        // Water is the one Sparte billed in a volume.
        assert_eq!(Sparte::Wasser.billing_unit(), MeasurementUnit::CubicMetre);
        assert_eq!(Sparte::Gas.billing_unit(), MeasurementUnit::KiloWattHour);
    }

    /// Both spellings of the cubic metre are in use on the wire.
    #[test]
    fn unit_parse_accepts_superscript_cubic_metre() {
        assert_eq!(
            MeasurementUnit::parse("m³"),
            Some(MeasurementUnit::CubicMetre)
        );
        assert_eq!(
            MeasurementUnit::parse("m3"),
            Some(MeasurementUnit::CubicMetre)
        );
        assert_eq!(
            MeasurementUnit::parse(" M3 "),
            Some(MeasurementUnit::CubicMetre)
        );
        assert_eq!(
            MeasurementUnit::parse("kWh_th"),
            Some(MeasurementUnit::KiloWattHour)
        );
        assert_eq!(MeasurementUnit::parse("furlong"), None);
    }

    /// `parse` rejects any unit it would have to rescale, so a caller cannot
    /// read MWh as kWh.
    #[test]
    fn strict_parse_rejects_units_needing_rescaling() {
        assert_eq!(MeasurementUnit::parse("MWh"), None);
        assert_eq!(MeasurementUnit::parse("GJ"), None);
        assert_eq!(MeasurementUnit::parse("l"), None);
        // ...but the scaled parser accepts them.
        assert!(MeasurementUnit::parse_scaled("MWh").is_some());
        assert!(MeasurementUnit::parse_scaled("GJ").is_some());
    }

    /// GJ→kWh is 2500/9, a repeating decimal. Holding it as a rational and
    /// multiplying before dividing keeps the *defining* identities exact —
    /// 3.6 GJ is 1 000 kWh and 9 GJ is 2 500 kWh, to the digit — where a
    /// stored `277.777…8` factor would not.
    #[test]
    fn gigajoule_conversion_is_exact() {
        let gj = MeasurementUnit::parse_scaled("GJ").unwrap();
        assert_eq!(gj.unit, MeasurementUnit::KiloWattHour);
        assert!(!gj.is_canonical());

        // 3.6 GJ is exactly 1000 kWh — the identity that defines the factor.
        let kwh = gj.apply(Decimal::from_str_exact("3.6").unwrap());
        assert_eq!(
            kwh,
            Decimal::from(1000u32),
            "3.6 GJ must be exactly 1000 kWh"
        );

        // 9 GJ is exactly 2500 kWh — no residue from the repeating factor.
        assert_eq!(gj.apply(Decimal::from(9u32)), Decimal::from(2500u32));
    }

    /// Rec 20 codes are not the unit symbols, so each mapping is pinned.
    #[test]
    fn unece_rec20_codes_do_not_follow_the_obvious_mnemonic() {
        let kwh = MeasurementUnit::KiloWattHour;

        // Gigajoule is `GV`.
        let gv = MeasurementUnit::parse_scaled("GV").unwrap();
        assert_eq!((gv.unit, gv.num, gv.den), (kwh, 2_500, 9));

        // Megajoule is `3B`.
        let mj = MeasurementUnit::parse_scaled("3B").unwrap();
        assert_eq!((mj.unit, mj.num, mj.den), (kwh, 5, 18));
        assert_eq!(mj, MeasurementUnit::parse_scaled("MJ").unwrap());

        // Cubic metre is `MTQ`.
        assert_eq!(
            MeasurementUnit::parse_scaled("MTQ").unwrap().unit,
            MeasurementUnit::CubicMetre
        );

        // Joule round-trips exactly: 3.6e6 J is 1 kWh.
        let jou = MeasurementUnit::parse_scaled("JOU").unwrap();
        assert_eq!(jou.apply(Decimal::from(3_600_000u32)), Decimal::ONE);
    }

    #[test]
    fn scaled_units_cover_the_real_device_population() {
        let cases = [
            ("MWh", "0.5", "500"), // ista ultego III smart displays 0,01 MWh
            ("Wh", "2500", "2.5"),
            ("MJ", "18", "5"),    // 18 MJ = 5 kWh exactly
            ("l", "1500", "1.5"), // water submeters commonly report litres
            ("kWh", "42", "42"),  // canonical passes through untouched
        ];
        for (unit, input, expected) in cases {
            let scale =
                MeasurementUnit::parse_scaled(unit).unwrap_or_else(|| panic!("{unit} must parse"));
            assert_eq!(
                scale.apply(Decimal::from_str_exact(input).unwrap()),
                Decimal::from_str_exact(expected).unwrap(),
                "{input} {unit}"
            );
        }
    }

    /// Labels are the DB CHECK values.
    #[test]
    fn labels_match_the_db_check_values() {
        assert_eq!(Sparte::Waerme.as_str(), "WAERME");
        assert_eq!(Sparte::Wasser.as_str(), "WASSER");
        assert_eq!(MeasurementUnit::KiloWattHour.as_str(), "KWH");
        assert_eq!(MeasurementUnit::CubicMetre.as_str(), "M3");
    }
}

#[cfg(test)]
mod code_round_trip_tests {
    use super::*;

    /// `as_str`, `Display` and `FromStr` must agree over **every** variant, so a
    /// value that goes out to a log, a CLI or a database column comes back as
    /// itself. `ALL` is exhaustive by construction: adding a variant without
    /// extending it leaves the count assertion failing.
    #[test]
    fn every_variant_round_trips() {
        assert_eq!(Sparte::ALL.len(), Sparte::CODES.len());
        for (v, code) in Sparte::ALL.iter().zip(Sparte::CODES) {
            assert_eq!(v.as_str(), *code);
            assert_eq!(v.to_string(), *code);
            assert_eq!(&v.to_string().parse::<Sparte>().unwrap(), v);
        }

        assert_eq!(QualityFlag::ALL.len(), QualityFlag::CODES.len());
        for (v, code) in QualityFlag::ALL.iter().zip(QualityFlag::CODES) {
            assert_eq!(v.as_str(), *code);
            assert_eq!(v.to_string(), *code);
            assert_eq!(&v.to_string().parse::<QualityFlag>().unwrap(), v);
        }

        for (v, code) in MeasurementUnit::ALL.iter().zip(MeasurementUnit::CODES) {
            assert_eq!(v.as_str(), *code);
            assert_eq!(&v.to_string().parse::<MeasurementUnit>().unwrap(), v);
        }
    }

    /// Every variant needs a distinct code, or the round trip above would pass
    /// while two variants collapsed into one.
    #[test]
    fn codes_are_unique_within_each_type() {
        for codes in [QualityFlag::CODES, Sparte::CODES, MeasurementUnit::CODES] {
            let unique: std::collections::BTreeSet<_> = codes.iter().collect();
            assert_eq!(
                unique.len(),
                codes.len(),
                "codes must be distinct: {codes:?}"
            );
        }
    }

    /// `ALL` is the crate's promise that exhaustive iteration is supported —
    /// see the crate-level **Enum exhaustiveness** section. A new variant added
    /// without extending `ALL` and `CODES` together fails here.
    #[test]
    fn all_and_codes_stay_in_step() {
        assert_eq!(Sparte::ALL.len(), Sparte::CODES.len());
        assert_eq!(QualityFlag::ALL.len(), QualityFlag::CODES.len());
        assert_eq!(MeasurementUnit::ALL.len(), MeasurementUnit::CODES.len());
        assert_eq!(
            crate::LoadProfile::ALL.len(),
            crate::LoadProfile::CODES.len()
        );
        for (v, code) in crate::LoadProfile::ALL
            .iter()
            .zip(crate::LoadProfile::CODES)
        {
            assert_eq!(v.as_str(), *code, "{v:?}");
            assert_eq!(&code.parse::<crate::LoadProfile>().unwrap(), v);
        }
    }

    #[test]
    fn parsing_is_case_and_whitespace_insensitive() {
        assert_eq!("  strom ".parse::<Sparte>().unwrap(), Sparte::Strom);
        assert_eq!(
            "Measured".parse::<QualityFlag>().unwrap(),
            QualityFlag::Measured
        );
        assert_eq!(
            "kWh".parse::<MeasurementUnit>().unwrap(),
            MeasurementUnit::KiloWattHour
        );
    }

    /// An unrecognised status is a parse failure, not `Unknown`: the latter is a
    /// statement about the measurement, the former about the message.
    #[test]
    fn unrecognised_codes_are_errors_not_defaults() {
        let err = "GARBAGE".parse::<QualityFlag>().unwrap_err();
        assert_eq!(err.type_name(), "QualityFlag");
        assert_eq!(err.input(), "GARBAGE");
        assert_eq!(err.expected_values(), Some(QualityFlag::CODES));
        assert!(err.to_string().contains("MEASURED"), "{err}");

        assert!("".parse::<Sparte>().is_err());
        assert!("KOHLE".parse::<Sparte>().is_err());
        // A unit needing a rescale is rejected here — `parse_scaled` handles it.
        assert!("MWh".parse::<MeasurementUnit>().is_err());
    }

    /// The OBIS code on an interval is parsed, so an invalid one cannot be
    /// constructed at all — the failure surfaces at the boundary instead.
    #[test]
    fn interval_obis_code_is_typed() {
        use rust_decimal::dec;
        use time::macros::datetime;

        let iv = MeterInterval {
            from: datetime!(2026-01-01 0:00 UTC),
            to: datetime!(2026-01-01 0:15 UTC),
            value: dec!(2.5),
            quality: QualityFlag::Measured,
            obis_code: Some(ObisCode::STROM_BEZUG_HT),
        };
        assert_eq!(iv.tariff_register(), Some(1));
        assert_eq!(iv.direction(), Some(Direction::Import));

        // An unparseable code fails at the boundary, not later.
        assert!("not an obis code".parse::<ObisCode>().is_err());
    }
}
