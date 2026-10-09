//! HeizkostenV — the heat and warm-water quantities the Verordnung defines.
//!
//! | Clause | Function | Quantity |
//! |---|---|---|
//! | § 9 Abs. 2 | [`warm_water_heat_kwh`], [`warm_water_heat_kwh_unmetered`] | heat attributable to central warm water |
//! | § 9 Abs. 3 | [`brennstoffverbrauch`] | the fuel quantity B of the warm-water system |
//! | § 6a Abs. 2 | [`verbrauchsinformation`] | the monthly consumption and its two comparisons |
//! | § 9a | [`verbrauchsschaetzung`] | whether a consumption estimate is permitted (the 25 % rule) |
//!
//! Out of scope: cost distribution (§§ 7, 8), the Gradtagszahlen of § 9b and
//! the witterungsbereinigte comparison of § 6a Abs. 3.
//! Source: HeizkostenV (`gesetze-im-internet.de`, as in force).

use rust_decimal::{Decimal, dec};
use time::Month;

use crate::series::Series;
use crate::series::interval::Unit;
use crate::time::calendar::Period;

// ── Warm water → heat energy (HeizkostenV §9 Abs. 2) ─────────────────────────

/// The `2,5` of HeizkostenV §9 Abs. 2 Satz 2 — a Zahlenwertgleichung constant.
const WARMWASSER_FAKTOR: Decimal = dec!(2.5);

/// The assumed cold-water inlet temperature of §9 Abs. 2 Satz 2, in °C.
const KALTWASSER_TEMPERATUR_C: Decimal = dec!(10);

/// The `32` of HeizkostenV §9 Abs. 2 Satz 4, in kWh per m² and year.
const WARMWASSER_FLAECHENFAKTOR: Decimal = dec!(32);

/// §9 Abs. 2 Satz 6: *"bei brennwertbezogener Abrechnung von Erdgas mit 1,11 zu
/// multiplizieren"*.
const BRENNWERT_ERDGAS_FAKTOR: Decimal = dec!(1.11);

/// §9 Abs. 2 Satz 6: *"bei eigenständiger gewerblicher Wärmelieferung durch
/// 1,15 zu dividieren"*.
const GEWERBLICHE_WAERMELIEFERUNG_DIVISOR: Decimal = dec!(1.15);

/// §9 Abs. 2 Satz 6: *"bei dem Betrieb einer monovalenten Wärmepumpe mit 0,30
/// zu multiplizieren"*.
const MONOVALENTE_WAERMEPUMPE_FAKTOR: Decimal = dec!(0.30);

/// Adjustments applied to a §9 Abs. 2 result.
///
/// §9 Abs. 2 Satz 6 applies them to either equation's result; they are not
/// exclusive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct WarmWaterAdjustments {
    /// *"bei brennwertbezogener Abrechnung von Erdgas mit 1,11 zu multiplizieren"*.
    pub brennwert_erdgas: bool,
    /// *"bei eigenständiger gewerblicher Wärmelieferung durch 1,15 zu dividieren"*.
    ///
    /// **Eigenständig** is a term of art (cf. §1 Abs. 1 Nr. 2).
    pub eigenstaendige_gewerbliche_waermelieferung: bool,
    /// *"bei dem Betrieb einer monovalenten Wärmepumpe mit 0,30 zu multiplizieren"*.
    pub monovalente_waermepumpe: bool,
}

impl WarmWaterAdjustments {
    /// No adjustment.
    pub const NONE: Self = Self {
        brennwert_erdgas: false,
        eigenstaendige_gewerbliche_waermelieferung: false,
        monovalente_waermepumpe: false,
    };

    fn apply(self, base: Decimal) -> Decimal {
        let mut q = base;
        if self.brennwert_erdgas {
            q *= BRENNWERT_ERDGAS_FAKTOR;
        }
        if self.eigenstaendige_gewerbliche_waermelieferung {
            q /= GEWERBLICHE_WAERMELIEFERUNG_DIVISOR;
        }
        if self.monovalente_waermepumpe {
            q *= MONOVALENTE_WAERMEPUMPE_FAKTOR;
        }
        q
    }
}

/// Heat attributable to a central warm-water system from the **metered volume**,
/// per HeizkostenV §9 Abs. 2 Satz 2.
///
/// ```text
/// Q [kWh/a] = 2.5 × V [m³] × (t_w [°C] − 10)
/// ```
///
/// The fallback to a Wärmezähler (§9 Abs. 2 Satz 1), admitted where measurement
/// *"nur mit einem unzumutbar hohen Aufwand"* is possible. A
/// Zahlenwertgleichung; since 2.5 includes the Erzeugeraufwandszahl (Satz 3
/// Nr. 1), **Q is generator-input heat, not delivered useful heat**.
///
/// `mean_temp_c` is *"die gemessene oder geschätzte mittlere Temperatur"*.
/// `None` below the 10 °C cold-water reference (a negative heat quantity);
/// exactly 10 °C gives zero.
///
/// # Example
///
/// ```rust
/// use metering::{heat::heizkosten::warm_water_heat_kwh, heat::heizkosten::WarmWaterAdjustments};
/// use rust_decimal::Decimal;
///
/// // 40 m³ of warm water at 60 °C
/// let q = warm_water_heat_kwh(
///     Decimal::from(40u32),
///     Decimal::from(60u32),
///     WarmWaterAdjustments::NONE,
/// );
/// assert_eq!(q, Some(Decimal::from(5000u32))); // 2.5 × 40 × 50
///
/// // Below the 10 °C cold-water reference: refused.
/// assert_eq!(
///     warm_water_heat_kwh(Decimal::from(40u32), Decimal::from(5u32), WarmWaterAdjustments::NONE),
///     None,
/// );
/// ```
#[must_use]
pub fn warm_water_heat_kwh(
    volume_m3: Decimal,
    mean_temp_c: Decimal,
    adjustments: WarmWaterAdjustments,
) -> Option<Decimal> {
    (mean_temp_c >= KALTWASSER_TEMPERATUR_C).then(|| {
        adjustments.apply(WARMWASSER_FAKTOR * volume_m3 * (mean_temp_c - KALTWASSER_TEMPERATUR_C))
    })
}

/// Heat attributable to a central warm-water system from **floor area**, per
/// HeizkostenV §9 Abs. 2 Satz 4: `Q [kWh/a] = 32 × A_Wohn [m²]`.
///
/// Admitted only *"in Ausnahmefällen"* where **neither** the heat quantity **nor**
/// the warm-water volume can be measured. `flaeche_m2` is the *"Wohn- oder
/// Nutzfläche"* supplied by the central system. Unlike the 2.5 of Satz 2, the
/// 32 excludes Speicher-, Verteilungs- und Zirkulationsverluste (Satz 5 Nr. 1).
#[must_use]
pub fn warm_water_heat_kwh_unmetered(
    flaeche_m2: Decimal,
    adjustments: WarmWaterAdjustments,
) -> Decimal {
    adjustments.apply(WARMWASSER_FLAECHENFAKTOR * flaeche_m2)
}

// ── § 9 Abs. 3 — the fuel quantity ───────────────────────────────────────────

/// A fuel of the § 9 Abs. 3 Satz 4 table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Brennstoff {
    /// Leichtes Heizöl extra leichtflüssig — 10 kWh/l.
    LeichtesHeizoel,
    /// Schweres Heizöl — 10,9 kWh/l.
    SchweresHeizoel,
    /// Erdgas H — 10 kWh/m³.
    ErdgasH,
    /// Erdgas L — 9 kWh/m³.
    ErdgasL,
    /// Flüssiggas — 13 kWh/kg.
    Fluessiggas,
    /// Koks — 8 kWh/kg.
    Koks,
    /// Braunkohle — 5,5 kWh/kg.
    Braunkohle,
    /// Steinkohle — 8 kWh/kg.
    Steinkohle,
    /// Brennholz (lufttrocken) — 4,1 kWh/kg.
    Brennholz,
    /// Holzpellets — 5 kWh/kg.
    Holzpellets,
    /// Holzhackschnitzel (lufttrocken) — 4 kWh/kg.
    Holzhackschnitzel,
}

/// The unit a fuel quantity B is measured in: *"in Litern, Kubikmetern oder
/// Kilogramm"* (§ 9 Abs. 3 Satz 1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Brennstoffeinheit {
    /// l.
    Liter,
    /// m³.
    Kubikmeter,
    /// kg.
    Kilogramm,
}

impl Brennstoff {
    /// Every fuel, in the order of the table.
    pub const ALL: [Self; 11] = [
        Self::LeichtesHeizoel,
        Self::SchweresHeizoel,
        Self::ErdgasH,
        Self::ErdgasL,
        Self::Fluessiggas,
        Self::Koks,
        Self::Braunkohle,
        Self::Steinkohle,
        Self::Brennholz,
        Self::Holzpellets,
        Self::Holzhackschnitzel,
    ];

    /// Stable code.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::LeichtesHeizoel => "LEICHTES_HEIZOEL",
            Self::SchweresHeizoel => "SCHWERES_HEIZOEL",
            Self::ErdgasH => "ERDGAS_H",
            Self::ErdgasL => "ERDGAS_L",
            Self::Fluessiggas => "FLUESSIGGAS",
            Self::Koks => "KOKS",
            Self::Braunkohle => "BRAUNKOHLE",
            Self::Steinkohle => "STEINKOHLE",
            Self::Brennholz => "BRENNHOLZ",
            Self::Holzpellets => "HOLZPELLETS",
            Self::Holzhackschnitzel => "HOLZHACKSCHNITZEL",
        }
    }

    /// The statutory fallback Heizwert Hi, in kWh per [`einheit`](Self::einheit)
    /// — the table of § 9 Abs. 3 Satz 4, usable *"hilfsweise"* when the
    /// supplier's Abrechnungsunterlagen state none.
    #[must_use]
    pub const fn heizwert(self) -> Decimal {
        match self {
            Self::LeichtesHeizoel | Self::ErdgasH => dec!(10),
            Self::SchweresHeizoel => dec!(10.9),
            Self::ErdgasL => dec!(9),
            Self::Fluessiggas => dec!(13),
            Self::Koks | Self::Steinkohle => dec!(8),
            Self::Braunkohle => dec!(5.5),
            Self::Brennholz => dec!(4.1),
            Self::Holzpellets => dec!(5),
            Self::Holzhackschnitzel => dec!(4),
        }
    }

    /// The unit the Heizwert refers to.
    #[must_use]
    pub const fn einheit(self) -> Brennstoffeinheit {
        match self {
            Self::LeichtesHeizoel | Self::SchweresHeizoel => Brennstoffeinheit::Liter,
            Self::ErdgasH | Self::ErdgasL => Brennstoffeinheit::Kubikmeter,
            _ => Brennstoffeinheit::Kilogramm,
        }
    }
}

/// Which Heizwert the fuel quantity is computed with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Heizwert {
    /// The Heizwert *"in den Abrechnungsunterlagen des
    /// Energieversorgungsunternehmens oder Brennstofflieferanten"*, kWh per
    /// unit — it takes precedence (§ 9 Abs. 3 Satz 3). The only choice for a
    /// fuel the table does not list.
    Abrechnungsunterlagen(Decimal),
    /// The statutory fallback of the table (§ 9 Abs. 3 Satz 4).
    Hilfsweise(Brennstoff),
}

/// The § 9 Abs. 3 fuel quantity and the Heizwert it used.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct Brennstoffmenge {
    /// B — fuel consumed by the central warm-water system, in the fuel's unit.
    pub menge: Decimal,
    /// Hi used, kWh per unit.
    pub heizwert_kwh: Decimal,
    /// Where Hi came from.
    pub quelle: Heizwert,
}

/// HeizkostenV § 9 Abs. 3: the fuel consumption B of the central warm-water
/// system, B = Q ÷ Hi.
///
/// Q is the § 9 Abs. 2 heat quantity in kWh, Hi the Heizwert in kWh per
/// litre, cubic metre or kilogram (Satz 2 Nr. 1, 2). Unrounded.
///
/// `None` for a negative Q, a Heizwert that is not positive, or overflow.
///
/// ```rust
/// use metering::heat::heizkosten::{Brennstoff, Heizwert, brennstoffverbrauch};
/// use rust_decimal::dec;
///
/// // 5 000 kWh of warm-water heat from Erdgas H at the fallback 10 kWh/m³.
/// let b = brennstoffverbrauch(dec!(5000), Heizwert::Hilfsweise(Brennstoff::ErdgasH)).unwrap();
/// assert_eq!(b.heizwert_kwh, dec!(10));
/// assert_eq!(b.menge, dec!(500)); // m³
/// ```
#[must_use]
pub fn brennstoffverbrauch(q_kwh: Decimal, heizwert: Heizwert) -> Option<Brennstoffmenge> {
    let hi = match heizwert {
        Heizwert::Abrechnungsunterlagen(v) => v,
        Heizwert::Hilfsweise(f) => f.heizwert(),
    };
    if hi <= Decimal::ZERO || q_kwh < Decimal::ZERO {
        return None;
    }
    Some(Brennstoffmenge {
        menge: q_kwh.checked_div(hi)?,
        heizwert_kwh: hi,
        quelle: heizwert,
    })
}

// ── § 6a — monthly consumption information ──────────────────────────────────

/// What the remotely read series measures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Erfassung {
    /// A heat meter: values are kWh.
    Waermemenge,
    /// Heizkostenverteiler: values are units. § 6a Abs. 2 Nr. 1 asks for kWh,
    /// and the Verordnung defines no conversion — the factor is the caller's.
    Heizkostenverteiler {
        /// kWh per Verteiler unit, > 0.
        kwh_je_einheit: Decimal,
    },
}

/// Why § 6a information could not be produced.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum HeizkostenError {
    /// An interval runs across a month boundary, so it belongs to neither
    /// month.
    #[error("the interval starting {at} runs across a month boundary")]
    Straddles {
        /// Its start.
        at: time::OffsetDateTime,
    },
    /// The series' channel names a unit other than kWh for a heat meter.
    #[error("a heat-meter series must be in kWh")]
    Unit,
    /// The conversion factor of a Heizkostenverteiler is not positive.
    #[error("the kWh per Heizkostenverteiler unit must be positive")]
    Factor,
    /// The month lies outside the supported calendar.
    #[error("the month is outside the supported calendar")]
    Calendar,
    /// A sum or product left the `Decimal` range.
    #[error("arithmetic overflow")]
    Overflow,
}

/// One month's consumption, and whether the series covers all of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct Monatsverbrauch {
    /// The Berlin calendar month.
    pub monat: Period,
    /// Consumption in the covered part of the month, kWh.
    pub kwh: Decimal,
    /// `true` when billable intervals cover the whole month.
    pub vollstaendig: bool,
}

/// The § 6a Abs. 2 Nr. 1 and 2 figures of one month.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct Verbrauchsinformation {
    /// Nr. 1 — *"Verbrauch des Nutzers im letzten Monat in
    /// Kilowattstunden"*. `None` when the series holds no value in it.
    pub monat: Option<Monatsverbrauch>,
    /// Nr. 2 — the previous month; `None` when no data were recorded
    /// (*"soweit diese Daten erhoben worden sind"*).
    pub vormonat: Option<Monatsverbrauch>,
    /// Nr. 2 — the same month of the previous year; `None` likewise.
    pub vorjahresmonat: Option<Monatsverbrauch>,
}

/// HeizkostenV § 6a Abs. 2: the month's consumption in kWh and the two
/// comparisons with the same user's data.
///
/// The Nr. 3 comparison (*"normierten oder durch Vergleichstests
/// ermittelten Durchschnittsnutzers derselben Nutzerkategorie"*) is the
/// caller's figure. Months are Berlin calendar months on the series' day
/// boundary; non-billable values count as not covered.
///
/// # Errors
///
/// [`HeizkostenError`] for a straddling interval, a heat-meter series not
/// in kWh, a non-positive conversion factor, a month outside the calendar,
/// or overflow.
pub fn verbrauchsinformation(
    series: &Series,
    erfassung: Erfassung,
    year: i32,
    month: Month,
) -> Result<Verbrauchsinformation, HeizkostenError> {
    let factor = match erfassung {
        Erfassung::Waermemenge => {
            if series.unit().is_some_and(|u| u != Unit::KiloWattHour) {
                return Err(HeizkostenError::Unit);
            }
            Decimal::ONE
        }
        Erfassung::Heizkostenverteiler { kwh_je_einheit } => {
            if kwh_je_einheit <= Decimal::ZERO {
                return Err(HeizkostenError::Factor);
            }
            kwh_je_einheit
        }
    };
    let boundary = series.boundary();
    let previous = if month == Month::January {
        (
            year.checked_sub(1).ok_or(HeizkostenError::Calendar)?,
            Month::December,
        )
    } else {
        (year, month.previous())
    };
    let figure = |y: i32, m: Month| -> Result<Option<Monatsverbrauch>, HeizkostenError> {
        let Some(period) = boundary.month(y, m) else {
            return Ok(None);
        };
        let mut kwh = Decimal::ZERO;
        let mut covered = time::Duration::ZERO;
        let mut any = false;
        for iv in series.iter() {
            if iv.to() <= period.start() || iv.from() >= period.end() {
                continue;
            }
            if iv.from() < period.start() || iv.to() > period.end() {
                return Err(HeizkostenError::Straddles { at: iv.from() });
            }
            if !iv.quality().is_billable() {
                continue;
            }
            any = true;
            kwh = kwh
                .checked_add(iv.value())
                .ok_or(HeizkostenError::Overflow)?;
            covered += iv.duration();
        }
        Ok(any.then_some(Monatsverbrauch {
            monat: period,
            kwh: kwh.checked_mul(factor).ok_or(HeizkostenError::Overflow)?,
            vollstaendig: covered == period.duration(),
        }))
    };
    if boundary.month(year, month).is_none() {
        return Err(HeizkostenError::Calendar);
    }
    Ok(Verbrauchsinformation {
        monat: figure(year, month)?,
        vormonat: figure(previous.0, previous.1)?,
        vorjahresmonat: figure(year.checked_sub(1).ok_or(HeizkostenError::Calendar)?, month)?,
    })
}

// ── § 9a — estimation ────────────────────────────────────────────────────────

/// The 25 of § 9a Abs. 2: *"25 vom Hundert"* of the area or volume, above
/// which consumption may not be estimated.
pub const SCHAETZGRENZE_PROZENT: Decimal = dec!(25);

/// The basis a § 9a Abs. 1 estimate is made on — one of the three the clause
/// names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Schaetzgrundlage {
    /// *"Verbrauchs der betroffenen Räume in vergleichbaren Zeiträumen"*.
    VergleichbareZeitraeume,
    /// *"Verbrauchs vergleichbarer anderer Räume im jeweiligen
    /// Abrechnungszeitraum"*.
    VergleichbareRaeume,
    /// *"Durchschnittsverbrauchs des Gebäudes oder der Nutzergruppe"*.
    Durchschnittsverbrauch,
}

impl Schaetzgrundlage {
    /// Every basis, in the clause's order.
    pub const ALL: [Self; 3] = [
        Self::VergleichbareZeitraeume,
        Self::VergleichbareRaeume,
        Self::Durchschnittsverbrauch,
    ];

    /// Stable code.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::VergleichbareZeitraeume => "VERGLEICHBARE_ZEITRAEUME",
            Self::VergleichbareRaeume => "VERGLEICHBARE_RAEUME",
            Self::Durchschnittsverbrauch => "DURCHSCHNITTSVERBRAUCH",
        }
    }
}

crate::ids::codes::string_codes! {
    Brennstoff;
    Schaetzgrundlage;
}

/// A § 9a Abs. 1 estimate that the 25 % rule admits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct Verbrauchsschaetzung {
    /// The basis it was made on.
    pub grundlage: Schaetzgrundlage,
    /// The estimated consumption — the caller's figure on that basis; the
    /// Verordnung names the bases, not an arithmetic.
    pub verbrauch: Decimal,
    /// The affected share of the area or volume, in per cent.
    pub anteil_prozent: Decimal,
}

/// Why a § 9a estimate is refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum SchaetzungError {
    /// § 9a Abs. 2: the affected area or volume *"überschreitet"* 25 % of the
    /// total, so the costs are distributed by the fixed-cost key alone.
    #[error("the affected share exceeds 25 % (§ 9a Abs. 2 HeizkostenV)")]
    UeberSchaetzgrenze,
    /// The total is not positive, or the affected part is negative or larger
    /// than it.
    #[error("the affected and total area or volume are inconsistent")]
    Flaeche,
}

/// HeizkostenV § 9a: admit an estimate of a user's consumption, on a named
/// basis, unless the area (or volume) affected exceeds 25 % of the total.
///
/// `betroffen` and `gesamt` are both Wohn- oder Nutzfläche, or both umbauter
/// Raum, in the same unit. Exactly 25 % is admitted (Abs. 2: *"überschreitet"*).
///
/// # Errors
///
/// [`SchaetzungError::UeberSchaetzgrenze`] above 25 %;
/// [`SchaetzungError::Flaeche`] for inconsistent areas or overflow.
pub fn verbrauchsschaetzung(
    grundlage: Schaetzgrundlage,
    verbrauch: Decimal,
    betroffen: Decimal,
    gesamt: Decimal,
) -> Result<Verbrauchsschaetzung, SchaetzungError> {
    if gesamt <= Decimal::ZERO || betroffen < Decimal::ZERO || betroffen > gesamt {
        return Err(SchaetzungError::Flaeche);
    }
    let hundred = Decimal::ONE_HUNDRED;
    let lhs = betroffen
        .checked_mul(hundred)
        .ok_or(SchaetzungError::Flaeche)?;
    let rhs = gesamt
        .checked_mul(SCHAETZGRENZE_PROZENT)
        .ok_or(SchaetzungError::Flaeche)?;
    if lhs > rhs {
        return Err(SchaetzungError::UeberSchaetzgrenze);
    }
    Ok(Verbrauchsschaetzung {
        grundlage,
        verbrauch,
        anteil_prozent: lhs.checked_div(gesamt).ok_or(SchaetzungError::Flaeche)?,
    })
}

#[cfg(test)]
mod quantity_tests {
    use super::*;
    use crate::series::interval::{MeterInterval, QualityFlag};
    use crate::time::calendar::DayBoundary;
    use crate::time::resolution::Resolution;
    use time::Date;

    /// Every fallback Heizwert of the § 9 Abs. 3 table, and its unit.
    #[test]
    fn the_heizwert_table_is_the_statutes() {
        let table: Vec<(Brennstoff, Decimal)> =
            Brennstoff::ALL.iter().map(|f| (*f, f.heizwert())).collect();
        assert_eq!(
            table,
            [
                (Brennstoff::LeichtesHeizoel, dec!(10)),
                (Brennstoff::SchweresHeizoel, dec!(10.9)),
                (Brennstoff::ErdgasH, dec!(10)),
                (Brennstoff::ErdgasL, dec!(9)),
                (Brennstoff::Fluessiggas, dec!(13)),
                (Brennstoff::Koks, dec!(8)),
                (Brennstoff::Braunkohle, dec!(5.5)),
                (Brennstoff::Steinkohle, dec!(8)),
                (Brennstoff::Brennholz, dec!(4.1)),
                (Brennstoff::Holzpellets, dec!(5)),
                (Brennstoff::Holzhackschnitzel, dec!(4)),
            ]
        );
        assert_eq!(
            Brennstoff::SchweresHeizoel.einheit(),
            Brennstoffeinheit::Liter
        );
        assert_eq!(Brennstoff::ErdgasH.einheit(), Brennstoffeinheit::Kubikmeter);
        assert_eq!(
            Brennstoff::Fluessiggas.einheit(),
            Brennstoffeinheit::Kilogramm
        );
    }

    /// B = Q / Hi; the supplier's Heizwert takes precedence and is reported.
    #[test]
    fn fuel_quantity_reports_its_heizwert() {
        let b = brennstoffverbrauch(
            dec!(5000),
            Heizwert::Hilfsweise(Brennstoff::LeichtesHeizoel),
        )
        .unwrap();
        assert_eq!(b.menge, dec!(500));
        let b =
            brennstoffverbrauch(dec!(5000), Heizwert::Abrechnungsunterlagen(dec!(10.4))).unwrap();
        assert_eq!(b.heizwert_kwh, dec!(10.4));
        assert_eq!(b.quelle, Heizwert::Abrechnungsunterlagen(dec!(10.4)));
        assert!(brennstoffverbrauch(dec!(1), Heizwert::Abrechnungsunterlagen(dec!(0))).is_none());
        assert!(brennstoffverbrauch(dec!(-1), Heizwert::Hilfsweise(Brennstoff::Koks)).is_none());
    }

    /// Exactly 25 % is admitted; anything above is refused.
    #[test]
    fn the_25_percent_rule() {
        assert_eq!(SCHAETZGRENZE_PROZENT, dec!(25));
        let ok = verbrauchsschaetzung(
            Schaetzgrundlage::VergleichbareRaeume,
            dec!(1200),
            dec!(25),
            dec!(100),
        )
        .unwrap();
        assert_eq!(ok.anteil_prozent, dec!(25));
        assert_eq!(ok.grundlage, Schaetzgrundlage::VergleichbareRaeume);
        assert_eq!(
            verbrauchsschaetzung(
                Schaetzgrundlage::VergleichbareRaeume,
                dec!(1),
                dec!(25.01),
                dec!(100)
            ),
            Err(SchaetzungError::UeberSchaetzgrenze)
        );
        assert_eq!(
            verbrauchsschaetzung(
                Schaetzgrundlage::Durchschnittsverbrauch,
                dec!(1),
                dec!(1),
                dec!(0)
            ),
            Err(SchaetzungError::Flaeche)
        );
    }

    fn daily(first: Date, values: &[Decimal]) -> Series {
        let ivs = values
            .iter()
            .enumerate()
            .map(|(i, &v)| {
                let day = DayBoundary::Strom
                    .day(first + time::Duration::days(i as i64))
                    .unwrap();
                MeterInterval::new(day.start(), day.end(), v, QualityFlag::Measured).unwrap()
            })
            .collect();
        Series::new(Resolution::Day, DayBoundary::Strom, ivs).unwrap()
    }

    /// A month with the spring transition: 31 daily values, one of them a
    /// 23-hour day, cover March completely.
    #[test]
    fn monthly_information_on_the_berlin_calendar() {
        let feb_mar = daily(time::macros::date!(2026 - 02 - 01), &vec![dec!(2); 28 + 31]);
        let info =
            verbrauchsinformation(&feb_mar, Erfassung::Waermemenge, 2026, Month::March).unwrap();
        let m = info.monat.unwrap();
        assert_eq!((m.kwh, m.vollstaendig), (dec!(62), true));
        assert_eq!(info.vormonat.unwrap().kwh, dec!(56));
        assert_eq!(info.vorjahresmonat, None);

        // Moved in on 17 March: the month is partial and says so.
        let partial = daily(time::macros::date!(2026 - 03 - 17), &vec![dec!(1); 15]);
        let m = verbrauchsinformation(
            &partial,
            Erfassung::Heizkostenverteiler {
                kwh_je_einheit: dec!(3),
            },
            2026,
            Month::March,
        )
        .unwrap()
        .monat
        .unwrap();
        assert_eq!((m.kwh, m.vollstaendig), (dec!(45), false));
        assert_eq!(
            verbrauchsinformation(
                &partial,
                Erfassung::Heizkostenverteiler {
                    kwh_je_einheit: dec!(0)
                },
                2026,
                Month::March
            ),
            Err(HeizkostenError::Factor)
        );
    }

    /// An interval across the month is refused, naming its start.
    #[test]
    fn an_interval_across_the_month_is_refused() {
        let year = DayBoundary::Strom.year(2026).unwrap();
        let iv = MeterInterval::new(year.start(), year.end(), dec!(1200), QualityFlag::Measured)
            .unwrap();
        let series = Series::new(Resolution::Year, DayBoundary::Strom, vec![iv]).unwrap();
        assert_eq!(
            verbrauchsinformation(&series, Erfassung::Waermemenge, 2026, Month::March),
            Err(HeizkostenError::Straddles { at: year.start() })
        );
    }

    /// A heat-meter series labelled with a volume channel is refused.
    #[test]
    fn a_heat_meter_series_not_in_kwh_is_refused() {
        let m3: crate::ObisCode = "7-0:3.0.0".parse().unwrap();
        let ivs = (0..31)
            .map(|i| {
                let day = DayBoundary::Strom
                    .day(time::macros::date!(2026 - 03 - 01) + time::Duration::days(i))
                    .unwrap();
                MeterInterval::new(day.start(), day.end(), dec!(1), QualityFlag::Measured)
                    .unwrap()
                    .with_obis(m3)
            })
            .collect();
        let series = Series::new(Resolution::Day, DayBoundary::Strom, ivs).unwrap();
        assert_ne!(series.unit(), Some(Unit::KiloWattHour));
        assert_eq!(
            verbrauchsinformation(&series, Erfassung::Waermemenge, 2026, Month::March),
            Err(HeizkostenError::Unit)
        );
    }
}

#[cfg(test)]
mod warm_water_tests {
    use super::*;

    fn d(s: &str) -> Decimal {
        Decimal::from_str_exact(s).unwrap()
    }

    /// The worked identity from HeizkostenV §9 Abs. 2 Satz 2.
    #[test]
    fn metered_warm_water_follows_the_statutory_formula() {
        // 2.5 × 40 m³ × (60 − 10) = 5000 kWh
        assert_eq!(
            warm_water_heat_kwh(
                Decimal::from(40u32),
                Decimal::from(60u32),
                WarmWaterAdjustments::NONE
            ),
            Some(Decimal::from(5000u32))
        );
    }

    /// Zero at the cold-inlet temperature, refused below it.
    #[test]
    fn at_and_below_cold_inlet_temperature() {
        assert_eq!(
            warm_water_heat_kwh(
                Decimal::from(40u32),
                Decimal::from(10u32),
                WarmWaterAdjustments::NONE
            ),
            Some(Decimal::ZERO)
        );
        assert_eq!(
            warm_water_heat_kwh(
                Decimal::from(40u32),
                Decimal::from(5u32),
                WarmWaterAdjustments::NONE
            ),
            None
        );
    }

    #[test]
    fn adjustments_match_the_statutory_factors() {
        let v = Decimal::from(40u32);
        let t = Decimal::from(60u32);
        let base = Decimal::from(5000u32);

        let brennwert = WarmWaterAdjustments {
            brennwert_erdgas: true,
            ..WarmWaterAdjustments::NONE
        };
        assert_eq!(warm_water_heat_kwh(v, t, brennwert), Some(base * d("1.11")));

        let wp = WarmWaterAdjustments {
            monovalente_waermepumpe: true,
            ..WarmWaterAdjustments::NONE
        };
        assert_eq!(warm_water_heat_kwh(v, t, wp), Some(base * d("0.30")));

        // Eigenständige gewerbliche Wärmelieferung divides.
        let gewerblich = WarmWaterAdjustments {
            eigenstaendige_gewerbliche_waermelieferung: true,
            ..WarmWaterAdjustments::NONE
        };
        assert_eq!(
            warm_water_heat_kwh(v, t, gewerblich),
            Some(base / d("1.15"))
        );
    }

    /// §9 Abs. 2 Satz 6 adjustments are not exclusive.
    #[test]
    fn adjustments_compose() {
        let both = WarmWaterAdjustments {
            eigenstaendige_gewerbliche_waermelieferung: true,
            monovalente_waermepumpe: true,
            ..WarmWaterAdjustments::NONE
        };
        let q = warm_water_heat_kwh(Decimal::from(40u32), Decimal::from(60u32), both);
        assert_eq!(q, Some(Decimal::from(5000u32) / d("1.15") * d("0.30")));
    }

    /// The adjustments apply to the floor-area equation too ("Satz 2 oder 4").
    #[test]
    fn unmetered_fallback_uses_floor_area_and_takes_adjustments() {
        // 32 × 75 m² = 2400 kWh
        assert_eq!(
            warm_water_heat_kwh_unmetered(Decimal::from(75u32), WarmWaterAdjustments::NONE),
            Decimal::from(2400u32)
        );
        let brennwert = WarmWaterAdjustments {
            brennwert_erdgas: true,
            ..WarmWaterAdjustments::NONE
        };
        assert_eq!(
            warm_water_heat_kwh_unmetered(Decimal::from(75u32), brennwert),
            Decimal::from(2400u32) * d("1.11")
        );
    }
}
