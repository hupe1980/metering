//! § 42c EnWG Energy Sharing: whether a delivery point can take part,
//! assessed from master data and from the series it delivers.
//!
//! § 42c EnWG is in force since 22 December 2025 (BGBl. 2025 I Nr. 347); Abs. 4
//! obliges every Verteilernetzbetreiber to enable sharing from 1 June 2026
//! *"innerhalb des Bilanzierungsgebietes eines
//! Elektrizitätsverteilernetzbetreibers"* and from 1 June 2028 also into a
//! directly adjacent Bilanzierungsgebiet in the same Regelzone.
//!
//! Abs. 1 admits a point only when consumption *and* generation are measured
//! by:
//!
//! > „Zählerstandsgangmessung nach § 2 Satz 1 Nummer 27 des
//! > Messstellenbetriebsgesetzes **oder** durch eine viertelstündliche
//! > registrierende Leistungsmessung"
//!
//! Two independent bases ([`EligibilityBasis`]): a conventional RLM meter
//! qualifies without an iMSys, and an iMSys not configured for
//! Zählerstandsgangmessung delivers no quarter-hour series.
//!
//! | Step | Question | Input |
//! |---|---|---|
//! | [`assess_capability`] | Can the point produce quarter-hour values? | device master data |
//! | [`assess_delivery`] | Is it producing them? | observed series |
//! | [`combine_readiness`] | What must the operator do? | both verdicts |
//!
//! Definitions (§ 2 Satz 1 MsbG):
//!
//! - **Nr. 27 Zählerstandsgangmessung**: „die Messung einer
//!   Reihe viertelstündig ermittelter Zählerstände von elektrischer Arbeit und
//!   stündlich ermittelter Zählerstände von Gasmengen". § 42c is Strom-only.
//! - **Nr. 7 intelligentes Messsystem (iMSys)**: a moderne Messeinrichtung or
//!   an RLM meter bound into a communication network via a Smart-Meter-Gateway.
//! - **Nr. 15 moderne Messeinrichtung (mME)**: no gateway, no interval series;
//!   not sufficient on its own.

use crate::series::Series;
use crate::time::calendar::Period;
use crate::time::resolution::Resolution;

// ── Qualifying basis ──────────────────────────────────────────────────────────

/// The statutory basis on which a delivery point qualifies under § 42c Abs. 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EligibilityBasis {
    /// Zählerstandsgangmessung per § 2 Satz 1 Nr. 27 MsbG — the iMSys route.
    Zaehlerstandsgangmessung,
    /// Viertelstündliche registrierende Leistungsmessung — the RLM route; needs
    /// no Smart-Meter-Gateway.
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

/// Why an assessment reached the verdict it did — a code to match on;
/// [`description_de`](Self::description_de) renders it.
///
/// Exhaustive on purpose: each finding routes to an action, so a new one
/// breaks a consumer's `match`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Finding {
    /// The meter is flagged as not remotely readable, so no series of
    /// viertelstündig ermittelte Zählerstände can be transmitted.
    NotRemotelyReadable,
    /// The Smart-Meter-Gateway is not operational, so the iMSys transmits
    /// nothing.
    GatewayNotOperational,
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
    /// Coverage is below the configured threshold.
    CoverageBelowThreshold,
}

impl Finding {
    /// Stable DB/wire label. Matches the `serde` tag and
    /// [`FromStr`](std::str::FromStr) input.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NotRemotelyReadable => "NOT_REMOTELY_READABLE",
            Self::GatewayNotOperational => "GATEWAY_NOT_OPERATIONAL",
            Self::ModerneMesseinrichtungWithoutGateway => "MODERNE_MESSEINRICHTUNG_WITHOUT_GATEWAY",
            Self::BalancingMethodHasNoQuarterHourValues => {
                "BALANCING_METHOD_HAS_NO_QUARTER_HOUR_VALUES"
            }
            Self::MeterTypeQualifiesForNeitherLimb => "METER_TYPE_QUALIFIES_FOR_NEITHER_LIMB",
            Self::ZaehlertypMissing => "ZAEHLERTYP_MISSING",
            Self::BilanzierungsmethodeMissing => "BILANZIERUNGSMETHODE_MISSING",
            Self::NoReadings => "NO_READINGS",
            Self::NotQuarterHourResolution => "NOT_QUARTER_HOUR_RESOLUTION",
            Self::CoverageBelowThreshold => "COVERAGE_BELOW_THRESHOLD",
        }
    }

    /// Every finding, in declaration order.
    pub const ALL: [Self; 10] = [
        Self::NotRemotelyReadable,
        Self::GatewayNotOperational,
        Self::ModerneMesseinrichtungWithoutGateway,
        Self::BalancingMethodHasNoQuarterHourValues,
        Self::MeterTypeQualifiesForNeitherLimb,
        Self::ZaehlertypMissing,
        Self::BilanzierungsmethodeMissing,
        Self::NoReadings,
        Self::NotQuarterHourResolution,
        Self::CoverageBelowThreshold,
    ];

    /// A German rendering for an operator-facing report.
    #[must_use]
    pub const fn description_de(self) -> &'static str {
        match self {
            Self::NotRemotelyReadable => {
                "Zähler ist als nicht fernauslesbar gekennzeichnet — keine \
                 Zählerstandsgangmessung möglich"
            }
            Self::GatewayNotOperational => {
                "Smart-Meter-Gateway nicht in Betrieb — keine Zählerstandsgangmessung"
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
            Self::CoverageBelowThreshold => "Abdeckung unter der Schwelle",
        }
    }
}

// ── Capability (master data) ──────────────────────────────────────────────────

/// The metering equipment installed at a delivery point: the BO4E
/// `Zaehlertyp` value set narrowed to the distinctions § 42c turns on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Zaehlertyp {
    /// Intelligentes Messsystem — a modern meter behind a Smart-Meter-Gateway
    /// (§ 2 Satz 1 Nr. 7 MsbG).
    IntelligentesMesssystem,
    /// Moderne Messeinrichtung — no gateway, no interval series (§ 2 Satz 1
    /// Nr. 15 MsbG); not sufficient for § 42c on its own.
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
pub enum Bilanzierungsmethode {
    /// Registrierende Leistungsmessung — the second § 42c limb, sufficient on
    /// its own and needing no gateway.
    Rlm,
    /// Standardlastprofil — no quarter-hour values.
    Slp,
    /// Zählerstandsgangmessung at an intelligentes Messsystem.
    Ims,
    /// Temperaturabhängiges Lastprofil (Gas); never qualifies.
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

/// Device master data for one delivery point. A missing field yields a
/// finding, never a guess.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MeteringCapabilityInput {
    /// The installed metering equipment.
    pub zaehlertyp: Option<Zaehlertyp>,
    /// BO4E `Zaehler.istFernauslesbar`; `Some(false)` blocks the
    /// Zählerstandsgang limb.
    pub ist_fernauslesbar: Option<bool>,
    /// How the Marktlokation is balanced.
    pub bilanzierungsmethode: Option<Bilanzierungsmethode>,
    /// Whether the Smart-Meter-Gateway is operational; `Some(false)` blocks
    /// the Zählerstandsgang limb.
    pub smgw_operational: Option<bool>,
}

/// Outcome of the master-data capability assessment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "SCREAMING_SNAKE_CASE"))]
pub enum Capability {
    /// Master data supports a § 42c-qualifying measurement.
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

/// Master data that cannot all be true at once; refused rather than assessed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum Contradiction {
    /// A moderne Messeinrichtung balanced as RLM or as iMSys: it has neither a
    /// load-profile register nor a gateway (§ 2 Satz 1 Nr. 15 MsbG).
    #[error("a moderne Messeinrichtung cannot be balanced as {0}")]
    ModerneMesseinrichtungBalancedAs(Bilanzierungsmethode),
    /// A conventional meter balanced as iMSys (Zählerstandsgang).
    #[error("a conventional meter cannot be balanced as IMS")]
    ConventionalBalancedAsIms,
    /// An operational gateway on a meter type that is not an iMSys: a meter
    /// behind a working gateway is one (§ 2 Satz 1 Nr. 7 MsbG).
    #[error("an operational gateway on a {0} meter")]
    GatewayOnNonImsys(Zaehlertyp),
}

/// Assess § 42c capability from device master data.
///
/// `RLM` qualifies on its own limb. `IMS`, an iMSys meter or an operational
/// gateway qualify on the Zählerstandsgang limb unless the point is reported
/// not remotely readable or its gateway down. A moderne Messeinrichtung, or
/// `SLP`/`TLP`/`Pauschal`, is disqualified; missing fields give
/// [`Capability::Unknown`].
///
/// # Errors
///
/// A [`Contradiction`] for master data that cannot all be true.
pub fn assess_capability(
    input: &MeteringCapabilityInput,
) -> Result<(Capability, Vec<Finding>), Contradiction> {
    let methode = input.bilanzierungsmethode;
    let typ = input.zaehlertyp;
    match (typ, methode) {
        (
            Some(Zaehlertyp::ModerneMesseinrichtung),
            Some(m @ (Bilanzierungsmethode::Rlm | Bilanzierungsmethode::Ims)),
        ) => return Err(Contradiction::ModerneMesseinrichtungBalancedAs(m)),
        (Some(Zaehlertyp::Conventional), Some(Bilanzierungsmethode::Ims)) => {
            return Err(Contradiction::ConventionalBalancedAsIms);
        }
        _ => {}
    }
    if input.smgw_operational == Some(true)
        && let Some(t @ (Zaehlertyp::ModerneMesseinrichtung | Zaehlertyp::Conventional)) = typ
    {
        return Err(Contradiction::GatewayOnNonImsys(t));
    }

    // Limb 2 — viertelstündliche registrierende Leistungsmessung.
    if methode == Some(Bilanzierungsmethode::Rlm) {
        return Ok((
            Capability::Qualified(EligibilityBasis::RegistrierendeLeistungsmessung),
            Vec::new(),
        ));
    }

    // Limb 1 — Zählerstandsgangmessung via iMSys.
    let looks_imsys = methode == Some(Bilanzierungsmethode::Ims)
        || typ == Some(Zaehlertyp::IntelligentesMesssystem)
        || input.smgw_operational == Some(true);
    if looks_imsys {
        let mut findings = Vec::new();
        if input.ist_fernauslesbar == Some(false) {
            findings.push(Finding::NotRemotelyReadable);
        }
        if input.smgw_operational == Some(false) {
            findings.push(Finding::GatewayNotOperational);
        }
        if findings.is_empty() {
            return Ok((
                Capability::Qualified(EligibilityBasis::Zaehlerstandsgangmessung),
                findings,
            ));
        }
        return Ok((Capability::Disqualified, findings));
    }

    let mut findings = Vec::new();
    if typ == Some(Zaehlertyp::ModerneMesseinrichtung) {
        findings.push(Finding::ModerneMesseinrichtungWithoutGateway);
        return Ok((Capability::Disqualified, findings));
    }
    if methode.is_some_and(Bilanzierungsmethode::precludes_quarter_hour_values) {
        findings.push(Finding::BalancingMethodHasNoQuarterHourValues);
        return Ok((Capability::Disqualified, findings));
    }

    if typ.is_none() {
        findings.push(Finding::ZaehlertypMissing);
    }
    if methode.is_none() {
        findings.push(Finding::BilanzierungsmethodeMissing);
    }
    if findings.is_empty() {
        findings.push(Finding::MeterTypeQualifiesForNeitherLimb);
        return Ok((Capability::Disqualified, findings));
    }
    Ok((Capability::Unknown, findings))
}

// ── Delivery (observed data) ──────────────────────────────────────────────────

/// Whether the point is in fact delivering a quarter-hour series.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

/// Minimum share of the window's quarter-hours, in percent, for a point to
/// count as delivering. § 42c fixes none; this is an operational choice.
pub const DEFAULT_COVERAGE_THRESHOLD_PCT: u8 = 95;

/// Assess actual quarter-hour delivery from the observed series over `window`.
///
/// Coverage is the billable intervals of `series` inside `window` against
/// the quarter-hours the window holds (DST-aware), compared in integers.
/// No billable interval in the window is [`Absent`](Delivery::Absent); a
/// non-quarter-hour series is [`Insufficient`](Delivery::Insufficient).
#[must_use]
pub fn assess_delivery(
    series: &Series,
    window: Period,
    coverage_threshold_pct: u8,
) -> (Delivery, Vec<Finding>) {
    let present = series
        .iter()
        .filter(|iv| {
            iv.quality().is_billable() && window.start() <= iv.from() && iv.to() <= window.end()
        })
        .count();
    if present == 0 {
        return (Delivery::Absent, vec![Finding::NoReadings]);
    }
    if series.resolution() != Resolution::QUARTER_HOUR {
        return (
            Delivery::Insufficient,
            vec![Finding::NotQuarterHourResolution],
        );
    }
    let expected = window.count(Resolution::QUARTER_HOUR).unwrap_or(0);
    let covered = u64::try_from(present)
        .unwrap_or(u64::MAX)
        .saturating_mul(100);
    if covered < u64::from(expected) * u64::from(coverage_threshold_pct) {
        return (
            Delivery::Insufficient,
            vec![Finding::CoverageBelowThreshold],
        );
    }
    (Delivery::Delivering, Vec::new())
}

// ── Combined verdict ──────────────────────────────────────────────────────────

/// Overall § 42c readiness for one delivery point.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SharingReadiness {
    /// Capable and delivering — can join a sharing community today.
    Ready,
    /// Capable, but no conforming quarter-hour series is arriving: needs a
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
/// Delivery alone never establishes eligibility: § 42c is about the
/// measurement installed, so capability decides first.
#[must_use]
pub const fn combine_readiness(capability: Capability, delivery: Delivery) -> SharingReadiness {
    match (capability, delivery) {
        (Capability::Qualified(_), Delivery::Delivering) => SharingReadiness::Ready,
        (Capability::Qualified(_), _) => SharingReadiness::CapableNotDelivering,
        (Capability::Disqualified, _) => SharingReadiness::NotCapable,
        (Capability::Unknown, _) => SharingReadiness::Unknown,
    }
}

crate::ids::codes::string_codes! {
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
    use crate::series::interval::{MeterInterval, QualityFlag};
    use crate::time::calendar::DayBoundary;
    use rust_decimal::dec;
    use time::Duration;
    use time::macros::date;

    fn cap(
        zt: Option<Zaehlertyp>,
        fern: Option<bool>,
        bm: Option<Bilanzierungsmethode>,
        smgw: Option<bool>,
    ) -> Result<(Capability, Vec<Finding>), Contradiction> {
        assess_capability(&MeteringCapabilityInput {
            zaehlertyp: zt,
            ist_fernauslesbar: fern,
            bilanzierungsmethode: bm,
            smgw_operational: smgw,
        })
    }

    #[test]
    fn rlm_qualifies_without_a_gateway() {
        let (c, _) = cap(
            Some(Zaehlertyp::Conventional),
            None,
            Some(Bilanzierungsmethode::Rlm),
            None,
        )
        .unwrap();
        assert_eq!(
            c,
            Capability::Qualified(EligibilityBasis::RegistrierendeLeistungsmessung)
        );
    }

    #[test]
    fn imsys_qualifies_on_the_zsg_limb() {
        let (c, _) = cap(
            Some(Zaehlertyp::IntelligentesMesssystem),
            Some(true),
            None,
            None,
        )
        .unwrap();
        assert_eq!(
            c,
            Capability::Qualified(EligibilityBasis::Zaehlerstandsgangmessung)
        );
    }

    #[test]
    fn an_imsys_whose_gateway_is_down_is_disqualified() {
        let (c, findings) = cap(
            Some(Zaehlertyp::IntelligentesMesssystem),
            Some(true),
            Some(Bilanzierungsmethode::Ims),
            Some(false),
        )
        .unwrap();
        assert_eq!(c, Capability::Disqualified);
        assert_eq!(findings, vec![Finding::GatewayNotOperational]);
    }

    #[test]
    fn imsys_that_cannot_be_read_remotely_is_disqualified() {
        let (c, findings) = cap(
            Some(Zaehlertyp::IntelligentesMesssystem),
            Some(false),
            Some(Bilanzierungsmethode::Ims),
            None,
        )
        .unwrap();
        assert_eq!(c, Capability::Disqualified);
        assert_eq!(findings, vec![Finding::NotRemotelyReadable]);
    }

    #[test]
    fn contradictory_master_data_is_refused() {
        assert_eq!(
            cap(
                Some(Zaehlertyp::ModerneMesseinrichtung),
                Some(false),
                Some(Bilanzierungsmethode::Rlm),
                None
            ),
            Err(Contradiction::ModerneMesseinrichtungBalancedAs(
                Bilanzierungsmethode::Rlm
            ))
        );
        assert_eq!(
            cap(
                Some(Zaehlertyp::Conventional),
                None,
                Some(Bilanzierungsmethode::Ims),
                None
            ),
            Err(Contradiction::ConventionalBalancedAsIms)
        );
        assert_eq!(
            cap(
                Some(Zaehlertyp::ModerneMesseinrichtung),
                None,
                None,
                Some(true)
            ),
            Err(Contradiction::GatewayOnNonImsys(
                Zaehlertyp::ModerneMesseinrichtung
            ))
        );
    }

    #[test]
    fn moderne_messeinrichtung_and_slp_are_not_sufficient() {
        let (c, f) = cap(Some(Zaehlertyp::ModerneMesseinrichtung), None, None, None).unwrap();
        assert_eq!(
            (c, f),
            (
                Capability::Disqualified,
                vec![Finding::ModerneMesseinrichtungWithoutGateway]
            )
        );
        let (c, _) = cap(
            Some(Zaehlertyp::Conventional),
            None,
            Some(Bilanzierungsmethode::Slp),
            None,
        )
        .unwrap();
        assert_eq!(c, Capability::Disqualified);
    }

    #[test]
    fn missing_master_data_is_unknown_not_a_guess() {
        let (c, findings) = cap(None, None, None, None).unwrap();
        assert_eq!(c, Capability::Unknown);
        assert_eq!(
            findings,
            vec![
                Finding::ZaehlertypMissing,
                Finding::BilanzierungsmethodeMissing
            ]
        );
        for f in Finding::ALL {
            assert!(!f.description_de().is_empty(), "{f:?}");
        }
    }

    fn day_series(resolution: Resolution, present: usize) -> (Series, Period) {
        let day = DayBoundary::Strom.day(date!(2026 - 06 - 01)).unwrap();
        let step = resolution.fixed_seconds().map(i64::from).unwrap_or(900);
        let ivs = (0..present)
            .map(|i| {
                let from = day.start() + Duration::seconds(step * i as i64);
                MeterInterval::new(
                    from,
                    from + Duration::seconds(step),
                    dec!(1),
                    QualityFlag::Measured,
                )
                .unwrap()
            })
            .collect();
        (
            Series::new(resolution, DayBoundary::Strom, ivs).unwrap(),
            day,
        )
    }

    #[test]
    fn delivery_coverage_is_computed_from_the_series() {
        let (full, day) = day_series(Resolution::QUARTER_HOUR, 96);
        assert_eq!(
            assess_delivery(&full, day, DEFAULT_COVERAGE_THRESHOLD_PCT),
            (Delivery::Delivering, vec![])
        );
        // 91 of 96 is 94.8 %: below 95.
        let (thin, day) = day_series(Resolution::QUARTER_HOUR, 91);
        assert_eq!(
            assess_delivery(&thin, day, DEFAULT_COVERAGE_THRESHOLD_PCT),
            (
                Delivery::Insufficient,
                vec![Finding::CoverageBelowThreshold]
            )
        );
        let (hourly, day) = day_series(Resolution::Hour, 24);
        assert_eq!(
            assess_delivery(&hourly, day, DEFAULT_COVERAGE_THRESHOLD_PCT),
            (
                Delivery::Insufficient,
                vec![Finding::NotQuarterHourResolution]
            )
        );
    }

    /// One old reading outside the window is no delivery at all.
    #[test]
    fn readings_outside_the_window_do_not_count() {
        let (old, _) = day_series(Resolution::QUARTER_HOUR, 1);
        let later = DayBoundary::Strom.day(date!(2026 - 07 - 01)).unwrap();
        assert_eq!(
            assess_delivery(&old, later, DEFAULT_COVERAGE_THRESHOLD_PCT),
            (Delivery::Absent, vec![Finding::NoReadings])
        );
    }

    #[test]
    fn capable_but_silent_is_its_own_verdict() {
        let verdict = combine_readiness(
            Capability::Qualified(EligibilityBasis::Zaehlerstandsgangmessung),
            Delivery::Absent,
        );
        assert_eq!(verdict, SharingReadiness::CapableNotDelivering);
        assert_eq!(
            combine_readiness(Capability::Unknown, Delivery::Delivering),
            SharingReadiness::Unknown
        );
    }
}
