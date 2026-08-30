//! §42c EnWG Energy-Sharing metering eligibility — pure decision logic.
//!
//! § 42c EnWG itself entered into force on **22 December 2025** (BGBl. 2025 I
//! Nr. 347). What starts later is the **Netzbetreiber's duty** to make sharing
//! possible: Abs. 4 obliges every Verteilernetzbetreiber to ensure it from
//! **1 June 2026** *"innerhalb des Bilanzierungsgebietes eines
//! Elektrizitätsverteilernetzbetreibers"* and, from 1 June 2028, also into the
//! Bilanzierungsgebiet of a directly adjacent operator in the same Regelzone.
//!
//! The distinction matters for a readiness report: the eligibility conditions
//! of Abs. 1 have been law since December 2025 and can be assessed against a
//! portfolio today, while a point that is [`NotCapable`] before June 2026 is
//! not yet in breach of anything.
//!
//! [`NotCapable`]: SharingReadiness::NotCapable
//!
//! Its binding practical constraint is not the allocation engine — it is which
//! delivery points can produce quarter-hour values at all. §42c Abs. 1 admits a
//! point only when both consumption *and* generation are measured by:
//!
//! > „Zählerstandsgangmessung nach § 2 Satz 1 Nummer 27 des
//! > Messstellenbetriebsgesetzes **oder** durch eine viertelstündliche
//! > registrierende Leistungsmessung"
//!
//! The **`oder` is load-bearing**. Zählerstandsgangmessung and viertelstündliche
//! RLM are two independent qualifying bases, so a conventional RLM meter
//! installed before the iMSys rollout qualifies on its own. Treating
//! "Zählerstandsgangmessung" as a synonym for "iMSys" both over-restricts (it
//! excludes conforming RLM) and over-permits (an iMSys that is not configured for
//! Zählerstandsgangmessung produces no quarter-hour series).
//!
//! # The two dimensions
//!
//! Eligibility is not one question but two, answered from different stores:
//!
//! | Dimension | Question | Source |
//! |---|---|---|
//! | **Capability** | Can this point produce quarter-hour values? | Device master data |
//! | **Delivery** | Is it actually producing them? | Observed intervals |
//!
//! Keeping them apart is the point. A point with an iMSys installed but no
//! Zählerstandsgangmessung configured is *capable but not delivering* — it needs
//! a configuration order, not a meter rollout. Collapsing the two into a single
//! boolean hides exactly the distinction an operator has to act on.
//!
//! # Definitions
//!
//! - **§2 Satz 1 Nr. 27 MsbG — Zählerstandsgangmessung**: „die Messung einer
//!   Reihe viertelstündig ermittelter Zählerstände von elektrischer Arbeit und
//!   stündlich ermittelter Zählerstände von Gasmengen". Electricity is
//!   quarter-hourly; §42c is Strom-only, so the gas branch never applies here.
//! - **§2 Satz 1 Nr. 7 MsbG — intelligentes Messsystem (iMSys)**: a moderne
//!   Messeinrichtung *or* a Messeinrichtung zur registrierenden Leistungsmessung
//!   bound into a communication network via a Smart-Meter-Gateway.
//! - **§2 Satz 1 Nr. 15 MsbG — moderne Messeinrichtung (mME)**: reflects
//!   consumption and usage time. No gateway, no interval series — **not**
//!   sufficient for §42c on its own.

use time::OffsetDateTime;

use crate::classification::Messtyp;
use crate::resolution::IntervalResolution;

// ── Qualifying basis ──────────────────────────────────────────────────────────

/// The statutory basis on which a delivery point qualifies under §42c Abs. 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "SCREAMING_SNAKE_CASE"))]
pub enum EligibilityBasis {
    /// Zählerstandsgangmessung per §2 Satz 1 Nr. 27 MsbG — the iMSys route.
    Zaehlerstandsgangmessung,
    /// Viertelstündliche registrierende Leistungsmessung — the RLM route.
    ///
    /// Independently sufficient; does not require a Smart-Meter-Gateway.
    RegistrierendeLeistungsmessung,
}

impl EligibilityBasis {
    /// Every basis, in declaration order.
    pub const ALL: [Self; 2] = [
        Self::Zaehlerstandsgangmessung,
        Self::RegistrierendeLeistungsmessung,
    ];

    /// Stable DB/wire label. Matches the `serde` tag and
    /// [`FromStr`](std::str::FromStr) input.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Zaehlerstandsgangmessung => "ZAEHLERSTANDSGANGMESSUNG",
            Self::RegistrierendeLeistungsmessung => "REGISTRIERENDE_LEISTUNGSMESSUNG",
        }
    }

    /// The statutory citation this basis rests on.
    #[must_use]
    pub const fn legal_basis(self) -> &'static str {
        match self {
            Self::Zaehlerstandsgangmessung => "§2 Satz 1 Nr. 27 MsbG",
            Self::RegistrierendeLeistungsmessung => "§42c Abs. 1 EnWG",
        }
    }
}

// ── Findings ──────────────────────────────────────────────────────────────────

/// Why an assessment reached the verdict it did.
///
/// A closed vocabulary rather than the `Vec<String>` of German prose this used
/// to return. A domain library that emits display text has decided the
/// caller's language, their wording and their formatting, and left them
/// nothing to match on: routing a readiness report by *reason* meant
/// `contains("fernauslesbar")`. Render these where the language is known.
///
/// Exhaustive, like every other domain enum here — see the crate-level
/// **Enum exhaustiveness** section. `#[non_exhaustive]` is reserved for
/// *errors*; a finding is routed, stored and displayed, so a new one should
/// break a consumer's `match` and make a human decide what it means.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "SCREAMING_SNAKE_CASE"))]
pub enum Finding {
    /// The meter is flagged as not remotely readable, so no series of
    /// viertelstündig ermittelte Zählerstände can be transmitted.
    NotRemotelyReadable,
    /// A moderne Messeinrichtung has no gateway and no interval series
    /// (§ 2 Satz 1 Nr. 15 MsbG).
    ModerneMesseinrichtungWithoutGateway,
    /// The Bilanzierungsmethode yields no quarter-hour values.
    BalancingMethodHasNoQuarterHourValues,
    /// The meter type is neither an iMSys nor covered by RLM.
    MeterTypeQualifiesForNeitherLimb,
    /// No Zählertyp in the master data.
    ZaehlertypMissing,
    /// No Bilanzierungsmethode on the Marktlokation.
    BilanzierungsmethodeMissing,
    /// No readings at all in the observation window.
    NoReadings,
    /// Readings arrived, but not at quarter-hour resolution.
    NotQuarterHourResolution,
    /// The interval length could not be determined from the series.
    ResolutionUndeterminable,
    /// Coverage is below the configured threshold.
    CoverageBelowThreshold,
}

impl Finding {
    /// Stable DB/wire label. Matches the `serde` tag and
    /// [`FromStr`](std::str::FromStr) input.
    ///
    /// [`description_de`](Self::description_de) is the German prose; this is
    /// the code a readiness report is routed and stored on.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NotRemotelyReadable => "NOT_REMOTELY_READABLE",
            Self::ModerneMesseinrichtungWithoutGateway => "MODERNE_MESSEINRICHTUNG_WITHOUT_GATEWAY",
            Self::BalancingMethodHasNoQuarterHourValues => {
                "BALANCING_METHOD_HAS_NO_QUARTER_HOUR_VALUES"
            }
            Self::MeterTypeQualifiesForNeitherLimb => "METER_TYPE_QUALIFIES_FOR_NEITHER_LIMB",
            Self::ZaehlertypMissing => "ZAEHLERTYP_MISSING",
            Self::BilanzierungsmethodeMissing => "BILANZIERUNGSMETHODE_MISSING",
            Self::NoReadings => "NO_READINGS",
            Self::NotQuarterHourResolution => "NOT_QUARTER_HOUR_RESOLUTION",
            Self::ResolutionUndeterminable => "RESOLUTION_UNDETERMINABLE",
            Self::CoverageBelowThreshold => "COVERAGE_BELOW_THRESHOLD",
        }
    }

    /// Every finding, in declaration order.
    pub const ALL: [Self; 10] = [
        Self::NotRemotelyReadable,
        Self::ModerneMesseinrichtungWithoutGateway,
        Self::BalancingMethodHasNoQuarterHourValues,
        Self::MeterTypeQualifiesForNeitherLimb,
        Self::ZaehlertypMissing,
        Self::BilanzierungsmethodeMissing,
        Self::NoReadings,
        Self::NotQuarterHourResolution,
        Self::ResolutionUndeterminable,
        Self::CoverageBelowThreshold,
    ];

    /// A German rendering, for an operator-facing report.
    ///
    /// Provided as a convenience, not as the interface: match on the variant.
    #[must_use]
    pub const fn description_de(self) -> &'static str {
        match self {
            Self::NotRemotelyReadable => {
                "Zähler ist als nicht fernauslesbar gekennzeichnet — keine \
                 Zählerstandsgangmessung möglich"
            }
            Self::ModerneMesseinrichtungWithoutGateway => {
                "moderne Messeinrichtung ohne Smart-Meter-Gateway \
                 (§ 2 Satz 1 Nr. 15 MsbG) — iMSys-Rollout oder RLM erforderlich"
            }
            Self::BalancingMethodHasNoQuarterHourValues => {
                "Bilanzierungsmethode liefert keine Viertelstundenwerte"
            }
            Self::MeterTypeQualifiesForNeitherLimb => "Zählertyp ist weder iMSys noch RLM",
            Self::ZaehlertypMissing => "kein Zählertyp im Stammdatensatz hinterlegt",
            Self::BilanzierungsmethodeMissing => "keine Bilanzierungsmethode an der Marktlokation",
            Self::NoReadings => "keine Messwerte im Betrachtungszeitraum",
            Self::NotQuarterHourResolution => "Messwerte liegen nicht viertelstündlich vor",
            Self::ResolutionUndeterminable => "Intervalllänge konnte nicht bestimmt werden",
            Self::CoverageBelowThreshold => "Abdeckung unter der Schwelle",
        }
    }
}

// ── Capability (master data) ──────────────────────────────────────────────────

/// The metering equipment installed at a delivery point.
///
/// Modelled on the BO4E `Zaehlertyp` value set, narrowed to the distinctions
/// § 42c turns on. Anything that is a meter but not one of the first two is
/// [`Conventional`](Self::Conventional) — a Drehstromzähler, a Wechselstrom-
/// zähler and a Ferraris meter differ in ways § 42c does not care about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "SCREAMING_SNAKE_CASE"))]
pub enum Zaehlertyp {
    /// Intelligentes Messsystem — a modern meter behind a Smart-Meter-Gateway
    /// (§ 2 Satz 1 Nr. 7 MsbG).
    IntelligentesMesssystem,
    /// Moderne Messeinrichtung — reflects consumption and usage time, but has
    /// no gateway and produces no interval series (§ 2 Satz 1 Nr. 15 MsbG).
    /// **Not** sufficient for § 42c on its own.
    ModerneMesseinrichtung,
    /// Any conventional meter: Drehstrom-, Wechselstrom-, Ferrariszähler.
    Conventional,
}

impl Zaehlertyp {
    /// Every variant, in declaration order.
    pub const ALL: [Self; 3] = [
        Self::IntelligentesMesssystem,
        Self::ModerneMesseinrichtung,
        Self::Conventional,
    ];

    /// Stable DB/wire label. Matches the `serde` tag and
    /// [`FromStr`](std::str::FromStr) input.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::IntelligentesMesssystem => "INTELLIGENTES_MESSSYSTEM",
            Self::ModerneMesseinrichtung => "MODERNE_MESSEINRICHTUNG",
            Self::Conventional => "CONVENTIONAL",
        }
    }
}

/// How the Marktlokation is balanced (`Marktlokation.bilanzierungsmethode`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "SCREAMING_SNAKE_CASE"))]
pub enum Bilanzierungsmethode {
    /// Registrierende Leistungsmessung — the second § 42c limb, sufficient on
    /// its own and needing no gateway.
    Rlm,
    /// Standardlastprofil — no quarter-hour values.
    Slp,
    /// Zählerstandsgangmessung at an intelligentes Messsystem.
    Ims,
    /// Temperaturabhängiges Lastprofil (Gas). § 42c is Strom-only, so this can
    /// never qualify.
    Tlp,
    /// Pauschale Abrechnung — no measurement at all.
    Pauschal,
}

impl Bilanzierungsmethode {
    /// Every variant, in declaration order.
    pub const ALL: [Self; 5] = [Self::Rlm, Self::Slp, Self::Ims, Self::Tlp, Self::Pauschal];

    /// Stable DB/wire label. Matches the `serde` tag and
    /// [`FromStr`](std::str::FromStr) input.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Rlm => "RLM",
            Self::Slp => "SLP",
            Self::Ims => "IMS",
            Self::Tlp => "TLP",
            Self::Pauschal => "PAUSCHAL",
        }
    }

    /// `true` when the method produces no quarter-hour series and so rules the
    /// point out on its own.
    #[must_use]
    pub const fn precludes_quarter_hour_values(self) -> bool {
        matches!(self, Self::Slp | Self::Tlp | Self::Pauschal)
    }
}

/// Device master data for one delivery point.
///
/// Every field is `Option` because master data is routinely incomplete, and the
/// assessment reports *why* it could not decide rather than guessing.
///
/// The two enum fields were free-text `Option<String>` compared with `==`
/// against literals such as `"INTELLIGENTES_MESSSYSTEM"`. That silently
/// disqualified every record whose source spelled the value differently —
/// lowercase, hyphenated, or simply another vocabulary — and the resulting
/// verdict was `Disqualified`, not `Unknown`, so nothing indicated a mapping
/// bug. Parsing into these enums at the boundary makes an unmappable value an
/// error where the raw string is still available to report.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MeteringCapabilityInput {
    /// The installed metering equipment.
    pub zaehlertyp: Option<Zaehlertyp>,
    /// BO4E `Zaehler.istFernauslesbar`. A meter that cannot be read remotely
    /// cannot supply a quarter-hour series regardless of its type.
    pub ist_fernauslesbar: Option<bool>,
    /// How the Marktlokation is balanced.
    pub bilanzierungsmethode: Option<Bilanzierungsmethode>,
    /// Whether an operational Smart-Meter-Gateway session exists for this point.
    pub smgw_operational: Option<bool>,
}

/// Outcome of the master-data capability assessment.
///
/// `Copy`, like every other verdict in this module: a discriminant plus an
/// [`EligibilityBasis`], which is itself `Copy`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "SCREAMING_SNAKE_CASE"))]
pub enum Capability {
    /// Master data supports a §42c-qualifying measurement.
    Qualified(EligibilityBasis),
    /// Master data positively rules the point out.
    Disqualified,
    /// Master data is insufficient to decide.
    Unknown,
}

impl Capability {
    /// The qualifying basis, when one was established.
    #[must_use]
    pub const fn basis(self) -> Option<EligibilityBasis> {
        match self {
            Self::Qualified(b) => Some(b),
            _ => None,
        }
    }
}

/// Assess §42c capability from device master data.
///
/// Order matters. `bilanzierungsmethode = RLM` is checked before the meter type
/// because RLM qualifies on its own statutory limb and does not require a
/// gateway. `IMS` and an `INTELLIGENTES_MESSSYSTEM` meter qualify on the
/// Zählerstandsgangmessung limb, but only when the point is remotely readable —
/// an iMSys with no working gateway produces nothing.
///
/// A `MODERNE_MESSEINRICHTUNG` is explicitly disqualified: per §2 Satz 1 Nr. 15
/// MsbG an mME records consumption and usage time but has no gateway and no
/// interval series.
#[must_use]
pub fn assess_capability(input: &MeteringCapabilityInput) -> (Capability, Vec<Finding>) {
    let mut findings = Vec::new();
    let methode = input.bilanzierungsmethode;
    let typ = input.zaehlertyp;

    // Limb 2 — viertelstündliche registrierende Leistungsmessung.
    if methode == Some(Bilanzierungsmethode::Rlm) {
        return (
            Capability::Qualified(EligibilityBasis::RegistrierendeLeistungsmessung),
            findings,
        );
    }

    // Limb 1 — Zählerstandsgangmessung via iMSys.
    let looks_imsys = methode == Some(Bilanzierungsmethode::Ims)
        || typ == Some(Zaehlertyp::IntelligentesMesssystem)
        || input.smgw_operational == Some(true);

    if looks_imsys {
        // Remote readability is a precondition, not a nicety: without it there
        // is no series of viertelstündig ermittelte Zählerstände to transmit.
        if input.ist_fernauslesbar == Some(false) {
            findings.push(Finding::NotRemotelyReadable);
            return (Capability::Disqualified, findings);
        }
        return (
            Capability::Qualified(EligibilityBasis::Zaehlerstandsgangmessung),
            findings,
        );
    }

    if typ == Some(Zaehlertyp::ModerneMesseinrichtung) {
        findings.push(Finding::ModerneMesseinrichtungWithoutGateway);
        return (Capability::Disqualified, findings);
    }

    if methode.is_some_and(Bilanzierungsmethode::precludes_quarter_hour_values) {
        findings.push(Finding::BalancingMethodHasNoQuarterHourValues);
        return (Capability::Disqualified, findings);
    }

    // Nothing positive and nothing disqualifying: say which field is missing
    // rather than guessing from the other.
    if typ.is_none() {
        findings.push(Finding::ZaehlertypMissing);
    }
    if methode.is_none() {
        findings.push(Finding::BilanzierungsmethodeMissing);
    }
    if findings.is_empty() {
        findings.push(Finding::MeterTypeQualifiesForNeitherLimb);
        return (Capability::Disqualified, findings);
    }
    (Capability::Unknown, findings)
}

// ── Delivery (observed data) ──────────────────────────────────────────────────

/// Observed interval evidence for one delivery point.
#[derive(Debug, Clone, Default)]
pub struct DeliveryEvidenceInput {
    /// Detected interval length across the observation window, as returned by
    /// [`crate::detect_interval_length`].
    pub resolution: Option<IntervalResolution>,
    /// Classification derived from the observed series.
    pub messtyp: Option<Messtyp>,
    /// Share of expected quarter-hour slots actually present, 0.0–100.0.
    pub coverage_pct: Option<f64>,
    /// Number of readings inspected.
    pub reading_count: u64,
    /// Most recent reading timestamp, if any.
    pub last_reading_at: Option<OffsetDateTime>,
}

/// Whether the point is in fact delivering a quarter-hour series.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "SCREAMING_SNAKE_CASE"))]
pub enum Delivery {
    /// Quarter-hour values observed at or above the coverage threshold.
    Delivering,
    /// Values observed, but not at quarter-hour resolution or below threshold.
    Insufficient,
    /// No readings in the observation window.
    Absent,
}

impl Delivery {
    /// Every outcome, in declaration order.
    pub const ALL: [Self; 3] = [Self::Delivering, Self::Insufficient, Self::Absent];

    /// Stable DB/wire label. Matches the `serde` tag and
    /// [`FromStr`](std::str::FromStr) input.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Delivering => "DELIVERING",
            Self::Insufficient => "INSUFFICIENT",
            Self::Absent => "ABSENT",
        }
    }
}

/// Minimum share of expected quarter-hour slots for a point to count as
/// delivering.
///
/// §42c fixes no coverage figure; this is an operational threshold for the
/// readiness report, deliberately strict because a sharing allocation cannot
/// close an interval it has no value for.
pub const DEFAULT_COVERAGE_THRESHOLD_PCT: f64 = 95.0;

/// Assess actual quarter-hour delivery from observed intervals.
#[must_use]
pub fn assess_delivery(
    input: &DeliveryEvidenceInput,
    coverage_threshold_pct: f64,
) -> (Delivery, Vec<Finding>) {
    if input.reading_count == 0 {
        return (Delivery::Absent, vec![Finding::NoReadings]);
    }

    if input.resolution != Some(IntervalResolution::QuarterHour) {
        let finding = if input.resolution.is_some() {
            Finding::NotQuarterHourResolution
        } else {
            Finding::ResolutionUndeterminable
        };
        return (Delivery::Insufficient, vec![finding]);
    }

    if input
        .coverage_pct
        .is_some_and(|cov| cov < coverage_threshold_pct)
    {
        return (
            Delivery::Insufficient,
            vec![Finding::CoverageBelowThreshold],
        );
    }

    (Delivery::Delivering, Vec::new())
}

// ── Combined verdict ──────────────────────────────────────────────────────────

/// Overall §42c readiness for one delivery point.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "SCREAMING_SNAKE_CASE"))]
pub enum SharingReadiness {
    /// Capable and delivering — can join a sharing community today.
    Ready,
    /// Capable, but no conforming quarter-hour series is arriving.
    ///
    /// The actionable middle state: needs a Zählerstandsgangmessung
    /// configuration order, not a meter rollout.
    CapableNotDelivering,
    /// Master data rules the point out — an iMSys rollout or RLM is required.
    NotCapable,
    /// Insufficient master data to decide.
    Unknown,
}

impl SharingReadiness {
    /// Every verdict, in declaration order.
    pub const ALL: [Self; 4] = [
        Self::Ready,
        Self::CapableNotDelivering,
        Self::NotCapable,
        Self::Unknown,
    ];

    /// Stable DB/wire label. Matches the `serde` tag and
    /// [`FromStr`](std::str::FromStr) input.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ready => "READY",
            Self::CapableNotDelivering => "CAPABLE_NOT_DELIVERING",
            Self::NotCapable => "NOT_CAPABLE",
            Self::Unknown => "UNKNOWN",
        }
    }

    /// The operator action this verdict calls for.
    #[must_use]
    pub const fn required_action(self) -> &'static str {
        match self {
            Self::Ready => "keine",
            Self::CapableNotDelivering => "Zählerstandsgangmessung beauftragen",
            Self::NotCapable => "iMSys-Rollout oder RLM-Umbau beauftragen",
            Self::Unknown => "Stammdaten vervollständigen",
        }
    }
}

/// Combine a capability and a delivery assessment into one verdict.
///
/// Delivery alone never establishes eligibility: §42c is a statement about the
/// measurement installed at the point, so a conforming series from an
/// unidentifiable meter still leaves the master data to be fixed.
#[must_use]
pub const fn combine_readiness(capability: Capability, delivery: Delivery) -> SharingReadiness {
    match (capability, delivery) {
        (Capability::Qualified(_), Delivery::Delivering) => SharingReadiness::Ready,
        (Capability::Qualified(_), _) => SharingReadiness::CapableNotDelivering,
        (Capability::Disqualified, _) => SharingReadiness::NotCapable,
        (Capability::Unknown, _) => SharingReadiness::Unknown,
    }
}

crate::codes::string_codes! {
    EligibilityBasis;
    Finding;
    Zaehlertyp;
    Bilanzierungsmethode;
    Delivery;
    SharingReadiness;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cap(
        zt: Option<Zaehlertyp>,
        fern: Option<bool>,
        bm: Option<Bilanzierungsmethode>,
    ) -> MeteringCapabilityInput {
        MeteringCapabilityInput {
            zaehlertyp: zt,
            ist_fernauslesbar: fern,
            bilanzierungsmethode: bm,
            smgw_operational: None,
        }
    }

    #[test]
    fn rlm_qualifies_without_a_gateway() {
        // §42c Abs. 1: "oder durch eine viertelstündliche registrierende
        // Leistungsmessung" — an independent limb.
        let (c, _) = assess_capability(&cap(
            Some(Zaehlertyp::Conventional),
            None,
            Some(Bilanzierungsmethode::Rlm),
        ));
        assert_eq!(
            c,
            Capability::Qualified(EligibilityBasis::RegistrierendeLeistungsmessung)
        );
    }

    #[test]
    fn imsys_qualifies_on_the_zsg_limb() {
        let (c, _) = assess_capability(&cap(
            Some(Zaehlertyp::IntelligentesMesssystem),
            Some(true),
            None,
        ));
        assert_eq!(
            c,
            Capability::Qualified(EligibilityBasis::Zaehlerstandsgangmessung)
        );
    }

    #[test]
    fn imsys_that_cannot_be_read_remotely_is_disqualified() {
        let (c, reasons) = assess_capability(&cap(
            Some(Zaehlertyp::IntelligentesMesssystem),
            Some(false),
            Some(Bilanzierungsmethode::Ims),
        ));
        assert_eq!(c, Capability::Disqualified);
        assert_eq!(reasons, vec![Finding::NotRemotelyReadable]);
    }

    #[test]
    fn moderne_messeinrichtung_is_not_sufficient() {
        // §2 Satz 1 Nr. 15 MsbG — no gateway, no interval series.
        let (c, reasons) =
            assess_capability(&cap(Some(Zaehlertyp::ModerneMesseinrichtung), None, None));
        assert_eq!(c, Capability::Disqualified);
        assert_eq!(reasons, vec![Finding::ModerneMesseinrichtungWithoutGateway]);
    }

    #[test]
    fn slp_is_disqualified() {
        let (c, _) = assess_capability(&cap(
            Some(Zaehlertyp::Conventional),
            None,
            Some(Bilanzierungsmethode::Slp),
        ));
        assert_eq!(c, Capability::Disqualified);
    }

    #[test]
    fn missing_master_data_is_unknown_not_a_guess() {
        let (c, reasons) = assess_capability(&cap(None, None, None));
        assert_eq!(c, Capability::Unknown);
        assert_eq!(
            reasons,
            vec![
                Finding::ZaehlertypMissing,
                Finding::BilanzierungsmethodeMissing
            ],
            "both gaps reported"
        );
    }

    #[test]
    fn delivery_requires_quarter_hour_resolution() {
        let ev = DeliveryEvidenceInput {
            resolution: Some(IntervalResolution::Hour),
            reading_count: 100,
            ..Default::default()
        };
        let (d, reasons) = assess_delivery(&ev, DEFAULT_COVERAGE_THRESHOLD_PCT);
        assert_eq!(d, Delivery::Insufficient);
        assert_eq!(reasons, vec![Finding::NotQuarterHourResolution]);

        // "could not tell" is a different finding from "told, and it is wrong".
        let unknown = DeliveryEvidenceInput {
            resolution: None,
            reading_count: 100,
            ..Default::default()
        };
        let (_, reasons) = assess_delivery(&unknown, DEFAULT_COVERAGE_THRESHOLD_PCT);
        assert_eq!(reasons, vec![Finding::ResolutionUndeterminable]);
    }

    #[test]
    fn delivery_requires_coverage_above_threshold() {
        let ev = DeliveryEvidenceInput {
            resolution: Some(IntervalResolution::QuarterHour),
            coverage_pct: Some(80.0),
            reading_count: 100,
            ..Default::default()
        };
        let (d, reasons) = assess_delivery(&ev, DEFAULT_COVERAGE_THRESHOLD_PCT);
        assert_eq!(d, Delivery::Insufficient);
        assert_eq!(reasons, vec![Finding::CoverageBelowThreshold]);
    }

    /// Findings are a closed vocabulary a caller can match on, and every one of
    /// them can still be rendered for a human.
    #[test]
    fn findings_are_matchable_and_renderable() {
        for f in Finding::ALL {
            assert!(!f.description_de().is_empty(), "{f:?}");
        }
        let (_, reasons) = assess_capability(&cap(None, None, None));
        assert!(reasons.contains(&Finding::ZaehlertypMissing));
    }

    #[test]
    fn capable_but_silent_is_its_own_verdict() {
        // The state the readiness report exists to surface.
        let verdict = combine_readiness(
            Capability::Qualified(EligibilityBasis::Zaehlerstandsgangmessung),
            Delivery::Absent,
        );
        assert_eq!(verdict, SharingReadiness::CapableNotDelivering);
        assert_eq!(
            verdict.required_action(),
            "Zählerstandsgangmessung beauftragen"
        );
    }

    #[test]
    fn delivery_alone_does_not_establish_eligibility() {
        assert_eq!(
            combine_readiness(Capability::Unknown, Delivery::Delivering),
            SharingReadiness::Unknown
        );
    }
}
