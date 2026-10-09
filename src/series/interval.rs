//! Core metering types: [`MeterInterval`], [`Sparte`], [`Unit`],
//! [`QualityFlag`], [`Direction`].
//!
//! Each enum has one stable SCREAMING_SNAKE_CASE code on which `as_str`,
//! [`std::fmt::Display`], [`FromStr`] and the `serde` form agree.

use std::fmt;
use std::str::FromStr;

use rust_decimal::Decimal;
use time::OffsetDateTime;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use crate::error::ParseError;
use crate::ids::obis::ObisCode;

/// Energy commodity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Sparte {
    /// Electricity.
    Strom,
    /// Natural gas.
    Gas,
    /// Heat (Fern-/Nahwärme, Wärmemengenzähler per EN 1434 / MID MI-004),
    /// registered directly in thermal kWh. Governed by **HeizkostenV**, not MsbG.
    Waerme,
    /// Water (Kalt-/Warmwasser), metered **and billed** in m³. For the heat
    /// share of warm water see [`crate::heat::heizkosten::warm_water_heat_kwh`]
    /// (HeizkostenV §9 Abs. 2).
    Wasser,
}

impl Sparte {
    /// The unit this Sparte's meter register advances in (m³ Betriebsvolumen
    /// for gas).
    #[must_use]
    pub const fn measured_unit(self) -> Unit {
        match self {
            Self::Strom | Self::Waerme => Unit::KiloWattHour,
            Self::Gas | Self::Wasser => Unit::CubicMetre,
        }
    }

    /// The unit this Sparte is settled and invoiced in; differs from
    /// [`Sparte::measured_unit`] only for gas.
    #[must_use]
    pub const fn billing_unit(self) -> Unit {
        match self {
            Self::Strom | Self::Gas | Self::Waerme => Unit::KiloWattHour,
            Self::Wasser => Unit::CubicMetre,
        }
    }

    /// `true` when the measured unit differs from the billing unit (gas only),
    /// so a reading must go through [`crate::gas::conversion::gas_m3_to_kwh_hs`].
    #[must_use]
    pub const fn requires_conversion(self) -> bool {
        matches!(
            (self.measured_unit(), self.billing_unit()),
            (Unit::KiloWattHour, Unit::CubicMetre) | (Unit::CubicMetre, Unit::KiloWattHour)
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

crate::ids::codes::string_codes! {
    // `WÄRME` is read; only the ASCII `WAERME` is written, so the stored
    // code has one encoding.
    Sparte, aliases = [("WÄRME", Self::Waerme)];
}

/// The unit of an interval value, a register reading or an OBIS channel
/// ([`ObisCode::unit`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Unit {
    /// Active (or thermal) energy, kWh.
    KiloWattHour,
    /// Reactive energy, kvarh.
    KiloVarHour,
    /// Active power, kW.
    KiloWatt,
    /// Reactive power, kvar.
    KiloVar,
    /// Volume, m³ — gas before conversion, water.
    CubicMetre,
    /// Calorific value (Brennwert), kWh/m³.
    KiloWattHourPerCubicMetre,
    /// A dimensionless ratio (the gas Zustandszahl).
    Dimensionless,
}

impl Unit {
    /// Every variant, in declaration order.
    pub const ALL: [Self; 7] = [
        Self::KiloWattHour,
        Self::KiloVarHour,
        Self::KiloWatt,
        Self::KiloVar,
        Self::CubicMetre,
        Self::KiloWattHourPerCubicMetre,
        Self::Dimensionless,
    ];

    /// Stable DB/wire code. Matches the `serde` tag and [`FromStr`] input.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::KiloWattHour => "KWH",
            Self::KiloVarHour => "KVARH",
            Self::KiloWatt => "KW",
            Self::KiloVar => "KVAR",
            Self::CubicMetre => "M3",
            Self::KiloWattHourPerCubicMetre => "KWH_PER_M3",
            Self::Dimensionless => "ONE",
        }
    }

    /// The unit symbol, as printed on a meter display.
    #[must_use]
    pub const fn symbol(self) -> &'static str {
        match self {
            Self::KiloWattHour => "kWh",
            Self::KiloVarHour => "kvarh",
            Self::KiloWatt => "kW",
            Self::KiloVar => "kvar",
            Self::CubicMetre => "m³",
            Self::KiloWattHourPerCubicMetre => "kWh/m³",
            Self::Dimensionless => "1",
        }
    }

    /// `true` for an energy — kWh or kvarh.
    #[must_use]
    pub const fn is_energy(self) -> bool {
        matches!(self, Self::KiloWattHour | Self::KiloVarHour)
    }

    /// `true` for an energy or a volume — a quantity a register accumulates
    /// and a Lastgang can be differenced from.
    #[must_use]
    pub const fn is_cumulative(self) -> bool {
        matches!(
            self,
            Self::KiloWattHour | Self::KiloVarHour | Self::CubicMetre
        )
    }

    /// Parse an energy or volume symbol (Wh…GWh, MJ, GJ, m³, litres, and their
    /// UN/ECE Rec. 20 codes), case-insensitively, into the canonical kWh or m³
    /// plus the exact factor converting into it. `None` for anything else.
    ///
    /// EN 1434-1 cl. 6.3.1 lets heat meters register in J or Wh and any
    /// decimal multiple.
    #[must_use]
    pub fn parse_scaled(s: &str) -> Option<UnitScale> {
        let (unit, num, den) = match s.trim().to_lowercase().as_str() {
            "kwh" | "kwh_th" | "kwh_hs" => (Self::KiloWattHour, 1, 1),
            "wh" => (Self::KiloWattHour, 1, 1_000),
            "mwh" => (Self::KiloWattHour, 1_000, 1),
            "gwh" => (Self::KiloWattHour, 1_000_000, 1),
            // 1 GJ = 1000/3.6 kWh, a repeating decimal: held as 2500/9.
            "gj" => (Self::KiloWattHour, 2_500, 9),
            "mj" => (Self::KiloWattHour, 5, 18),
            "m3" | "m³" | "cbm" => (Self::CubicMetre, 1, 1),
            "l" | "ltr" | "liter" | "litre" => (Self::CubicMetre, 1, 1_000),

            // UN/ECE Rec. 20 codes (UTILMD DE6411, EN 16931/PEPPOL).
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

impl fmt::Display for Unit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.pad(self.as_str())
    }
}

impl Unit {
    /// The codes [`Display`](fmt::Display) writes, in [`ALL`](Self::ALL) order.
    pub const CODES: &'static [&'static str] =
        &["KWH", "KVARH", "KW", "KVAR", "M3", "KWH_PER_M3", "ONE"];
}

impl FromStr for Unit {
    type Err = ParseError;

    /// Parses the codes, case-insensitively, and every symbol
    /// [`parse_scaled`](Self::parse_scaled) accepts at factor 1 (`"kWh"`,
    /// `"m³"`). A unit needing a rescale (MWh, GJ, litres) is rejected — use
    /// [`parse_scaled`](Self::parse_scaled).
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let t = s.trim();
        if let Some(u) = Self::ALL
            .into_iter()
            .find(|u| u.as_str().eq_ignore_ascii_case(t))
        {
            return Ok(u);
        }
        Self::parse_scaled(t)
            .filter(|scale| scale.is_canonical())
            .map(|scale| scale.unit)
            .ok_or_else(|| ParseError::one_of("Unit", s, Self::CODES))
    }
}

#[cfg(feature = "serde")]
impl Serialize for Unit {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

#[cfg(feature = "serde")]
impl<'de> Deserialize<'de> for Unit {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = <std::borrow::Cow<'de, str>>::deserialize(deserializer)?;
        text.parse().map_err(serde::de::Error::custom)
    }
}

/// A parsed unit with the exact rational factor into the canonical [`Unit`].
///
/// Kept as numerator/denominator because useful factors repeat (1 GJ =
/// 277.7… kWh); [`apply`](Self::apply) multiplies first and rounds once.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnitScale {
    unit: Unit,
    num: i64,
    den: i64,
}

impl UnitScale {
    /// The canonical unit the value converts into.
    #[must_use]
    pub const fn unit(self) -> Unit {
        self.unit
    }

    /// The factor as `(numerator, denominator)`.
    #[must_use]
    pub const fn factor(self) -> (i64, i64) {
        (self.num, self.den)
    }

    /// `true` when the source unit is already canonical (factor 1:1).
    #[must_use]
    pub const fn is_canonical(self) -> bool {
        self.num == self.den
    }

    /// Convert `value` into [`unit`](Self::unit). `None` on overflow.
    #[must_use]
    pub fn apply(self, value: Decimal) -> Option<Decimal> {
        if self.is_canonical() {
            return Some(value);
        }
        value
            .checked_mul(Decimal::from(self.num))?
            .checked_div(Decimal::from(self.den))
    }
}

/// BDEW / MSCONS quality flag — the MSCONS `MESSWERTSTATUS` and BO4E
/// `Messwertstatus`.
///
/// Only [`Faulty`](Self::Faulty) and [`Unknown`](Self::Unknown) block billing
/// ([`is_billable`](Self::is_billable)).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum QualityFlag {
    /// Reading as measured (Abgelesen / Messwert).
    Measured,
    /// Estimated value (Prognosewert) — the basis of an Abschlagsrechnung and
    /// of SLP profiling.
    Estimated,
    /// Substituted value (Ersatzwert) — see [`crate::vee::substitute`].
    Substituted,
    /// Calculated value (Rechenwert), derived from other readings (e.g.
    /// Residuallast = Bezug − Einspeisung).
    Calculated,
    /// Corrected value (Nachbearbeitungswert), superseding an earlier one.
    Corrected,
    /// Preliminary value (Vorläufiger Wert) — billable, but may be revised.
    Preliminary,
    /// Faulty measurement (Fehlerhaft / Unplausibel) — not billable; needs a
    /// substitute.
    Faulty,
    /// Quality not known.
    #[default]
    Unknown,
}

impl QualityFlag {
    /// `true` for every flag except `Faulty` and `Unknown`.
    ///
    /// Estimated and substituted values are billable: excluding them would bill
    /// zero for every SLP point and every measurement outage.
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

    /// `true` for `Preliminary` and `Estimated` — billable values an invoice
    /// should mark as provisional.
    #[must_use]
    pub const fn is_provisional(self) -> bool {
        matches!(self, Self::Preliminary | Self::Estimated)
    }

    /// How far this flag is from a measurement, on a **strict** total order:
    /// `Measured` (0) < `Calculated` < `Corrected` < `Substituted` <
    /// `Estimated` < `Preliminary` < `Faulty` < `Unknown` (7).
    ///
    /// A bucket takes the worst flag of its members (see
    /// [`mod@crate::series::resample`], [`crate::allocation::formula`]). The
    /// ranks are distinct, so [`worst_of`](Self::worst_of) does not depend on
    /// the order of its input.
    ///
    /// ```rust
    /// use metering::QualityFlag;
    ///
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

    /// The worse of two flags, by [`severity_rank`](Self::severity_rank);
    /// commutative and associative.
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

    /// The market's own qualifier for a value of this quality, if it has one.
    ///
    /// The `QTY` Mengen-Qualifier of EDI@Energy **MSCONS MIG 2.5**:
    ///
    /// | Flag | Code | Market term |
    /// |---|---|---|
    /// | [`Measured`](Self::Measured) | `220` | Wahrer Wert |
    /// | [`Substituted`](Self::Substituted) | `67` | Ersatzwert |
    /// | [`Estimated`](Self::Estimated) | `187` | Prognosewert |
    /// | [`Preliminary`](Self::Preliminary) | `Z18` | Vorläufiger Wert |
    /// | [`Faulty`](Self::Faulty) | `20` | Nicht verwendbarer Wert |
    ///
    /// `None` for the other three: [`Calculated`](Self::Calculated) is marked
    /// by the Plausibilisierungshinweis `STS+Z33 ZR5`, [`Corrected`](Self::Corrected)
    /// by `STS+Z34 Korrekturgrund` beside the replaced value's qualifier, and
    /// [`Unknown`](Self::Unknown) has no market code (`Z30 Fehlender Wert`
    /// means a missing value) — resolve it before writing a message.
    ///
    /// ```rust
    /// use metering::QualityFlag;
    ///
    /// assert_eq!(QualityFlag::Measured.market_code(), Some("220"));
    /// assert_eq!(QualityFlag::Substituted.market_code(), Some("67"));
    /// assert_eq!(QualityFlag::Unknown.market_code(), None);
    /// ```
    #[must_use]
    pub const fn market_code(self) -> Option<&'static str> {
        match self {
            Self::Measured => Some("220"),
            Self::Substituted => Some("67"),
            Self::Estimated => Some("187"),
            Self::Preliminary => Some("Z18"),
            Self::Faulty => Some("20"),
            Self::Calculated | Self::Corrected | Self::Unknown => None,
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

crate::ids::codes::string_codes! {
    // No aliases: an unrecognised status is a parse failure, not `UNKNOWN`.
    QualityFlag;
}

/// Which way energy crossed the measurement point.
///
/// Read off OBIS value group C ([`ObisCode::direction`],
/// [`MeterInterval::direction`]) rather than stored beside it. A negative kWh
/// means a Korrekturenergiemenge (EDI@Energy *Codeliste* v2.5c §2.1), never a
/// reversed flow. For the balance of a bidirectional point see
/// [`sum_by_direction`](crate::billing::aggregation::sum_by_direction).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
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

    /// The German market term, *Bezug* or *Einspeisung*, for display;
    /// [`as_str`](Self::as_str) is the stored code.
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

crate::ids::codes::string_codes! {
    // The market's `BEZUG` / `EINSPEISUNG` are read; `IMPORT` / `EXPORT` are
    // written.
    Direction, aliases = [("BEZUG", Self::Import), ("EINSPEISUNG", Self::Export)];
}

/// A single metered interval `[from, to)` — the energy or volume *in* a
/// period (Lastgang). Zählerstände are in [`crate::series::reading`]; an
/// ordered run is a [`Series`](crate::Series).
///
/// [`new`](Self::new) refuses an empty or reversed span, and every
/// constructor takes the [`QualityFlag`] explicitly. [`value`](Self::value) is
/// in the channel's unit ([`ObisCode::unit`], [`Sparte::billing_unit`]): kWh
/// (gas after conversion), or m³ for Wasser.
///
/// ```rust
/// use metering::{MeterInterval, QualityFlag};
/// use rust_decimal::dec;
/// use time::macros::datetime;
///
/// let iv = MeterInterval::new(
///     datetime!(2026-06-01 12:00 UTC),
///     datetime!(2026-06-01 12:15 UTC),
///     dec!(2.5),
///     QualityFlag::Measured,
/// )?;
/// assert_eq!(iv.demand_kw(), Some(dec!(10)));
///
/// // A reversed span is refused.
/// assert!(MeterInterval::new(iv.to(), iv.from(), dec!(1), QualityFlag::Measured).is_err());
/// # Ok::<(), metering::series::interval::IntervalError>(())
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(try_from = "RawInterval"))]
pub struct MeterInterval {
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::rfc3339"))]
    from: OffsetDateTime,
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::rfc3339"))]
    to: OffsetDateTime,
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::decimal"))]
    value: Decimal,
    quality: QualityFlag,
    #[cfg_attr(feature = "serde", serde(default))]
    obis: Option<ObisCode>,
}

/// The unvalidated wire shape; deserialisation goes through
/// [`MeterInterval::new`].
#[cfg(feature = "serde")]
#[derive(Deserialize)]
struct RawInterval {
    #[serde(with = "crate::wire::rfc3339")]
    from: OffsetDateTime,
    #[serde(with = "crate::wire::rfc3339")]
    to: OffsetDateTime,
    #[serde(with = "crate::wire::decimal")]
    value: Decimal,
    quality: QualityFlag,
    #[serde(default)]
    obis: Option<ObisCode>,
}

#[cfg(feature = "serde")]
impl TryFrom<RawInterval> for MeterInterval {
    type Error = IntervalError;

    fn try_from(raw: RawInterval) -> Result<Self, Self::Error> {
        let iv = Self::new(raw.from, raw.to, raw.value, raw.quality)?;
        Ok(match raw.obis {
            Some(code) => iv.with_obis(code),
            None => iv,
        })
    }
}

/// Why a [`MeterInterval`] could not be built.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum IntervalError {
    /// `from` is not before `to`.
    #[error("interval [{from}, {to}) is empty or reversed")]
    Empty {
        /// The rejected start.
        from: OffsetDateTime,
        /// The rejected end.
        to: OffsetDateTime,
    },
    /// The end of a fixed-length slot does not fit in the time range.
    #[error("interval starting {from} overflows the time range")]
    Overflow {
        /// The rejected start.
        from: OffsetDateTime,
    },
}

impl MeterInterval {
    /// An interval `[from, to)` carrying `value` of `quality`.
    ///
    /// # Errors
    ///
    /// [`IntervalError::Empty`] unless `from < to`.
    pub fn new(
        from: OffsetDateTime,
        to: OffsetDateTime,
        value: Decimal,
        quality: QualityFlag,
    ) -> Result<Self, IntervalError> {
        if from >= to {
            return Err(IntervalError::Empty { from, to });
        }
        Ok(Self {
            from,
            to,
            value,
            quality,
            obis: None,
        })
    }

    /// [`new`](Self::new) plus an optional channel, for in-crate builders.
    pub(crate) fn build(
        from: OffsetDateTime,
        to: OffsetDateTime,
        value: Decimal,
        quality: QualityFlag,
        obis: Option<ObisCode>,
    ) -> Result<Self, IntervalError> {
        let mut iv = Self::new(from, to, value, quality)?;
        iv.obis = obis;
        Ok(iv)
    }

    /// A quarter-hour starting at `from` — the German settlement slot, a
    /// fixed 15 min in UTC on every date.
    ///
    /// # Errors
    ///
    /// [`IntervalError::Overflow`] at the very end of the time range.
    pub fn quarter_hour(
        from: OffsetDateTime,
        value: Decimal,
        quality: QualityFlag,
    ) -> Result<Self, IntervalError> {
        let to = from
            .checked_add(time::Duration::minutes(15))
            .ok_or(IntervalError::Overflow { from })?;
        Self::new(from, to, value, quality)
    }

    /// An hour starting at `from` — the German gas settlement slot.
    ///
    /// # Errors
    ///
    /// [`IntervalError::Overflow`] at the very end of the time range.
    pub fn hour(
        from: OffsetDateTime,
        value: Decimal,
        quality: QualityFlag,
    ) -> Result<Self, IntervalError> {
        let to = from
            .checked_add(time::Duration::hours(1))
            .ok_or(IntervalError::Overflow { from })?;
        Self::new(from, to, value, quality)
    }

    /// The same interval on a named OBIS channel.
    #[must_use]
    pub const fn with_obis(mut self, code: ObisCode) -> Self {
        self.obis = Some(code);
        self
    }

    /// The same interval with a different quality.
    #[must_use]
    pub const fn with_quality(mut self, quality: QualityFlag) -> Self {
        self.quality = quality;
        self
    }

    /// The same span and channel with a different value and quality — the
    /// shape a substitute or correction takes.
    #[must_use]
    pub const fn with_value(mut self, value: Decimal, quality: QualityFlag) -> Self {
        self.value = value;
        self.quality = quality;
        self
    }

    /// Interval start (UTC, inclusive).
    #[must_use]
    pub const fn from(&self) -> OffsetDateTime {
        self.from
    }

    /// Interval end (UTC, exclusive).
    #[must_use]
    pub const fn to(&self) -> OffsetDateTime {
        self.to
    }

    /// The metered quantity, in the channel's unit.
    #[must_use]
    pub const fn value(&self) -> Decimal {
        self.value
    }

    /// The reading quality.
    #[must_use]
    pub const fn quality(&self) -> QualityFlag {
        self.quality
    }

    /// The OBIS channel, when the message named one.
    #[must_use]
    pub const fn obis(&self) -> Option<ObisCode> {
        self.obis
    }

    /// The span's length — always positive.
    #[must_use]
    pub fn duration(&self) -> time::Duration {
        self.to - self.from
    }

    /// Duration in whole seconds.
    #[must_use]
    pub fn duration_secs(&self) -> i64 {
        self.duration().whole_seconds()
    }

    /// Mean power in kW, `kWh × 3600 ÷ seconds`.
    ///
    /// `None` when the channel's unit is known not to be an energy, and on
    /// overflow.
    #[must_use]
    pub fn demand_kw(&self) -> Option<Decimal> {
        if let Some(unit) = self.obis.and_then(|c| c.unit())
            && !unit.is_energy()
        {
            return None;
        }
        // Multiply first: one rounding, at the end.
        self.value
            .checked_mul(Decimal::from(3600u32))?
            .checked_div(Decimal::from(self.duration_secs()))
    }

    /// Which way the energy flowed, read off the OBIS code; `None` without a
    /// code or when the code has no direction.
    #[must_use]
    pub fn direction(&self) -> Option<Direction> {
        self.obis.and_then(ObisCode::direction)
    }

    /// The tariff of the OBIS channel — see [`ObisCode::tariff`].
    #[must_use]
    pub fn tariff(&self) -> Option<u8> {
        self.obis?.tariff()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal::dec;
    use time::macros::datetime;

    #[test]
    fn construction_refuses_an_empty_or_reversed_span() {
        let t = datetime!(2026-06-01 12:00 UTC);
        assert_eq!(
            MeterInterval::new(t, t, dec!(1), QualityFlag::Measured),
            Err(IntervalError::Empty { from: t, to: t })
        );
        assert!(
            MeterInterval::new(
                t,
                t - time::Duration::minutes(15),
                dec!(1),
                QualityFlag::Measured
            )
            .is_err()
        );
        assert!(
            MeterInterval::quarter_hour(
                OffsetDateTime::new_utc(time::Date::MAX, time::Time::from_hms(23, 59, 0).unwrap()),
                dec!(1),
                QualityFlag::Measured
            )
            .is_err()
        );
    }

    #[test]
    fn a_quarter_hour_is_fifteen_minutes_even_across_a_transition() {
        let slot = MeterInterval::quarter_hour(
            datetime!(2026-10-25 0:00 UTC),
            dec!(1),
            QualityFlag::Measured,
        )
        .unwrap();
        assert_eq!(slot.duration_secs(), 900);
        assert_eq!(slot.to(), datetime!(2026-10-25 0:15 UTC));
    }

    #[test]
    fn demand_is_only_for_energy() {
        let iv = MeterInterval::quarter_hour(
            datetime!(2026-01-01 0:00 UTC),
            dec!(2.5),
            QualityFlag::Measured,
        )
        .unwrap();
        assert_eq!(iv.demand_kw(), Some(dec!(10)));
        assert_eq!(
            iv.clone()
                .with_obis(ObisCode::STROM_BEZUG_LASTGANG)
                .demand_kw(),
            Some(dec!(10))
        );
        assert_eq!(iv.with_obis(ObisCode::GAS_VOLUME_M3).demand_kw(), None);
    }

    #[test]
    fn quality_flag_billable() {
        assert!(QualityFlag::Measured.is_billable());
        assert!(QualityFlag::Substituted.is_billable());
        assert!(QualityFlag::Estimated.is_billable());
        assert!(QualityFlag::Corrected.is_billable());
        assert!(QualityFlag::Preliminary.is_billable());
        assert!(!QualityFlag::Faulty.is_billable());
        assert!(!QualityFlag::Unknown.is_billable());
    }

    #[test]
    fn severity_ranks_are_a_strict_total_order() {
        let ranks: std::collections::BTreeSet<u8> =
            QualityFlag::ALL.iter().map(|q| q.severity_rank()).collect();
        assert_eq!(ranks.len(), QualityFlag::ALL.len());
        for a in QualityFlag::ALL {
            for b in QualityFlag::ALL {
                assert_eq!(a.worse_of(b), b.worse_of(a), "{a} vs {b}");
            }
        }
        assert_eq!(QualityFlag::worst_of([]), QualityFlag::Unknown);
    }

    #[test]
    fn interval_obis_code_is_typed() {
        let iv = MeterInterval::quarter_hour(
            datetime!(2026-01-01 0:00 UTC),
            dec!(2.5),
            QualityFlag::Measured,
        )
        .unwrap()
        .with_obis(ObisCode::STROM_BEZUG_HT);
        assert_eq!(iv.tariff(), Some(1));
        assert_eq!(iv.direction(), Some(Direction::Import));
    }
}

#[cfg(test)]
mod unit_tests {
    use super::*;

    #[test]
    fn measured_unit_is_what_the_register_counts() {
        assert_eq!(Sparte::Gas.measured_unit(), Unit::CubicMetre);
        assert_eq!(Sparte::Wasser.measured_unit(), Unit::CubicMetre);
        assert_eq!(Sparte::Waerme.measured_unit(), Unit::KiloWattHour);
        assert_eq!(Sparte::Strom.measured_unit(), Unit::KiloWattHour);
        for sparte in Sparte::ALL {
            assert_eq!(
                sparte.measured_unit() != sparte.billing_unit(),
                sparte.requires_conversion()
            );
        }
    }

    #[test]
    fn unit_parse_accepts_symbols_and_codes() {
        assert_eq!("m³".parse::<Unit>(), Ok(Unit::CubicMetre));
        assert_eq!(" M3 ".parse::<Unit>(), Ok(Unit::CubicMetre));
        assert_eq!("kWh_th".parse::<Unit>(), Ok(Unit::KiloWattHour));
        assert_eq!("kvarh".parse::<Unit>(), Ok(Unit::KiloVarHour));
        assert_eq!(
            "KWH_PER_M3".parse::<Unit>(),
            Ok(Unit::KiloWattHourPerCubicMetre)
        );
        assert!("MWh".parse::<Unit>().is_err());
        assert!("furlong".parse::<Unit>().is_err());
        for (u, code) in Unit::ALL.iter().zip(Unit::CODES) {
            assert_eq!(u.as_str(), *code);
            assert_eq!(code.parse::<Unit>(), Ok(*u));
        }
    }

    #[test]
    fn gigajoule_conversion_is_exact() {
        let gj = Unit::parse_scaled("GJ").unwrap();
        assert_eq!(gj.unit(), Unit::KiloWattHour);
        assert_eq!(
            gj.apply(Decimal::from_str_exact("3.6").unwrap()),
            Some(Decimal::from(1000u32))
        );
        assert_eq!(gj.apply(Decimal::from(9u32)), Some(Decimal::from(2500u32)));
        let mj = Unit::parse_scaled("3B").unwrap();
        assert_eq!(mj.factor(), (5, 18));
        let jou = Unit::parse_scaled("JOU").unwrap();
        assert_eq!(jou.apply(Decimal::from(3_600_000u32)), Some(Decimal::ONE));
        assert_eq!(Unit::parse_scaled("MWh").unwrap().apply(Decimal::MAX), None);
    }

    #[test]
    fn codes_round_trip() {
        for (v, code) in Sparte::ALL.iter().zip(Sparte::CODES) {
            assert_eq!(&code.parse::<Sparte>().unwrap(), v);
        }
        for (v, code) in QualityFlag::ALL.iter().zip(QualityFlag::CODES) {
            assert_eq!(&code.parse::<QualityFlag>().unwrap(), v);
        }
        let err = "GARBAGE".parse::<QualityFlag>().unwrap_err();
        assert_eq!(err.expected_values(), Some(QualityFlag::CODES));
    }
}
