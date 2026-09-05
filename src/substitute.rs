//! Ersatzwertbildung — substitute values for missing or rejected readings.
//!
//! ## Legal basis
//!
//! - **§ 60 Abs. 1 MsbG** places the duty on the Messstellenbetreiber: the data
//!   collected under §§ 55–59 must be *aufbereitet* and transmitted to the
//!   berechtigte Stellen.
//! - **§ 60 Abs. 2 MsbG** names what that preparation includes: *"Bei
//!   Messstellen mit intelligenten Messsystemen sollen die Aufbereitung der
//!   Messwerte, insbesondere die Plausibilisierung und die Ersatzwertbildung im
//!   Smart-Meter-Gateway, und die Datenübermittlung über das Smart-Meter-Gateway
//!   direkt an die berechtigten Stellen erfolgen, soweit das Bundesamt für
//!   Sicherheit in der Informationstechnik dies als technisch möglich bewertet
//!   und die Bundesnetzagentur auf Basis dieser Bewertung eine Festlegung nach
//!   § 75 Satz 1 Nummer 4 trifft."*
//!
//!   Two things that sentence does **not** say. It prescribes no *procedure* —
//!   no method, no reference period, no ranking. And its *placement* in the
//!   Smart-Meter-Gateway is conditional on a BSI assessment and a BNetzA
//!   Festlegung: until one is made, Satz 2 permits the preparation to happen
//!   *"außerhalb des Smart-Meter-Gateways"*, which is the case this crate is
//!   written for.
//! - **BNetzA Festlegungen** — the current consolidated MaKo Lesefassungen are
//!   **BK6-24-174** (GPKE / WiM / MaBiS, in force 6 June 2025) — carry the
//!   process rules, and **VDE-AR-N 4400 (Metering Code)** the technical ones.
//!
//! ## Why the methods are configuration, not constants
//!
//! VDE-AR-N 4400 is a paywalled Anwendungsregel whose text cannot be reproduced
//! or verified here, so every threshold is a parameter with a documented default
//! and the operator's own metering-code settings win. What this module
//! guarantees is the arithmetic and the audit trail, not conformance to a
//! document neither the author nor the reader can cite.
//!
//! | This crate | Corresponds to | Configurable |
//! |---|---|---|
//! | [`SubstituteMethod::LinearInterpolation`] | interpolation across a short gap | [`FillGapsConfig::short_gap_threshold`] |
//! | [`SubstituteMethod::PriorPeriodAverage`] | Vergleichstag: the same slot on comparable days of the preceding week | [`REFERENCE_PERIOD_DAYS`], [`ReferenceDayMatch`] |
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
//! personenbezogene Messwerte must be erased or anonymised *"unter Beachtung
//! mess- und eichrechtlicher Vorgaben"* as soon as they are no longer needed,
//! *"spätestens jedoch nach drei Jahren ab dem Schluss des Kalenderjahres, in
//! dem der jeweilige Messwert erhoben wurde"*. Substitute values are Messwerte
//! for this purpose. Keeping them for three years *because the law says so*
//! reads the provision backwards; deleting on the anniversary regardless reads
//! past its opening clause.

use std::collections::BTreeMap;

use rust_decimal::Decimal;
use time::{Duration, OffsetDateTime};
use time_tz::{OffsetDateTimeExt as _, timezones};

use crate::calendar::DayBoundary;
use crate::interval::{MeterInterval, QualityFlag, Sparte};
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

/// Decimal places a **synthesised** value is cut to: **6**, a millionth of a
/// kWh.
///
/// Two of the four methods divide — an interpolation by the distance between
/// its anchors, a prior-period average by its sample count — and a `Decimal`
/// quotient carries up to 28 significant digits. What comes out is not an
/// intermediate: it is written into the returned series, stored, and settled
/// on, so it has to be a number someone can write down. The cut is the same
/// width and the same reason as
/// [`ALLOCATION_DP`](crate::ALLOCATION_DP), four orders of magnitude finer
/// than anything the market settles.
///
/// Rounding is half-away-from-zero rather than truncation: no conservation
/// identity runs through a substitute value — a filled series is complete, not
/// balanced — so the nearest representable value is the honest one, and
/// truncating would bias a long outage downwards.
pub const SUBSTITUTE_DP: u32 = 6;

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

    /// Stable DB/wire label. Matches the `serde` tag and
    /// [`FromStr`](std::str::FromStr) input.
    ///
    /// [`description`](Self::description) is the German prose for a human;
    /// this is the code for a column.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::LinearInterpolation => "LINEAR_INTERPOLATION",
            Self::PriorPeriodAverage => "PRIOR_PERIOD_AVERAGE",
            Self::ZeroFill => "ZERO_FILL",
            Self::LastValueCarryForward => "LAST_VALUE_CARRY_FORWARD",
        }
    }

    /// The market's own code for this Ersatzwertbildungsverfahren, if it has
    /// one for `sparte`.
    ///
    /// `STS+Z32` Statusanlass, EDI@Energy **MSCONS MIG 2.4c** (binding since
    /// 03.04.2024; MIG 2.5, binding 01.10.2026, carries the same 13 codes
    /// unchanged). The list is annotated per commodity and the annotations do
    /// not agree, which is why this takes a [`Sparte`]:
    ///
    /// | Method | Strom | Gas |
    /// |---|---|---|
    /// | [`LinearInterpolation`](Self::LinearInterpolation) | `Z92` | `Z92` |
    /// | [`PriorPeriodAverage`](Self::PriorPeriodAverage) | `ZJ2` | `Z95` |
    /// | [`LastValueCarryForward`](Self::LastValueCarryForward) | — | `Z93` |
    /// | [`ZeroFill`](Self::ZeroFill) | — | — |
    ///
    /// `None` is a real answer twice over. **A held value has no Strom code**:
    /// `Z93 Haltewert` is annotated *Gas* and the Strom list offers only
    /// `ZJ2 Statistische Methode`, which is the Vergleichswertverfahren and
    /// not a carry-forward. And **a zero fill is not an Ersatzwertbildung at
    /// all**: it asserts that nothing was delivered, which is a statement about
    /// the world rather than a method of reconstructing one, so no code
    /// describes it.
    ///
    /// A caller that must state a code where this returns `None` has a process
    /// question, not a formatting one — the honest options are a different
    /// method or a manual Klärfall.
    ///
    /// ```rust
    /// use metering::{Sparte, SubstituteMethod};
    ///
    /// assert_eq!(SubstituteMethod::LinearInterpolation.market_code(Sparte::Strom), Some("Z92"));
    /// assert_eq!(SubstituteMethod::PriorPeriodAverage.market_code(Sparte::Gas), Some("Z95"));
    /// assert_eq!(SubstituteMethod::LastValueCarryForward.market_code(Sparte::Strom), None);
    /// ```
    #[must_use]
    pub const fn market_code(self, sparte: Sparte) -> Option<&'static str> {
        match (self, sparte) {
            (Self::LinearInterpolation, _) => Some("Z92"),
            (Self::PriorPeriodAverage, Sparte::Strom) => Some("ZJ2"),
            (Self::PriorPeriodAverage, _) => Some("Z95"),
            (Self::LastValueCarryForward, Sparte::Strom) => None,
            (Self::LastValueCarryForward, _) => Some("Z93"),
            (Self::ZeroFill, _) => None,
        }
    }

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

/// Why a substitute value was needed — the market's own list.
///
/// These are the *Statusanlässe* of `STS+Z40 Grund der Ersatzwertbildung`,
/// EDI@Energy **MSCONS MIG 2.4c** (binding since 03.04.2024; MIG 2.5, binding
/// 01.10.2026, carries the same 28 codes unchanged). Every value a
/// Messstellenbetreiber may state for an Ersatzwert is here, and nothing else
/// is: an invented vocabulary would have to be mapped onto this one at the
/// market boundary, and a mapping that is not one-to-one is a place where the
/// reason changes meaning on the way out.
///
/// [`code`](Self::code) is the market code, [`description`](Self::description)
/// the published German title, and [`as_str`](Self::as_str) this crate's own
/// stable label for a database column — a code beginning with `Z` sorts and
/// reads badly in one, and `Z81` says nothing to a reader.
///
/// Distinct from [`SubstituteMethod`], which says *how* a value was produced.
/// The reason is an input the caller knows; the method is an output this module
/// determines.
///
/// ```rust
/// use metering::SubstitutionReason;
///
/// let reason = SubstitutionReason::CommunicationFailure;
/// assert_eq!(reason.code(), "Z75");
/// assert_eq!(reason.as_str(), "COMMUNICATION_FAILURE");
/// assert_eq!(reason.description(), "Kommunikationsstörung");
/// assert_eq!(SubstitutionReason::from_code("Z75"), Some(reason));
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "SCREAMING_SNAKE_CASE"))]
pub enum SubstitutionReason {
    /// `Z74` — the meter could not be reached for an on-site reading.
    #[default]
    NoAccess,
    /// `Z75` — remote read-out did not complete in time.
    CommunicationFailure,
    /// `Z76` — loss of a whole network area / missing primary voltage.
    GridOutage,
    /// `Z77` — loss of the measuring or auxiliary voltage (Strom).
    VoltageFailure,
    /// `Z78` — values incomplete because the device was exchanged.
    DeviceExchange,
    /// `Z79` — maintenance or repair on a calibrated device (Strom).
    Calibration,
    /// `Z80` — the device is running outside its permitted operating conditions.
    OutsideOperatingConditions,
    /// `Z81` — a defect was established at the metering equipment.
    MeteringEquipmentFault,
    /// `Z82` — possible defect; the equipment is under examination.
    MeasurementUncertain,
    /// `Z98` — Normvolumen taken from the Störmengenzählwerk (Gas).
    FaultRegisterUsed,
    /// `Z99` — factors needed for the Mengenumwertung are unavailable (Gas).
    ConversionIncomplete,
    /// `ZA0` — the device clock was outside its permitted bounds and was set.
    ClockAdjusted,
    /// `ZA1` — the delivered value is implausible.
    ImplausibleValue,
    /// `ZA3` — wrong transformer ratio.
    WrongTransformerRatio,
    /// `ZA4` — misread, transposed digits, wrong metering point.
    FaultyReading,
    /// `ZA5` — the calculation rule changed, or a sub-meter was taken into account.
    CalculationChanged,
    /// `ZA6` — the Messlokation was rebuilt.
    MeteringPointRebuilt,
    /// `ZA7` — an error in data processing.
    DataProcessingError,
    /// `ZB0` — a technical fault in the metering equipment.
    MeteringEquipmentDefect,
    /// `ZB9` — the tariff switching times changed.
    TariffTimesChanged,
    /// `ZC2` — the Tarifschaltgerät is defective (Strom).
    TariffSwitchDeviceDefect,
    /// `ZC4` — too few pulses under the Eichordnung to carry a value.
    InsufficientPulseWeight,
    /// `ZR1` — maintenance or repair on a calibrated device (Gas).
    MaintenanceCalibratedDevice,
    /// `ZR2` — the device marks its own results as disturbed (Gas).
    DeviceReportsDisturbedValues,
    /// `ZR3` — maintenance on eichrechtskonforme devices (Gas).
    MaintenanceConformantDevice,
    /// `ZR4` — G 685 Kap. 2.4/2.5 consistency and synchronicity check failed (Gas).
    ConsistencyCheckFailed,
    /// `ZS9` — the reasons are stated per Messlokation, for a 1:N relationship.
    StatedPerMeteringPoint,
    /// `ZT8` — a value was requested for a past instant the MSB holds none for.
    RetrospectiveRequest,
}

impl SubstitutionReason {
    /// Every reason, in the order the MIG lists them.
    pub const ALL: [Self; 28] = [
        Self::NoAccess,
        Self::CommunicationFailure,
        Self::GridOutage,
        Self::VoltageFailure,
        Self::DeviceExchange,
        Self::Calibration,
        Self::OutsideOperatingConditions,
        Self::MeteringEquipmentFault,
        Self::MeasurementUncertain,
        Self::FaultRegisterUsed,
        Self::ConversionIncomplete,
        Self::ClockAdjusted,
        Self::ImplausibleValue,
        Self::WrongTransformerRatio,
        Self::FaultyReading,
        Self::CalculationChanged,
        Self::MeteringPointRebuilt,
        Self::DataProcessingError,
        Self::MeteringEquipmentDefect,
        Self::TariffTimesChanged,
        Self::TariffSwitchDeviceDefect,
        Self::InsufficientPulseWeight,
        Self::MaintenanceCalibratedDevice,
        Self::DeviceReportsDisturbedValues,
        Self::MaintenanceConformantDevice,
        Self::ConsistencyCheckFailed,
        Self::StatedPerMeteringPoint,
        Self::RetrospectiveRequest,
    ];

    /// Stable DB/wire label. Matches the `serde` tag and
    /// [`FromStr`](std::str::FromStr) input.
    ///
    /// [`code`](Self::code) is the market's own three-character code and
    /// [`description`](Self::description) the German title.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NoAccess => "NO_ACCESS",
            Self::CommunicationFailure => "COMMUNICATION_FAILURE",
            Self::GridOutage => "GRID_OUTAGE",
            Self::VoltageFailure => "VOLTAGE_FAILURE",
            Self::DeviceExchange => "DEVICE_EXCHANGE",
            Self::Calibration => "CALIBRATION",
            Self::OutsideOperatingConditions => "OUTSIDE_OPERATING_CONDITIONS",
            Self::MeteringEquipmentFault => "METERING_EQUIPMENT_FAULT",
            Self::MeasurementUncertain => "MEASUREMENT_UNCERTAIN",
            Self::FaultRegisterUsed => "FAULT_REGISTER_USED",
            Self::ConversionIncomplete => "CONVERSION_INCOMPLETE",
            Self::ClockAdjusted => "CLOCK_ADJUSTED",
            Self::ImplausibleValue => "IMPLAUSIBLE_VALUE",
            Self::WrongTransformerRatio => "WRONG_TRANSFORMER_RATIO",
            Self::FaultyReading => "FAULTY_READING",
            Self::CalculationChanged => "CALCULATION_CHANGED",
            Self::MeteringPointRebuilt => "METERING_POINT_REBUILT",
            Self::DataProcessingError => "DATA_PROCESSING_ERROR",
            Self::MeteringEquipmentDefect => "METERING_EQUIPMENT_DEFECT",
            Self::TariffTimesChanged => "TARIFF_TIMES_CHANGED",
            Self::TariffSwitchDeviceDefect => "TARIFF_SWITCH_DEVICE_DEFECT",
            Self::InsufficientPulseWeight => "INSUFFICIENT_PULSE_WEIGHT",
            Self::MaintenanceCalibratedDevice => "MAINTENANCE_CALIBRATED_DEVICE",
            Self::DeviceReportsDisturbedValues => "DEVICE_REPORTS_DISTURBED_VALUES",
            Self::MaintenanceConformantDevice => "MAINTENANCE_CONFORMANT_DEVICE",
            Self::ConsistencyCheckFailed => "CONSISTENCY_CHECK_FAILED",
            Self::StatedPerMeteringPoint => "STATED_PER_METERING_POINT",
            Self::RetrospectiveRequest => "RETROSPECTIVE_REQUEST",
        }
    }

    /// The market code — `STS+Z40` Statusanlass, MSCONS MIG 2.4c.
    ///
    /// What a MSCONS writer puts on the wire. The mapping is one-to-one in
    /// both directions; [`from_code`](Self::from_code) inverts it.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::NoAccess => "Z74",
            Self::CommunicationFailure => "Z75",
            Self::GridOutage => "Z76",
            Self::VoltageFailure => "Z77",
            Self::DeviceExchange => "Z78",
            Self::Calibration => "Z79",
            Self::OutsideOperatingConditions => "Z80",
            Self::MeteringEquipmentFault => "Z81",
            Self::MeasurementUncertain => "Z82",
            Self::FaultRegisterUsed => "Z98",
            Self::ConversionIncomplete => "Z99",
            Self::ClockAdjusted => "ZA0",
            Self::ImplausibleValue => "ZA1",
            Self::WrongTransformerRatio => "ZA3",
            Self::FaultyReading => "ZA4",
            Self::CalculationChanged => "ZA5",
            Self::MeteringPointRebuilt => "ZA6",
            Self::DataProcessingError => "ZA7",
            Self::MeteringEquipmentDefect => "ZB0",
            Self::TariffTimesChanged => "ZB9",
            Self::TariffSwitchDeviceDefect => "ZC2",
            Self::InsufficientPulseWeight => "ZC4",
            Self::MaintenanceCalibratedDevice => "ZR1",
            Self::DeviceReportsDisturbedValues => "ZR2",
            Self::MaintenanceConformantDevice => "ZR3",
            Self::ConsistencyCheckFailed => "ZR4",
            Self::StatedPerMeteringPoint => "ZS9",
            Self::RetrospectiveRequest => "ZT8",
        }
    }

    /// The reason a market code names, or `None` for a code outside the list.
    ///
    /// Case-insensitive, whitespace-tolerant. `None` rather than a default:
    /// a code this crate does not know is a statement about the message, not
    /// about the value it describes.
    #[must_use]
    pub fn from_code(code: &str) -> Option<Self> {
        let upper = code.trim().to_uppercase();
        Self::ALL.into_iter().find(|r| r.code() == upper)
    }

    /// The published German title, verbatim from the MIG.
    #[must_use]
    pub const fn description(self) -> &'static str {
        match self {
            Self::NoAccess => "kein Zugang",
            Self::CommunicationFailure => "Kommunikationsstörung",
            Self::GridOutage => "Netzausfall",
            Self::VoltageFailure => "Spannungsausfall",
            Self::DeviceExchange => "Gerätewechsel",
            Self::Calibration => "Kalibrierung",
            Self::OutsideOperatingConditions => "Gerät arbeitet außerhalb der Betriebsbedingungen",
            Self::MeteringEquipmentFault => "Messeinrichtung gestört/defekt",
            Self::MeasurementUncertain => "Unsicherheit Messung",
            Self::FaultRegisterUsed => "Berücksichtigung Störmengenzählwerk",
            Self::ConversionIncomplete => "Mengenumwertung unvollständig",
            Self::ClockAdjusted => "Uhrzeit gestellt /Synchronisation",
            Self::ImplausibleValue => "Messwert unplausibel",
            Self::WrongTransformerRatio => "Falscher Wandlerfaktor",
            Self::FaultyReading => "Fehlerhafte Ablesung",
            Self::CalculationChanged => "Änderung der Berechnung",
            Self::MeteringPointRebuilt => "Umbau der Messlokation",
            Self::DataProcessingError => "Datenbearbeitungsfehler",
            Self::MeteringEquipmentDefect => "Störung / Defekt Messeinrichtung",
            Self::TariffTimesChanged => "Änderung Tarifschaltzeiten",
            Self::TariffSwitchDeviceDefect => "Tarifschaltgerät defekt",
            Self::InsufficientPulseWeight => "Impulswertigkeit nicht ausreichend",
            Self::MaintenanceCalibratedDevice => "Wartungsarbeiten an geeichtem Messgerät",
            Self::DeviceReportsDisturbedValues => "gestörte Werte",
            Self::MaintenanceConformantDevice => {
                "Wartungsarbeiten an eichrechtskonformen Messgeräten"
            }
            Self::ConsistencyCheckFailed => "Konsistenz- und Synchronprüfung",
            Self::StatedPerMeteringPoint => {
                "Grund der Ersatzwertbildung gemäß Angaben auf Ebene der Messlokation"
            }
            Self::RetrospectiveRequest => {
                "Anforderung in die Vergangenheit, zum angeforderten Zeitpunkt liegt kein Wert vor."
            }
        }
    }

    /// Whether the MIG states this reason for a commodity.
    ///
    /// The MIG annotates most codes *Strom*, *Gas* or *Strom / Gas*; a few
    /// carry no annotation at all, and those are reported as applying to both
    /// rather than to neither. Advisory: it says what the code list documents,
    /// not what a Netzbetreiber will accept.
    #[must_use]
    pub const fn applies_to(self, sparte: Sparte) -> bool {
        match self {
            // Strom only.
            Self::VoltageFailure | Self::Calibration | Self::TariffSwitchDeviceDefect => {
                matches!(sparte, Sparte::Strom)
            }
            // Gas only.
            Self::FaultRegisterUsed
            | Self::ConversionIncomplete
            | Self::MaintenanceCalibratedDevice
            | Self::DeviceReportsDisturbedValues
            | Self::MaintenanceConformantDevice
            | Self::ConsistencyCheckFailed => matches!(sparte, Sparte::Gas),
            // Stated for both, or stated for neither.
            _ => true,
        }
    }
}

/// Which prior-period days count as comparable — the Vergleichstag rule.
///
/// [`PriorPeriodAverage`](SubstituteMethod::PriorPeriodAverage) averages the
/// *same slot* over the preceding [`REFERENCE_PERIOD_DAYS`]. Which days that
/// leaves is a convention, and the two the German market uses differ on
/// exactly the days that matter most:
///
/// | | Werktag gap | Gap on 3 October, a Friday |
/// |---|---|---|
/// | [`Weekday`](Self::Weekday) | the previous Friday | the previous Friday — **a working day** |
/// | [`DayType`](Self::DayType) | the previous Werktage | the previous Sonn- und Feiertage |
///
/// A public holiday is a Sunday in load terms, so averaging it over working
/// days overstates it — and averaging a working day over a week containing a
/// holiday understates it. [`DayType`](Self::DayType) needs a
/// [`Bundesland`](crate::Bundesland), because the German holiday calendar is a
/// state one; [`Weekday`](Self::Weekday) needs no calendar at all and is the
/// default for that reason, not because it is the better rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "SCREAMING_SNAKE_CASE"))]
pub enum ReferenceDayMatch {
    /// Same weekday, same time of day. No holiday calendar consulted.
    #[default]
    Weekday,
    /// Same [`SlpDayType`](crate::SlpDayType) in this Bundesland, same time of
    /// day — the Vergleichstag as the SLP procedures define it.
    ///
    /// Widens the pool as well as correcting it: a Wednesday gap draws on every
    /// Werktag of the reference week rather than on the one previous Wednesday,
    /// so a single missing reference no longer drops the method to a
    /// carry-forward.
    DayType(crate::holiday::Bundesland),
}

crate::codes::string_codes! {
    SubstituteMethod;
    SubstitutionReason;
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
/// There is no `Default`: the grid resolution and the period are the two things
/// a gap fill cannot proceed without, and the two most easily got wrong, so they
/// are constructor arguments.
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

    /// Where a daily, monthly or yearly grid slot is cut.
    ///
    /// [`DayBoundary::Midnight`] by default. [`DayBoundary::Gastag`] walks
    /// 06:00-to-06:00 gas days instead — the grid a gas SLP allocation is
    /// filled against. Sub-daily resolutions are unaffected.
    pub day_boundary: DayBoundary,

    /// Recorded on every generated entry.
    pub reason: SubstitutionReason,

    /// Which prior-period days count as comparable.
    ///
    /// [`ReferenceDayMatch::Weekday`] by default — the rule that needs no
    /// holiday calendar. See [`ReferenceDayMatch`] for what that costs on a
    /// public holiday.
    pub reference_days: ReferenceDayMatch,
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
            day_boundary: DayBoundary::Midnight,
            reason: SubstitutionReason::NoAccess,
            reference_days: ReferenceDayMatch::Weekday,
        }
    }

    /// Cut daily, monthly and yearly slots on `boundary` (builder style).
    ///
    /// ```rust
    /// use metering::{FillGapsConfig, IntervalResolution, calendar::DayBoundary};
    /// use time::macros::datetime;
    ///
    /// let cfg = FillGapsConfig::new(
    ///     IntervalResolution::Day,
    ///     datetime!(2026-01-01 5:00 UTC),
    ///     datetime!(2026-01-08 5:00 UTC),
    /// )
    /// .on(DayBoundary::Gastag);
    /// assert_eq!(cfg.day_boundary, DayBoundary::Gastag);
    /// ```
    #[must_use]
    pub const fn on(mut self, boundary: DayBoundary) -> Self {
        self.day_boundary = boundary;
        self
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

    /// Match reference days on the SLP day type in `land` rather than on the
    /// weekday — so a public holiday averages over Sundays and holidays.
    ///
    /// ```rust
    /// use metering::{Bundesland, FillGapsConfig, IntervalResolution};
    /// use metering::substitute::ReferenceDayMatch;
    /// use time::macros::datetime;
    ///
    /// let cfg = FillGapsConfig::new(
    ///     IntervalResolution::QuarterHour,
    ///     datetime!(2026-10-03 0:00 UTC),
    ///     datetime!(2026-10-04 0:00 UTC),
    /// )
    /// .matching_day_types(Bundesland::By);
    /// assert_eq!(cfg.reference_days, ReferenceDayMatch::DayType(Bundesland::By));
    /// ```
    #[must_use]
    pub const fn matching_day_types(mut self, land: crate::holiday::Bundesland) -> Self {
        self.reference_days = ReferenceDayMatch::DayType(land);
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
    /// Input intervals that landed on no grid slot, ascending.
    ///
    /// An interval is placed by its `from` timestamp falling exactly on a slot
    /// start inside the period. Anything else — a series that sits off the
    /// grid, one that starts on a different boundary, an interval outside the
    /// requested period — is **not** in
    /// [`intervals`](Self::intervals), and its slot was filled with an invented
    /// value instead. That is a real answer to a real question ("complete this
    /// grid"), but a silent one, and silently replacing a measured value with a
    /// substitute is the worst outcome this module can produce. They are
    /// reported here so a caller can refuse, requantise, or widen the period.
    pub unplaced: Vec<MeterInterval>,
}

impl FilledSeries {
    /// Number of values that had to be invented.
    #[must_use]
    pub fn substituted_count(&self) -> usize {
        self.substitutions.len()
    }

    /// `true` when every supplied interval landed on a grid slot.
    ///
    /// A `false` here means part of the input was replaced by a substitute —
    /// see [`unplaced`](Self::unplaced).
    #[must_use]
    pub fn placed_everything(&self) -> bool {
        self.unplaced.is_empty()
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
/// length is measured once, at the gap's first missing slot, so a long gap
/// keeps its method to the last interval.
///
/// A **present but non-billable** slot (`Faulty`, `Unknown`) is never
/// overwritten, and never anchors an interpolation either: the line runs
/// between the billable values either side, each at the grid slot it actually
/// occupies, so every missing slot around a faulty reading lands on one line.
///
/// Leading and trailing gaps are filled too, and are the likeliest to have no
/// bracketing value: the entries record the fallback that ran.
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

    // Every `IntervalResolution` describes a grid — a fixed length or a calendar
    // period — so there is no "no grid" case left to guard against: a
    // `CustomSeconds` cannot be zero.
    //
    // An empty or inverted range has no slots at all, so there is nothing to
    // return — not even the input, which lies outside the requested range.
    if to <= from {
        return FilledSeries {
            intervals: Vec::new(),
            substitutions: Vec::new(),
            unplaced: intervals.to_vec(),
        };
    }

    // Grid slot → measured interval. A BTreeMap rather than a HashMap because
    // the gap walk below needs ordered lookahead for the closing value.
    let measured: BTreeMap<i64, &MeterInterval> = intervals
        .iter()
        .map(|iv| (iv.from.unix_timestamp(), iv))
        .collect();
    // Which of them a slot actually claimed. Anything left over was replaced
    // by an invented value, which the caller has to be told about.
    let mut placed: std::collections::BTreeSet<i64> = std::collections::BTreeSet::new();

    let reference = PriorPeriodIndex::build(&config.prior_period_intervals, config.reference_days);

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

    // The channel the substitutes are labelled with is the earliest interval's,
    // not `intervals[0]`'s: the input need not be sorted, and a label that
    // depends on the caller's ordering is not a label.
    let obis = intervals
        .iter()
        .min_by_key(|iv| iv.from)
        .and_then(|iv| iv.obis_code);
    let mut cursor = from;
    let mut idx = 0usize;
    while cursor < to {
        let Some(next) = advance(cursor, resolution, config.day_boundary) else {
            break;
        };
        let ts = cursor.unix_timestamp();

        if let Some(&iv) = measured.get(&ts) {
            placed.insert(ts);
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
                let g = Gap::measure(
                    &measured,
                    cursor,
                    idx,
                    resolution,
                    config.day_boundary,
                    to,
                    anchor,
                );
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
        // Cut once, where the value is formed. Both dividing methods can leave
        // a 28-digit quotient, and this one is written into the series.
        let value = value.round_dp(SUBSTITUTE_DP);

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

    let mut unplaced: Vec<MeterInterval> = intervals
        .iter()
        .filter(|iv| !placed.contains(&iv.from.unix_timestamp()))
        .cloned()
        .collect();
    unplaced.sort_by_key(|iv| (iv.from, iv.to));

    FilledSeries {
        intervals: out,
        substitutions,
        unplaced,
    }
}

/// The end of the grid slot starting at `cursor`.
///
/// Calendar resolutions resolve through [`crate::calendar`], so a `Day` step is
/// 23, 24 or 25 hours depending on the date, and `boundary` decides whether
/// that day starts at 00:00 or at the 06:00 Gastag. `None` when the arithmetic
/// leaves the representable range.
fn advance(
    cursor: OffsetDateTime,
    resolution: IntervalResolution,
    boundary: DayBoundary,
) -> Option<OffsetDateTime> {
    let next = match resolution {
        IntervalResolution::Day => boundary.day_end_utc(boundary.local_day(cursor)),
        IntervalResolution::Month => boundary.month_end_utc(boundary.local_day(cursor)),
        IntervalResolution::Year => boundary.year_end_utc(boundary.local_year(cursor)),
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
    #[allow(clippy::too_many_arguments)]
    fn measure(
        measured: &BTreeMap<i64, &MeterInterval>,
        start: OffsetDateTime,
        start_idx: usize,
        resolution: IntervalResolution,
        boundary: DayBoundary,
        end: OffsetDateTime,
        preceding: Option<(usize, Decimal)>,
    ) -> Self {
        // The contiguous missing run, bounded by the fill period.
        let mut run_len = 0usize;
        let mut cursor = start;
        while cursor < end && !measured.contains_key(&cursor.unix_timestamp()) {
            run_len += 1;
            let Some(next) = advance(cursor, resolution, boundary) else {
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
                walk = advance(walk, resolution, boundary)?;
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
                // Offsets into the whole series, not into the run: every
                // missing slot between the same two anchors then lands on the
                // same straight line, however the runs between them are
                // partitioned by a bordering faulty slot.
                // `u64`, not `u32` — a `usize` narrowed to `u32` truncates
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

/// Reference values indexed by comparable day and time of day, in German local
/// time.
struct PriorPeriodIndex {
    slots: BTreeMap<(u8, u8, u8), Vec<(OffsetDateTime, Decimal)>>,
    matching: ReferenceDayMatch,
}

impl PriorPeriodIndex {
    fn build(intervals: &[MeterInterval], matching: ReferenceDayMatch) -> Self {
        let mut slots: BTreeMap<(u8, u8, u8), Vec<(OffsetDateTime, Decimal)>> = BTreeMap::new();
        for iv in intervals.iter().filter(|iv| iv.quality.is_billable()) {
            slots
                .entry(slot_key(iv.from, matching))
                .or_default()
                .push((iv.from, iv.value));
        }
        Self { slots, matching }
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
        let samples = self.slots.get(&slot_key(target, self.matching))?;
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

/// (comparable day, hour, minute) in Europe/Berlin.
///
/// The first component is the weekday, or the SLP day type where the caller
/// supplied a Bundesland. Day types are offset past the seven weekdays so the
/// two rules can never collide in one index — they do not share a map, but a
/// key space that overlaps invites the bug where they do.
fn slot_key(ts: OffsetDateTime, matching: ReferenceDayMatch) -> (u8, u8, u8) {
    let local = ts.to_timezone(timezones::db::europe::BERLIN);
    let day = match matching {
        ReferenceDayMatch::Weekday => local.weekday().number_days_from_monday(),
        ReferenceDayMatch::DayType(land) => {
            7 + match crate::holiday::slp_day_type(local.date(), land) {
                crate::load_profile::SlpDayType::Werktag => 0,
                crate::load_profile::SlpDayType::Samstag => 1,
                crate::load_profile::SlpDayType::SonnFeiertag => 2,
            }
        }
    };
    (day, local.hour(), local.minute())
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

    /// A long gap keeps its configured method to the last interval: the length
    /// is measured once, at the first missing slot, not from a moving cursor
    /// that would shrink it as the gap fills.
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

    /// Only the preceding week counts, and the window is applied here rather
    /// than trusted from the caller.
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
            &cfg(0, 60).because(SubstitutionReason::CommunicationFailure),
        );
        assert_eq!(filled.substituted_count(), 2);
        for entry in &filled.substitutions {
            assert_eq!(entry.reason, SubstitutionReason::CommunicationFailure);
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
            &FillGapsConfig::new(
                IntervalResolution::from_seconds(86_400).unwrap(),
                period.0,
                period.1,
            ),
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

    /// A long gap must not be truncated: a scan capped at a fixed length would
    /// silently switch method past that point.
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
        assert_eq!(
            SubstitutionReason::ALL.len(),
            28,
            "STS+Z40 lists 28 Statusanlässe in MSCONS MIG 2.4c and 2.5 alike"
        );
    }

    /// The market codes are what a MSCONS writer puts on the wire, so the
    /// mapping has to be a bijection: two reasons sharing a code would make
    /// the round trip lossy, and a code of the wrong shape would be rejected
    /// by the counterparty rather than by us.
    #[test]
    fn every_reason_carries_a_distinct_well_formed_market_code() {
        let mut seen: Vec<&str> = Vec::new();
        for reason in SubstitutionReason::ALL {
            let code = reason.code();
            assert_eq!(code.len(), 3, "{reason:?} → {code}");
            assert!(
                code.starts_with('Z')
                    && code
                        .chars()
                        .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit()),
                "{reason:?} → {code}"
            );
            assert!(!seen.contains(&code), "{code} used twice");
            seen.push(code);
            assert_eq!(SubstitutionReason::from_code(code), Some(reason));
            assert_eq!(
                SubstitutionReason::from_code(&code.to_lowercase()),
                Some(reason)
            );
        }
        assert_eq!(SubstitutionReason::from_code("ZZZ"), None);
        assert_eq!(SubstitutionReason::from_code(""), None);
    }

    /// The Ersatzwertbildungsverfahren list is annotated per commodity, and the
    /// two annotations differ — a held value has no Strom code at all, and a
    /// zero fill has none anywhere.
    #[test]
    fn method_market_codes_follow_the_commodity() {
        use SubstituteMethod as M;
        assert_eq!(
            M::LinearInterpolation.market_code(Sparte::Strom),
            Some("Z92")
        );
        assert_eq!(M::LinearInterpolation.market_code(Sparte::Gas), Some("Z92"));
        assert_eq!(
            M::PriorPeriodAverage.market_code(Sparte::Strom),
            Some("ZJ2")
        );
        assert_eq!(M::PriorPeriodAverage.market_code(Sparte::Gas), Some("Z95"));
        assert_eq!(
            M::LastValueCarryForward.market_code(Sparte::Gas),
            Some("Z93")
        );
        assert_eq!(M::LastValueCarryForward.market_code(Sparte::Strom), None);
        for sparte in Sparte::ALL {
            assert_eq!(M::ZeroFill.market_code(sparte), None);
        }
    }

    /// A commodity annotation the MIG actually states, in both directions.
    #[test]
    fn reason_commodity_applicability_matches_the_mig() {
        use SubstitutionReason as R;
        assert!(R::VoltageFailure.applies_to(Sparte::Strom));
        assert!(!R::VoltageFailure.applies_to(Sparte::Gas));
        assert!(R::ConversionIncomplete.applies_to(Sparte::Gas));
        assert!(!R::ConversionIncomplete.applies_to(Sparte::Strom));
        // Stated "Strom / Gas", so both.
        assert!(R::NoAccess.applies_to(Sparte::Strom));
        assert!(R::NoAccess.applies_to(Sparte::Gas));
    }

    /// A prior-period average over a sample count that does not divide the sum
    /// is exactly where an uncut quotient would reach the returned series.
    #[test]
    fn a_synthesised_value_is_cut_to_a_representable_width() {
        let prior: Vec<MeterInterval> = [7, 14, 21]
            .into_iter()
            .map(|days_back| MeterInterval {
                from: BASE - Duration::days(days_back) + Duration::minutes(15),
                to: BASE - Duration::days(days_back) + Duration::minutes(30),
                value: dec!(1) / Decimal::from(3u32) * Decimal::from(days_back),
                quality: QualityFlag::Measured,
                obis_code: None,
            })
            .collect();

        let filled = fill_gaps(
            &[iv(0, dec!(2.0)), iv(30, dec!(2.0))],
            &cfg(0, 45).prior_period(prior).short_gap_threshold(0),
        );
        let synthesised = filled.substitutions[0].interval.value;
        assert!(
            synthesised.scale() <= SUBSTITUTE_DP,
            "{synthesised} carries {} places",
            synthesised.scale()
        );
    }

    /// A public holiday is a Sunday in load terms. Matching on the weekday
    /// averages the previous working Fridays into a 3 October gap and
    /// overstates it; matching on the day type draws on Sundays and holidays.
    #[test]
    fn a_holiday_gap_can_be_averaged_over_comparable_days() {
        use crate::holiday::Bundesland;
        use time::macros::date;

        // 3 October 2026 is a Saturday, so take 2027: it falls on a Sunday.
        // Use 1 May 2026 instead — Tag der Arbeit, a Friday.
        let gap_day = date!(2026 - 05 - 01);
        let slot = crate::calendar::day_start_utc(gap_day) + Duration::hours(12);

        // A fortnight of references: working days draw 10, Sundays and the
        // Ascension holiday (14 May) draw 2. Only the days *before* the gap
        // count, so build the two weeks before 1 May.
        let mut prior = Vec::new();
        for back in 1..=7i64 {
            let day = gap_day - Duration::days(back);
            let at = crate::calendar::day_start_utc(day) + Duration::hours(12);
            let quiet = Bundesland::By.is_holiday(day) || day.weekday() == time::Weekday::Sunday;
            prior.push(MeterInterval::quarter_hour(
                at,
                if quiet { dec!(2) } else { dec!(10) },
            ));
        }

        let cfg = |c: FillGapsConfig| {
            fill_gaps(&[], &c.prior_period(prior.clone()).short_gap_threshold(0))
        };
        let period = (slot, slot + Duration::minutes(15));

        let by_weekday = cfg(FillGapsConfig::new(
            IntervalResolution::QuarterHour,
            period.0,
            period.1,
        ));
        let by_day_type =
            cfg(
                FillGapsConfig::new(IntervalResolution::QuarterHour, period.0, period.1)
                    .matching_day_types(Bundesland::By),
            );

        // The previous Friday (24 April) is an ordinary working day.
        assert_eq!(by_weekday.intervals[0].value, dec!(10));
        // Sunday 26 April is the only comparable day in the week before.
        assert_eq!(by_day_type.intervals[0].value, dec!(2));
        assert_eq!(
            by_day_type.substitutions[0].method,
            SubstituteMethod::PriorPeriodAverage
        );
    }

    /// An interval that sits off the grid is not silently swapped for an
    /// invented one: it is reported, so the caller can refuse the fill.
    #[test]
    fn an_off_grid_interval_is_reported_rather_than_dropped() {
        let off_grid = MeterInterval {
            from: BASE + Duration::minutes(7),
            to: BASE + Duration::minutes(22),
            value: dec!(9.0),
            quality: QualityFlag::Measured,
            obis_code: None,
        };
        let filled = fill_gaps(&[iv(0, dec!(2.0)), off_grid.clone()], &cfg(0, 45));

        assert!(!filled.placed_everything());
        assert_eq!(filled.unplaced, vec![off_grid]);
        // ...and the slot it should have occupied was substituted instead.
        assert_eq!(filled.substituted_count(), 2);
    }
}

#[cfg(test)]
mod gas_day_grid_tests {
    use super::*;
    use crate::calendar;
    use rust_decimal::dec;
    use time::macros::date;

    /// A daily gas fill walks Gastage, so its slots start at 06:00 local and
    /// stretch to 25 hours across the autumn transition — the same DST
    /// correctness the calendar grid has, on the boundary gas balances on.
    #[test]
    fn a_daily_gas_fill_walks_gas_days() {
        let from = calendar::gas_day_start_utc(date!(2026 - 10 - 23));
        let to = calendar::gas_day_start_utc(date!(2026 - 10 - 27));

        let measured = vec![MeterInterval {
            from,
            to: calendar::gas_day_end_utc(date!(2026 - 10 - 23)),
            value: dec!(500),
            quality: QualityFlag::Measured,
            obis_code: None,
        }];

        let filled = fill_gaps(
            &measured,
            &FillGapsConfig::new(IntervalResolution::Day, from, to)
                .on(DayBoundary::Gastag)
                .with_method(SubstituteMethod::LastValueCarryForward)
                .short_gap_threshold(0),
        );

        assert_eq!(filled.intervals.len(), 4, "four Gastage");
        for (i, day) in [
            date!(2026 - 10 - 23),
            date!(2026 - 10 - 24),
            date!(2026 - 10 - 25),
            date!(2026 - 10 - 26),
        ]
        .into_iter()
        .enumerate()
        {
            assert_eq!(
                filled.intervals[i].from,
                calendar::gas_day_start_utc(day),
                "{day}"
            );
            assert_eq!(
                filled.intervals[i].to,
                calendar::gas_day_end_utc(day),
                "{day}"
            );
        }

        // Saturday's Gastag carries the repeated hour, so it is 25 hours long.
        let saturday = &filled.intervals[1];
        assert_eq!((saturday.to - saturday.from).whole_hours(), 25);
        assert_eq!(filled.substituted_count(), 3);
    }

    /// The default boundary is unchanged, so an electricity fill still walks
    /// calendar days.
    #[test]
    fn the_default_boundary_is_still_midnight() {
        let cfg = FillGapsConfig::new(
            IntervalResolution::Day,
            calendar::day_start_utc(date!(2026 - 06 - 01)),
            calendar::day_start_utc(date!(2026 - 06 - 03)),
        );
        assert_eq!(cfg.day_boundary, DayBoundary::Midnight);
        let filled = fill_gaps(&[], &cfg);
        assert_eq!(filled.intervals.len(), 2);
        assert_eq!(
            filled.intervals[0].from,
            calendar::day_start_utc(date!(2026 - 06 - 01))
        );
    }
}
