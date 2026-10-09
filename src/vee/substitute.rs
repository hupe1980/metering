//! Ersatzwertbildung — substitute values for missing and rejected slots.
//!
//! [`substitute`] replaces every slot an **error** finding of the [`Report`]
//! covers (gaps; values rejected as non-billable, negative or above capacity)
//! and nothing else.
//!
//! | Situation | Method | Strom | Gas |
//! |---|---|---|---|
//! | gap ≤ 2 h, validated measured values on both sides | [`Method::Interpolation`] | `Z92` | `Z92` |
//! | longer gap, or a gap at the edge of the data | [`Method::Vergleichswert`] — mean of the same slot on the previous same-day-type days | `ZJ2` | `Z95` |
//! | gas only, chosen explicitly | [`Method::Hold`] — the last measured value | — | `Z93` |
//! | a confirmed supply interruption, stated by the caller | [`Method::Zero`] | — | — |
//!
//! Basis: VDN MeteringCode 2006 A8.2.2.1 (interpolation from checked values
//! without an error status) and A8.2.2.2 (Vergleichswertverfahren); § 55
//! Abs. 2 ElWG (AT) and CPUC VEE Rev. 2.0 § 4.1 draw the same 2-hour line.
//! Zeros only for an outage that was *eindeutig festgestellt* (A7.1.2.2). The
//! codes are `STS+Z32` of the MSCONS MIG.
//!
//! References are only validated measured values — never a substitute, a
//! rejected slot or an outage — taken before the gap start. Where no method
//! applies the slot is [refused](Refusal), never invented.
//!
//! With two register readings ([`Policy::anchor`]) the substitutes between
//! them are rescaled so the series sums to the register advance exactly
//! (MeteringCode A8.2.2.2 *Skalierung*; Elexon BSCP502 § 4.2; NL Meetcode
//! § 5.4.3), rounded to [`SUBSTITUTE_DP`] with the remainder on the last one.
//!
//! ```rust
//! use metering::prelude::*;
//! use metering::time::holiday::Bundesland;
//! use metering::vee::substitute::{Method, SubstitutionReason};
//! use time::{Duration, macros::{date, datetime}};
//!
//! let day = DayBoundary::Strom.day(date!(2026 - 06 - 01)).unwrap();
//! let slots = (0..96)
//!     .filter(|i| !(40..44).contains(i)) // one hour missing
//!     .map(|i| MeterInterval::quarter_hour(day.start() + Duration::minutes(15 * i), dec!(1), QualityFlag::Measured))
//!     .collect::<Result<Vec<_>, _>>()?;
//! let series = Series::new(Resolution::QUARTER_HOUR, DayBoundary::Strom, slots)?;
//! let report = validate(&series, &Rules::strom(day, datetime!(2026-07-01 0:00 UTC), None));
//!
//! let policy = Policy::strom(Bundesland::Be, SubstitutionReason::CommunicationFailure);
//! let filled = substitute(&series, &report, &policy)?;
//! assert_eq!(filled.substitutes.len(), 4);
//! assert_eq!(filled.substitutes[0].method, Method::Interpolation);
//! assert_eq!(filled.substitutes[0].code, Some("Z92"));
//! assert_eq!(filled.series.len(), 96);
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

use std::collections::BTreeMap;

use rust_decimal::Decimal;
use time::{Duration, OffsetDateTime};

use crate::precision::{SUBSTITUTE_DP, SUBSTITUTE_STRATEGY};
use crate::series::interval::{MeterInterval, QualityFlag, Sparte};
use crate::series::reading::MeterReading;
use crate::series::{Series, SeriesError};
use crate::time::calendar::Period;
use crate::time::holiday::Bundesland;
use crate::vee::validation::{Report, Rule, Severity};
use crate::vee::{Data, is_actual, references, slots};

/// How a substitute value was produced.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Method {
    /// Linear interpolation between the measured values either side.
    Interpolation,
    /// Mean of the same slot on the previous days of the same day class.
    Vergleichswert,
    /// The last measured value, held (gas only).
    Hold,
    /// Zero — a confirmed supply interruption.
    Zero,
}

impl Method {
    /// Every method, in declaration order.
    pub const ALL: [Self; 4] = [
        Self::Interpolation,
        Self::Vergleichswert,
        Self::Hold,
        Self::Zero,
    ];

    /// Stable code; the `serde` tag and [`FromStr`](std::str::FromStr) input.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Interpolation => "INTERPOLATION",
            Self::Vergleichswert => "VERGLEICHSWERT",
            Self::Hold => "HOLD",
            Self::Zero => "ZERO",
        }
    }

    /// The `STS+Z32` code of this method in `sparte` (see the [module table](self));
    /// `None` for [`Method::Zero`] and for Wärme or Wasser, which the MIG does not code.
    #[must_use]
    pub const fn market_code(self, sparte: Sparte) -> Option<&'static str> {
        match (self, sparte) {
            (Self::Interpolation, Sparte::Strom | Sparte::Gas) => Some("Z92"),
            (Self::Vergleichswert, Sparte::Strom) => Some("ZJ2"),
            (Self::Vergleichswert, Sparte::Gas) => Some("Z95"),
            (Self::Hold, Sparte::Gas) => Some("Z93"),
            _ => None,
        }
    }
}

/// Why a substitute value was needed: the *Statusanlässe* of `STS+Z40 Grund
/// der Ersatzwertbildung`, EDI@Energy MSCONS MIG 2.5, and nothing else.
///
/// [`code`](Self::code) is the market code, [`description`](Self::description)
/// the German title, [`as_str`](Self::as_str) a readable database label. The
/// reason is a caller input; [`Method`] (*how*) is this module's output.
///
/// ```rust
/// use metering::vee::substitute::SubstitutionReason;
///
/// let reason = SubstitutionReason::CommunicationFailure;
/// assert_eq!(reason.code(), "Z75");
/// assert_eq!(reason.as_str(), "COMMUNICATION_FAILURE");
/// assert_eq!(reason.description(), "Kommunikationsstörung");
/// assert_eq!(SubstitutionReason::from_code("Z75"), Some(reason));
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum SubstitutionReason {
    /// `Z74` — the meter could not be reached for an on-site reading.
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

    /// Stable DB/wire label; the `serde` tag and [`FromStr`](std::str::FromStr) input.
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

    /// The `STS+Z40` market code; [`from_code`](Self::from_code) inverts it.
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

    /// The reason a market code names (case-insensitive, trimmed); `None` outside the list.
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

    /// Whether the MIG states this reason for `sparte`; a code it annotates
    /// for no Sparte counts as both. Advisory, not what a Netzbetreiber accepts.
    #[must_use]
    pub const fn applies_to(self, sparte: Sparte) -> bool {
        match self {
            Self::VoltageFailure | Self::Calibration | Self::TariffSwitchDeviceDefect => {
                matches!(sparte, Sparte::Strom)
            }
            Self::FaultRegisterUsed
            | Self::ConversionIncomplete
            | Self::MaintenanceCalibratedDevice
            | Self::DeviceReportsDisturbedValues
            | Self::MaintenanceConformantDevice
            | Self::ConsistencyCheckFailed => matches!(sparte, Sparte::Gas),
            _ => true,
        }
    }
}

crate::ids::codes::string_codes! {
    Method;
    SubstitutionReason;
}

/// Why a [`Policy`] could not be built.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum PolicyError {
    /// `Z93 Haltewert` is annotated Gas only in the MIG.
    #[error("a held value has no market code for {0}")]
    HoldNotCoded(Sparte),
    /// The end reading is not after the start reading.
    #[error("the anchor's end reading at {end} is not after its start at {start}")]
    AnchorReversed {
        /// The start reading's instant.
        start: OffsetDateTime,
        /// The end reading's instant.
        end: OffsetDateTime,
    },
    /// The register decreased; resolve a rollover with [`consumption_between`](crate::series::reading::consumption_between).
    #[error("the register decreased between {start} and {end}")]
    AnchorDecreasing {
        /// The start reading's instant.
        start: OffsetDateTime,
        /// The end reading's instant.
        end: OffsetDateTime,
    },
}

/// How [`substitute`] fills: by default interpolate up to 2 h, else Vergleichswert
/// over 3 comparable days. The Bundesland sets the holidays (counted as Sundays);
/// the reason (`STS+Z40`) goes on every substitute except a value rejected as
/// negative or above capacity, which carries `ZA1` *Messwert unplausibel*.
#[derive(Debug, Clone, PartialEq)]
pub struct Policy {
    sparte: Sparte,
    land: Bundesland,
    reason: SubstitutionReason,
    short: Duration,
    days: usize,
    hold: bool,
    outages: Vec<(OffsetDateTime, OffsetDateTime)>,
    anchor: Option<(MeterReading, MeterReading)>,
    history: Option<Series>,
}

impl Policy {
    /// The default for `sparte`.
    #[must_use]
    pub const fn new(sparte: Sparte, land: Bundesland, reason: SubstitutionReason) -> Self {
        Self {
            sparte,
            land,
            reason,
            short: Duration::hours(2),
            days: 3,
            hold: false,
            outages: Vec::new(),
            anchor: None,
            history: None,
        }
    }

    /// The electricity default.
    #[must_use]
    pub const fn strom(land: Bundesland, reason: SubstitutionReason) -> Self {
        Self::new(Sparte::Strom, land, reason)
    }

    /// The gas default.
    #[must_use]
    pub const fn gas(land: Bundesland, reason: SubstitutionReason) -> Self {
        Self::new(Sparte::Gas, land, reason)
    }

    /// The longest gap that is interpolated, inclusive; default 2 h (MeteringCode 2006
    /// A8.2.2.1: *≤ 2 Stunden*; § 55 Abs. 2 Z 1 ElWG: *weniger als zwei Stunden*). `Duration::ZERO` switches it off.
    #[must_use]
    pub const fn short(mut self, max_gap: Duration) -> Self {
        self.short = max_gap;
        self
    }

    /// How many comparable days the Vergleichswert averages. Default 3.
    #[must_use]
    pub const fn days(mut self, n: usize) -> Self {
        self.days = n;
        self
    }

    /// Fill long gaps with the last measured value (`Z93`), gas only.
    ///
    /// # Errors
    ///
    /// [`PolicyError::HoldNotCoded`] for any other Sparte.
    pub fn hold(mut self) -> Result<Self, PolicyError> {
        if self.sparte != Sparte::Gas {
            return Err(PolicyError::HoldNotCoded(self.sparte));
        }
        self.hold = true;
        Ok(self)
    }

    /// A confirmed supply interruption over `[from, to)`, filled with zeros (MeteringCode 2006 A7.1.2.2).
    #[must_use]
    pub fn outage(mut self, from: OffsetDateTime, to: OffsetDateTime) -> Self {
        self.outages.push((from, to));
        self
    }

    /// Prior data the Vergleichswert may draw on, besides the series itself.
    #[must_use]
    pub fn history(mut self, prior: Series) -> Self {
        self.history = Some(prior);
        self
    }

    /// Scale the substitutes in `[start.at, end.at)` so the series sums to the register advance.
    ///
    /// # Errors
    ///
    /// [`PolicyError::AnchorReversed`] or [`PolicyError::AnchorDecreasing`].
    pub fn anchor(mut self, start: MeterReading, end: MeterReading) -> Result<Self, PolicyError> {
        if end.at <= start.at {
            return Err(PolicyError::AnchorReversed {
                start: start.at,
                end: end.at,
            });
        }
        if end.value < start.value {
            return Err(PolicyError::AnchorDecreasing {
                start: start.at,
                end: end.at,
            });
        }
        self.anchor = Some((start, end));
        Ok(self)
    }

    fn in_outage(&self, slot: &Period) -> bool {
        self.outages
            .iter()
            .any(|(from, to)| *from <= slot.start() && slot.end() <= *to)
    }
}

/// One substitute value and its provenance.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Substitute {
    /// Slot start (UTC).
    pub from: OffsetDateTime,
    /// Slot end (UTC, exclusive).
    pub to: OffsetDateTime,
    /// The value written into the series.
    pub value: Decimal,
    /// How it was produced.
    pub method: Method,
    /// `STS+Z32` for the policy's Sparte — see [`Method::market_code`].
    pub code: Option<&'static str>,
    /// `STS+Z40`.
    pub reason: SubstitutionReason,
    /// Length of the gap (or rejected run) the slot belongs to.
    pub gap: Duration,
    /// Start instants of the values it was formed from.
    pub references: Vec<OffsetDateTime>,
    /// The value it replaced, for a rejected slot.
    pub original: Option<Decimal>,
    /// `true` when rescaled to a register advance.
    pub scaled: bool,
}

/// Why a span was left as it was.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum RefusalReason {
    /// No validated measured value on a comparable day before the gap.
    NoComparableHistory,
    /// No measured value before the gap to hold.
    NoHeldValue,
    /// The arithmetic left the `Decimal` range.
    Overflow,
    /// A reading instant is not a slot boundary of the series.
    AnchorOffGrid,
    /// A slot of the reading interval is still missing or rejected.
    AnchorIncomplete,
    /// The values outside the substitutes already exceed the advance.
    AnchorExceeded {
        /// Σ of the slots that are not substitutes.
        actual: Decimal,
        /// The register advance.
        advance: Decimal,
    },
}

/// A span [`substitute`] did not fill (or, for an anchor, did not scale).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct Refusal {
    /// First instant (UTC).
    pub from: OffsetDateTime,
    /// End (UTC, exclusive).
    pub to: OffsetDateTime,
    /// Why.
    pub reason: RefusalReason,
}

/// The filled series and what was done to it.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct Filled {
    /// The series with every substitute in place; a refused rejected slot stays, flagged [`QualityFlag::Faulty`].
    pub series: Series,
    /// One entry per substituted slot, ascending.
    pub substitutes: Vec<Substitute>,
    /// What was not filled, and why.
    pub refusals: Vec<Refusal>,
}

/// Why [`substitute`] could not run at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum SubstituteError {
    /// The report was not produced for this series.
    #[error("the report does not belong to this series")]
    ReportMismatch,
    /// The filled intervals did not form a series.
    #[error(transparent)]
    Series(#[from] SeriesError),
}

/// Replace every slot an error finding of `report` covers — see the [module docs](self).
///
/// # Errors
///
/// [`SubstituteError::ReportMismatch`] when `report` was produced for another series.
pub fn substitute(
    series: &Series,
    report: &Report,
    policy: &Policy,
) -> Result<Filled, SubstituteError> {
    if !report.describes(series) {
        return Err(SubstituteError::ReportMismatch);
    }
    let (boundary, resolution) = (series.boundary(), series.resolution());

    let mut todo: BTreeMap<OffsetDateTime, (Period, SubstitutionReason)> = BTreeMap::new();
    for f in report
        .findings
        .iter()
        .filter(|f| f.severity() == Severity::Error)
    {
        let reason = match f.rule {
            Rule::Negative | Rule::Capacity => SubstitutionReason::ImplausibleValue,
            _ => policy.reason,
        };
        for slot in slots(boundary, resolution, f.from, f.to).filter(|s| s.start() >= f.from) {
            todo.entry(slot.start()).or_insert((slot, reason));
        }
    }

    let mut filler = Filler {
        data: Data {
            series,
            prior: policy.history.as_ref(),
        },
        policy,
        todo: &todo,
        out: BTreeMap::new(),
        refused: Vec::new(),
    };
    for block in blocks(todo.values().map(|(slot, _)| *slot)) {
        let gap = block
            .last()
            .map_or(Duration::ZERO, |l| l.end() - block[0].start());
        let (outage, rest): (Vec<Period>, Vec<Period>) =
            block.into_iter().partition(|s| policy.in_outage(s));
        for slot in outage {
            filler.put(&slot, Decimal::ZERO, Method::Zero, gap, Vec::new());
        }
        for sub in blocks(rest) {
            filler.fill(&sub, gap);
        }
    }
    let Filler {
        mut out,
        mut refused,
        ..
    } = filler;

    if let Some((start, end)) = &policy.anchor
        && let Err(reason) = scale(series, &mut out, start, end)
    {
        refused.push((start.at, end.at, reason));
    }

    let mut intervals: Vec<MeterInterval> = series
        .iter()
        .filter(|iv| !out.contains_key(&iv.from()))
        .map(|iv| {
            if todo.contains_key(&iv.from()) {
                iv.clone().with_quality(QualityFlag::Faulty)
            } else {
                iv.clone()
            }
        })
        .collect();
    // A slot is a grid bucket, so `from < to` and the build cannot fail.
    intervals.extend(out.values().filter_map(|s| {
        MeterInterval::build(
            s.from,
            s.to,
            s.value,
            QualityFlag::Substituted,
            series.channel(),
        )
        .ok()
    }));
    Ok(Filled {
        series: Series::new(resolution, boundary, intervals)?,
        substitutes: out.into_values().collect(),
        refusals: merge(refused),
    })
}

/// Maximal runs of adjacent slots.
fn blocks(slots: impl IntoIterator<Item = Period>) -> Vec<Vec<Period>> {
    let mut out: Vec<Vec<Period>> = Vec::new();
    for slot in slots {
        match out.last_mut() {
            Some(block) if block.last().is_some_and(|l| l.end() == slot.start()) => {
                block.push(slot)
            }
            _ => out.push(vec![slot]),
        }
    }
    out
}

/// Adjacent refusals with the same reason, as one span.
fn merge(mut spans: Vec<(OffsetDateTime, OffsetDateTime, RefusalReason)>) -> Vec<Refusal> {
    spans.sort_by_key(|(from, ..)| *from);
    let mut out: Vec<Refusal> = Vec::new();
    for (from, to, reason) in spans {
        match out.last_mut() {
            Some(last) if last.to == from && last.reason == reason => last.to = to,
            _ => out.push(Refusal { from, to, reason }),
        }
    }
    out
}

struct Filler<'a> {
    data: Data<'a>,
    policy: &'a Policy,
    todo: &'a BTreeMap<OffsetDateTime, (Period, SubstitutionReason)>,
    out: BTreeMap<OffsetDateTime, Substitute>,
    refused: Vec<(OffsetDateTime, OffsetDateTime, RefusalReason)>,
}

impl Filler<'_> {
    /// A validated measured value: measured, not rejected, not in an outage.
    fn usable(&self, iv: &MeterInterval) -> bool {
        is_actual(iv.quality())
            && !self.todo.contains_key(&iv.from())
            && !self
                .policy
                .outages
                .iter()
                .any(|(f, t)| *f <= iv.from() && iv.to() <= *t)
    }

    fn put(
        &mut self,
        slot: &Period,
        value: Decimal,
        method: Method,
        gap: Duration,
        references: Vec<OffsetDateTime>,
    ) {
        let reason = self
            .todo
            .get(&slot.start())
            .map_or(self.policy.reason, |(_, r)| *r);
        self.out.insert(
            slot.start(),
            Substitute {
                from: slot.start(),
                to: slot.end(),
                value: value.round_dp_with_strategy(SUBSTITUTE_DP, SUBSTITUTE_STRATEGY),
                method,
                code: method.market_code(self.policy.sparte),
                reason,
                gap,
                references,
                original: self.data.series.get(slot.start()).map(MeterInterval::value),
                scaled: false,
            },
        );
    }

    fn refuse(&mut self, slot: &Period, reason: RefusalReason) {
        self.refused.push((slot.start(), slot.end(), reason));
    }

    /// The measured value ending at `at`, or starting at it.
    fn neighbour(&self, at: OffsetDateTime, ending: bool) -> Option<&MeterInterval> {
        let series = self.data.series;
        let iv = if ending {
            let previous = series
                .boundary()
                .bucket(at - Duration::nanoseconds(1), series.resolution())?;
            series.get(previous.start())?
        } else {
            series.get(at)?
        };
        self.usable(iv).then_some(iv)
    }

    /// One run of adjacent slots outside any outage, in a gap `gap` long.
    fn fill(&mut self, sub: &[Period], gap: Duration) {
        let (Some(first), Some(last)) = (sub.first(), sub.last()) else {
            return;
        };
        let (start, end) = (first.start(), last.end());
        let before = self.neighbour(start, true).cloned();
        let after = self.neighbour(end, false).cloned();
        if end - start <= self.policy.short
            && let (Some(a), Some(b)) = (&before, &after)
        {
            let k = Decimal::from(sub.len() + 1);
            for (i, slot) in sub.iter().enumerate() {
                // a + (b − a) × (i + 1) ÷ k: multiply before dividing.
                let value = b
                    .value()
                    .checked_sub(a.value())
                    .and_then(|d| d.checked_mul(Decimal::from(i + 1)))
                    .and_then(|d| d.checked_div(k))
                    .and_then(|d| d.checked_add(a.value()));
                match value {
                    Some(v) => self.put(
                        slot,
                        v,
                        Method::Interpolation,
                        gap,
                        vec![a.from(), b.from()],
                    ),
                    None => self.refuse(slot, RefusalReason::Overflow),
                }
            }
            return;
        }
        if self.policy.hold {
            let held = self
                .data
                .series
                .iter()
                .rev()
                .filter(|iv| iv.to() <= start)
                .find(|iv| self.usable(iv))
                .map(|iv| (iv.from(), iv.value()));
            for slot in sub {
                match held {
                    Some((at, v)) => self.put(slot, v, Method::Hold, gap, vec![at]),
                    None => self.refuse(slot, RefusalReason::NoHeldValue),
                }
            }
            return;
        }
        for slot in sub {
            let refs = references(
                &self.data,
                slot.start(),
                start,
                self.policy.land,
                self.policy.days,
                |iv| self.usable(iv),
            );
            let mean = refs
                .iter()
                .try_fold(Decimal::ZERO, |sum, (_, v)| sum.checked_add(*v))
                .and_then(|sum| sum.checked_div(Decimal::from(refs.len())));
            match mean {
                _ if refs.is_empty() => self.refuse(slot, RefusalReason::NoComparableHistory),
                Some(v) => {
                    let at = refs.iter().map(|(at, _)| *at).collect();
                    self.put(slot, v, Method::Vergleichswert, gap, at);
                }
                None => self.refuse(slot, RefusalReason::Overflow),
            }
        }
    }
}

/// Rescale the substitutes in `[start.at, end.at)` to the register advance.
fn scale(
    series: &Series,
    out: &mut BTreeMap<OffsetDateTime, Substitute>,
    start: &MeterReading,
    end: &MeterReading,
) -> Result<(), RefusalReason> {
    let (boundary, resolution) = (series.boundary(), series.resolution());
    let on_grid = |at: OffsetDateTime| {
        boundary
            .bucket(at, resolution)
            .is_some_and(|b| b.start() == at)
    };
    if !on_grid(start.at) || !on_grid(end.at) {
        return Err(RefusalReason::AnchorOffGrid);
    }
    let advance = end
        .value
        .checked_sub(start.value)
        .ok_or(RefusalReason::Overflow)?;
    let mut actual = Decimal::ZERO;
    let mut filled: Vec<OffsetDateTime> = Vec::new();
    for slot in slots(boundary, resolution, start.at, end.at) {
        if out.contains_key(&slot.start()) {
            filled.push(slot.start());
            continue;
        }
        let iv = series
            .get(slot.start())
            .filter(|iv| iv.quality().is_billable())
            .ok_or(RefusalReason::AnchorIncomplete)?;
        actual = actual
            .checked_add(iv.value())
            .ok_or(RefusalReason::Overflow)?;
    }
    let Some((&last, rest)) = filled.split_last() else {
        return Ok(());
    };
    let target = advance.checked_sub(actual).ok_or(RefusalReason::Overflow)?;
    if target < Decimal::ZERO {
        return Err(RefusalReason::AnchorExceeded { actual, advance });
    }
    let raw = filled
        .iter()
        .filter_map(|at| out.get(at))
        .try_fold(Decimal::ZERO, |sum, s| sum.checked_add(s.value))
        .ok_or(RefusalReason::Overflow)?;
    let n = Decimal::from(filled.len());
    let mut values = Vec::with_capacity(rest.len());
    let mut assigned = Decimal::ZERO;
    for at in rest {
        let s = out.get(at).ok_or(RefusalReason::AnchorIncomplete)?;
        // v × target ÷ Σ v, or an even share when every raw value is zero.
        let v = if raw.is_zero() {
            target.checked_div(n)
        } else {
            s.value.checked_mul(target).and_then(|p| p.checked_div(raw))
        }
        .ok_or(RefusalReason::Overflow)?
        .round_dp_with_strategy(SUBSTITUTE_DP, SUBSTITUTE_STRATEGY);
        assigned = assigned.checked_add(v).ok_or(RefusalReason::Overflow)?;
        values.push((*at, v));
    }
    values.push((
        last,
        target
            .checked_sub(assigned)
            .ok_or(RefusalReason::Overflow)?,
    ));
    for (at, v) in values {
        if let Some(s) = out.get_mut(&at) {
            s.value = v;
            s.scaled = true;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::time::calendar::DayBoundary;
    use crate::time::resolution::Resolution;
    use crate::vee::validation::{Grade, Rules, validate};
    use rust_decimal::dec;
    use time::Date;
    use time::macros::{date, datetime};

    const NOW: OffsetDateTime = datetime!(2027-01-01 0:00 UTC);
    const REASON: SubstitutionReason = SubstitutionReason::CommunicationFailure;

    fn day(d: Date) -> Period {
        DayBoundary::Strom.day(d).unwrap()
    }

    fn series_q(start: OffsetDateTime, values: &[Option<(Decimal, QualityFlag)>]) -> Series {
        let ivs = values
            .iter()
            .enumerate()
            .filter_map(|(i, v)| {
                let from = start + Duration::minutes(15 * i as i64);
                v.map(|(v, q)| MeterInterval::quarter_hour(from, v, q).unwrap())
            })
            .collect();
        Series::new(Resolution::QUARTER_HOUR, DayBoundary::Strom, ivs).unwrap()
    }

    fn series(start: OffsetDateTime, values: &[Option<Decimal>]) -> Series {
        let v: Vec<_> = values
            .iter()
            .map(|v| v.map(|v| (v, QualityFlag::Measured)))
            .collect();
        series_q(start, &v)
    }

    fn run(s: &Series, period: Period, policy: &Policy) -> Filled {
        let report = validate(
            s,
            &Rules::strom(period, NOW, None)
                .without_spike()
                .without_stale(),
        );
        substitute(s, &report, policy).unwrap()
    }

    fn policy() -> Policy {
        Policy::strom(Bundesland::Be, REASON)
    }

    /// The worked example of MeteringCode 2006 A8.2.2.1.
    #[test]
    fn the_meteringcode_interpolation_example() {
        let d = day(date!(2026 - 06 - 03));
        let mut v = vec![Some(dec!(4.0)); 96];
        let published = [
            Some(dec!(4.000)),
            Some(dec!(4.200)),
            Some(dec!(4.300)),
            Some(dec!(4.350)),
            Some(dec!(4.300)),
            None,
            None,
            None,
            None,
            Some(dec!(4.100)),
            Some(dec!(3.900)),
            Some(dec!(3.800)),
            Some(dec!(3.900)),
        ];
        v[..13].copy_from_slice(&published);
        let filled = run(&series(d.start(), &v), d, &policy());
        let values: Vec<Decimal> = filled.substitutes.iter().map(|s| s.value).collect();
        assert_eq!(values, [dec!(4.26), dec!(4.22), dec!(4.18), dec!(4.14)]);
        let s = &filled.substitutes[0];
        assert_eq!(
            (s.method, s.code, s.reason),
            (Method::Interpolation, Some("Z92"), REASON)
        );
        assert_eq!(s.gap, Duration::hours(1));
        assert_eq!(s.references.len(), 2);
        let rules = Rules::strom(d, NOW, None).without_spike().without_stale();
        assert_eq!(validate(&filled.series, &rules).grade(), Grade::A);
    }

    #[test]
    fn the_short_gap_line_is_two_hours() {
        let d = day(date!(2026 - 06 - 03));
        for (missing, method) in [(8, Some(Method::Interpolation)), (9, None)] {
            let mut v = vec![Some(dec!(1)); 96];
            for slot in v.iter_mut().skip(40).take(missing) {
                *slot = None;
            }
            let filled = run(&series(d.start(), &v), d, &policy());
            assert_eq!(
                filled.substitutes.first().map(|s| s.method),
                method,
                "{missing}"
            );
            if method.is_none() {
                assert_eq!(
                    filled.refusals[0].reason,
                    RefusalReason::NoComparableHistory
                );
                assert_eq!(
                    filled.series.len(),
                    96 - missing,
                    "refused slots stay missing"
                );
            }
        }
    }

    /// History from 25 May with one level per Werktag, then a 4-hour gap on `gap_day`.
    fn history_then_gap(gap_day: Date, land_policy: &Policy) -> Filled {
        let start = day(date!(2026 - 05 - 25)).start();
        let target = day(gap_day);
        let n = ((target.end() - start).whole_minutes() / 15) as usize;
        let mut v: Vec<Option<Decimal>> = (0..n)
            .map(|i| {
                let at = start + Duration::minutes(15 * i as i64);
                let date = DayBoundary::Strom.day_of(at).unwrap();
                Some(match date.weekday() {
                    time::Weekday::Sunday => dec!(1),
                    time::Weekday::Saturday => dec!(2),
                    _ => Decimal::from(date.day()),
                })
            })
            .collect();
        let first = ((target.start() - start).whole_minutes() / 15) as usize;
        for slot in v.iter_mut().skip(first + 20).take(16) {
            *slot = None;
        }
        run(&series(start, &v), target, land_policy)
    }

    #[test]
    fn a_long_gap_is_the_mean_of_comparable_days_coded_zj2() {
        // Wednesday 17 June: previous Werktage 16, 15 and 12 June.
        let filled = history_then_gap(date!(2026 - 06 - 17), &policy());
        assert_eq!(filled.substitutes.len(), 16);
        let s = &filled.substitutes[0];
        assert_eq!((s.method, s.code), (Method::Vergleichswert, Some("ZJ2")));
        assert_eq!(s.value, dec!(14.333333));
        assert_eq!(s.references.len(), 3);
        assert_eq!(s.gap, Duration::hours(4));
        let gas = Policy::gas(Bundesland::Be, REASON);
        assert_eq!(Method::Vergleichswert.market_code(Sparte::Gas), Some("Z95"));
        assert_eq!(
            history_then_gap(date!(2026 - 06 - 17), &gas).substitutes[0].code,
            Some("Z95")
        );
    }

    #[test]
    fn a_holiday_is_compared_with_sundays() {
        let by = Policy::strom(Bundesland::By, REASON);
        let be = policy();
        let on_holiday = date!(2026 - 06 - 04);
        let days = |p: &Policy| -> Vec<Date> {
            history_then_gap(on_holiday, p).substitutes[0]
                .references
                .iter()
                .filter_map(|at| DayBoundary::Strom.day_of(*at))
                .collect()
        };
        // Sunday 31 May and Pfingstmontag 25 May — the only Sonn-/Feiertage in the data.
        assert_eq!(days(&by), [date!(2026 - 05 - 31), date!(2026 - 05 - 25)]);
        assert_eq!(
            days(&be),
            [
                date!(2026 - 06 - 03),
                date!(2026 - 06 - 02),
                date!(2026 - 06 - 01)
            ]
        );
    }

    #[test]
    fn an_edge_gap_is_not_interpolated() {
        let d = day(date!(2026 - 06 - 03));
        let mut v = vec![Some(dec!(1)); 96];
        v[95] = None;
        let filled = run(&series(d.start(), &v), d, &policy());
        assert!(filled.substitutes.is_empty());
        assert_eq!(filled.refusals.len(), 1);
    }

    #[test]
    fn no_value_is_interpolated_from_a_substitute() {
        let d = day(date!(2026 - 06 - 03));
        let m = Some((dec!(1), QualityFlag::Measured));
        let mut v = vec![m; 96];
        v[40] = Some((dec!(1), QualityFlag::Substituted));
        v[41] = None;
        let filled = run(&series_q(d.start(), &v), d, &policy());
        assert!(filled.substitutes.is_empty());
        assert_eq!(
            filled.refusals[0].reason,
            RefusalReason::NoComparableHistory
        );
    }

    #[test]
    fn a_rejected_value_is_substituted_with_its_original_kept() {
        let d = day(date!(2026 - 06 - 03));
        let mut v = vec![Some(dec!(1)); 96];
        v[40] = Some(dec!(-3));
        let filled = run(&series(d.start(), &v), d, &policy());
        let s = &filled.substitutes[0];
        assert_eq!((s.value, s.original), (dec!(1), Some(dec!(-3))));
        assert_eq!(s.reason, SubstitutionReason::ImplausibleValue);
        assert_eq!(
            filled.series.get(s.from).map(MeterInterval::quality),
            Some(QualityFlag::Substituted)
        );
    }

    #[test]
    fn a_refused_rejected_value_is_flagged_faulty() {
        let d = day(date!(2026 - 06 - 03));
        let mut v = vec![Some(dec!(1)); 96];
        v[95] = Some(dec!(-3));
        let filled = run(&series(d.start(), &v), d, &policy());
        let iv = filled.series.get(d.end() - Duration::minutes(15)).unwrap();
        assert_eq!((iv.value(), iv.quality()), (dec!(-3), QualityFlag::Faulty));
    }

    #[test]
    fn hold_is_gas_only_and_zero_needs_a_stated_outage() {
        assert_eq!(
            policy().hold(),
            Err(PolicyError::HoldNotCoded(Sparte::Strom))
        );
        let d = day(date!(2026 - 06 - 03));
        let mut v = vec![Some(dec!(1)); 96];
        v[40] = Some(dec!(7));
        for slot in v.iter_mut().skip(41).take(12) {
            *slot = None;
        }
        let s = series(d.start(), &v);
        let gas = Policy::gas(Bundesland::Be, REASON).hold().unwrap();
        let held = run(&s, d, &gas);
        assert!(
            held.substitutes
                .iter()
                .all(|s| s.value == dec!(7) && s.code == Some("Z93"))
        );
        assert!(
            run(&s, d, &policy())
                .substitutes
                .iter()
                .all(|s| s.method != Method::Zero)
        );
        let gap = (
            d.start() + Duration::minutes(15 * 41),
            d.start() + Duration::minutes(15 * 53),
        );
        let zero = run(&s, d, &policy().outage(gap.0, gap.1));
        assert!(
            zero.substitutes
                .iter()
                .all(|s| s.method == Method::Zero && s.value.is_zero())
        );
        assert_eq!(zero.substitutes.len(), 12);
        assert_eq!(zero.substitutes[0].code, None);
    }

    #[test]
    fn the_register_anchor_conserves_energy_exactly() {
        let d = day(date!(2026 - 06 - 03));
        let mut v = vec![Some(dec!(0.25)); 96];
        for slot in v.iter_mut().skip(10).take(3) {
            *slot = None;
        }
        let s = series(d.start(), &v);
        let a = MeterReading::measured(d.start(), dec!(100));
        let b = MeterReading::measured(d.end(), dec!(124.1));
        let filled = run(&s, d, &policy().anchor(a, b).unwrap());
        assert!(filled.refusals.is_empty(), "{:?}", filled.refusals);
        assert_eq!(filled.series.billable_total(), Some(dec!(24.1)));
        assert!(filled.substitutes.iter().all(|s| s.scaled));
        // The actual values alone already exceed the advance: refused, unscaled.
        let low = MeterReading::measured(d.end(), dec!(110));
        let a = MeterReading::measured(d.start(), dec!(100));
        let filled = run(&s, d, &policy().anchor(a, low).unwrap());
        assert!(matches!(
            filled.refusals[0].reason,
            RefusalReason::AnchorExceeded { .. }
        ));
        assert!(filled.substitutes.iter().all(|s| !s.scaled));
    }

    #[test]
    fn a_report_for_another_series_is_refused() {
        let d = day(date!(2026 - 06 - 03));
        let a = series(d.start(), &vec![Some(dec!(1)); 96]);
        let b = series(d.start(), &vec![Some(dec!(1)); 95]);
        let report = validate(&a, &Rules::strom(d, NOW, None));
        assert_eq!(
            substitute(&b, &report, &policy()),
            Err(SubstituteError::ReportMismatch)
        );
        let reversed = policy().anchor(
            MeterReading::measured(d.end(), dec!(1)),
            MeterReading::measured(d.start(), dec!(2)),
        );
        assert!(matches!(reversed, Err(PolicyError::AnchorReversed { .. })));
    }
}
