//! § 14a EnWG netzorientierte Steuerung — the two powers a control decision
//! turns on.
//!
//! BNetzA **BK6-22-300** (27.11.2023, in force 01.01.2024) lets a
//! Netzbetreiber reduce the *netzwirksamer Leistungsbezug* of steuerbare
//! Verbrauchseinrichtungen, down to a guaranteed floor. Both are powers in kW:
//!
//! | Quantity | What it is | Source |
//! |---|---|---|
//! | [`netzwirksamer_leistungsbezug`] | the share of the grid draw the steuVE cause | Anlage 1 Ziff. 2.3 |
//! | [`mindestleistung_direktansteuerung`] / [`mindestleistung_ems`] | the floor `P_min,14a` below which it may not be pushed | Anlage 1 Ziff. 4.5.1 / 4.5.2, grouped per Ziff. 2.4.2 by [`steuerbare_verbrauchseinrichtungen`] |
//!
//! `P_min,14a` follows the Festlegung's formula and table exactly. Ziff. 2.3
//! only defines the share, not how to apportion it when local generation
//! covers part of the load, so that is a caller-chosen [`Verursachungsregel`].
//!
//! Ziff. 4.7 makes a separate Zählpunkt for the steuVE optional. Without a
//! sub-measurement, pass `measured.unwrap_or(nennleistung)`: the
//! Netzanschlussleistung can only overstate the steuVE share.
//!
//! The Netzentgelt modules are BK8-22/010-A; Modul 3's tariff windows are
//! [`crate::billing::zaehlzeit`].

use rust_decimal::Decimal;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

// ── Fallgruppen ───────────────────────────────────────────────────────────────

/// The four kinds of steuerbare Verbrauchseinrichtung of Anlage 1 Ziff. 2.4.1.
///
/// All four qualify only *"mit einer Netzanschlussleistung von mehr als
/// 4,2 Kilowatt (kW) und einem unmittelbaren oder mittelbaren Anschluss in der
/// Niederspannung"*.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum SteuVeFallgruppe {
    /// **a** — a Ladepunkt für Elektromobile that is not publicly accessible
    /// under § 2 Nr. 5 LSV.
    Ladepunkt,
    /// **b** — a Wärmepumpenheizung, including Zusatz- or Notheizvorrichtungen
    /// such as Heizstäbe.
    Waermepumpe,
    /// **c** — an Anlage zur Raumkühlung.
    Raumkuehlung,
    /// **d** — a Stromspeicher, in respect of its Einspeicherung.
    Stromspeicher,
}

impl SteuVeFallgruppe {
    /// Every Fallgruppe, in the order Ziff. 2.4.1 lists them.
    pub const ALL: [Self; 4] = [
        Self::Ladepunkt,
        Self::Waermepumpe,
        Self::Raumkuehlung,
        Self::Stromspeicher,
    ];

    /// Stable DB/wire label; the `serde` tag and [`FromStr`](std::str::FromStr) input.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ladepunkt => "LADEPUNKT",
            Self::Waermepumpe => "WAERMEPUMPE",
            Self::Raumkuehlung => "RAUMKUEHLUNG",
            Self::Stromspeicher => "STROMSPEICHER",
        }
    }

    /// The sub-item of Ziff. 2.4.1 this Fallgruppe is.
    #[must_use]
    pub const fn ziffer(self) -> &'static str {
        match self {
            Self::Ladepunkt => "2.4.1.a",
            Self::Waermepumpe => "2.4.1.b",
            Self::Raumkuehlung => "2.4.1.c",
            Self::Stromspeicher => "2.4.1.d",
        }
    }

    /// `true` for Fallgruppen b and c — grouped by Ziff. 2.4.2 and scaled above
    /// 11 kW by Ziff. 4.5.1 Satz 2 / 4.5.2 Satz 3.
    #[must_use]
    pub const fn is_grouped_and_scaled(self) -> bool {
        matches!(self, Self::Waermepumpe | Self::Raumkuehlung)
    }
}

crate::ids::codes::string_codes! {
    SteuVeFallgruppe;
}

// ── the published figures ─────────────────────────────────────────────────────

/// The Netzanschlussleistung a device — or a Ziff. 2.4.2 group — must
/// **exceed** to be a steuerbare Verbrauchseinrichtung: **4,2 kW**
/// (Anlage 1 Ziff. 2.4.1: *"mit einer Netzanschlussleistung von mehr als
/// 4,2 Kilowatt (kW)"*).
pub const STEUVE_SCHWELLE_KW: Decimal = Decimal::from_parts(42, 0, 0, false, 1);

/// The flat Mindestleistung per steuVE: **4,2 kW** (Ziff. 4.5.1 Satz 1:
/// *"beträgt die Mindestleistung 4,2 kW"*), and the `4,2 kW` of both
/// Ziff. 4.5.2 formulas.
///
/// Numerically equal to [`STEUVE_SCHWELLE_KW`], but a different provision.
pub const MINDESTLEISTUNG_KW: Decimal = Decimal::from_parts(42, 0, 0, false, 1);

/// The Netzanschlussleistung above which Fallgruppen b and c scale instead of
/// taking the flat floor: **11 kW** (Ziff. 4.5.1 Satz 2, Ziff. 4.5.2 Satz 3 —
/// *"eine Netzanschlussleistung über 11 kW"*). Strict.
pub const SKALIERUNG_SCHWELLE_KW: Decimal = Decimal::from_parts(11, 0, 0, false, 0);

/// The Skalierungsfaktor presumed appropriate: **0,4** (Ziff. 4.5.1 Satz 3:
/// *"Bis zum Inkrafttreten einer anderweitigen Empfehlung wird die
/// Angemessenheit vermutet, wenn der Skalierungsfaktor 0,4 beträgt."*).
pub const SKALIERUNGSFAKTOR: Decimal = Decimal::from_parts(4, 0, 0, false, 1);

/// The Gleichzeitigkeitsfaktoren of Ziff. 4.5.2, for `n_steuVE` = 2, 3, … ≥ 9:
///
/// | `n_steuVE` | 2 | 3 | 4 | 5 | 6 | 7 | 8 | ≥ 9 |
/// |---|---|---|---|---|---|---|---|---|
/// | GZF | 0,8 | 0,75 | 0,7 | 0,65 | 0,6 | 0,55 | 0,5 | 0,45 |
///
/// Presumed, like the whole of Ziff. 4.5.2 — *"Bis zum Inkrafttreten einer
/// anderweitigen Empfehlung wird die Angemessenheit vermutet, wenn die
/// Berechnung wie nachstehend erfolgt"*.
pub const GLEICHZEITIGKEITSFAKTOREN: [Decimal; 8] = [
    Decimal::from_parts(80, 0, 0, false, 2),
    Decimal::from_parts(75, 0, 0, false, 2),
    Decimal::from_parts(70, 0, 0, false, 2),
    Decimal::from_parts(65, 0, 0, false, 2),
    Decimal::from_parts(60, 0, 0, false, 2),
    Decimal::from_parts(55, 0, 0, false, 2),
    Decimal::from_parts(50, 0, 0, false, 2),
    Decimal::from_parts(45, 0, 0, false, 2),
];

/// The two parameters Ziff. 4.5 only **presumes** appropriate — the
/// Skalierungsfaktor and the Gleichzeitigkeitsfaktor table — which an
/// *anderweitige Empfehlung* may replace.
///
/// [`VERMUTUNG`](Self::VERMUTUNG) is the Festlegung's presumption; there is no
/// `Default`, so the caller names its basis.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[non_exhaustive]
pub struct Para14aConfig {
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::decimal"))]
    skalierungsfaktor: Decimal,
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::decimal_array"))]
    gleichzeitigkeitsfaktoren: [Decimal; 8],
}

impl Para14aConfig {
    /// The presumption of Ziff. 4.5.1 Satz 3 and Ziff. 4.5.2 Satz 2:
    /// [`SKALIERUNGSFAKTOR`] and [`GLEICHZEITIGKEITSFAKTOREN`].
    pub const VERMUTUNG: Self = Self {
        skalierungsfaktor: SKALIERUNGSFAKTOR,
        gleichzeitigkeitsfaktoren: GLEICHZEITIGKEITSFAKTOREN,
    };

    /// Replace the Skalierungsfaktor. `None` unless `0 < factor ≤ 1`.
    #[must_use]
    pub fn skalierungsfaktor(mut self, factor: Decimal) -> Option<Self> {
        (factor > Decimal::ZERO && factor <= Decimal::ONE).then(|| {
            self.skalierungsfaktor = factor;
            self
        })
    }

    /// Replace the Gleichzeitigkeitsfaktor table (`n` = 2 … ≥ 9). `None`
    /// unless every factor is in `(0, 1]`.
    #[must_use]
    pub fn gleichzeitigkeitsfaktoren(mut self, table: [Decimal; 8]) -> Option<Self> {
        table
            .iter()
            .all(|g| *g > Decimal::ZERO && *g <= Decimal::ONE)
            .then(|| {
                self.gleichzeitigkeitsfaktoren = table;
                self
            })
    }

    /// The Gleichzeitigkeitsfaktor for `n` EMS-controlled steuVE.
    ///
    /// `None` for `n < 2` (the term it multiplies is zero at `n = 1`); `n ≥ 9`
    /// takes the last entry.
    ///
    /// ```rust
    /// use metering::grid::para14a::Para14aConfig;
    /// use rust_decimal::dec;
    ///
    /// let cfg = Para14aConfig::VERMUTUNG;
    /// assert_eq!(cfg.gleichzeitigkeitsfaktor(2), Some(dec!(0.80)));
    /// assert_eq!(cfg.gleichzeitigkeitsfaktor(9), cfg.gleichzeitigkeitsfaktor(400));
    /// assert_eq!(cfg.gleichzeitigkeitsfaktor(1), None, "the table starts at two");
    /// ```
    #[must_use]
    pub fn gleichzeitigkeitsfaktor(&self, n_steuve: usize) -> Option<Decimal> {
        let index = n_steuve
            .checked_sub(2)?
            .min(self.gleichzeitigkeitsfaktoren.len() - 1);
        self.gleichzeitigkeitsfaktoren.get(index).copied()
    }
}

// ── Ziff. 2.4.2 — grouping ────────────────────────────────────────────────────

/// One Anlage behind the Netzanschluss, as installed — before Ziff. 2.4.2
/// decides what § 14a counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct Anlage {
    /// Which of the four Fallgruppen the Anlage belongs to.
    pub fallgruppe: SteuVeFallgruppe,
    /// Its own Netzanschlussleistung in kW.
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::decimal"))]
    pub netzanschlussleistung_kw: Decimal,
}

impl Anlage {
    /// An Anlage of `fallgruppe` with `netzanschlussleistung_kw` kW.
    #[must_use]
    pub const fn new(fallgruppe: SteuVeFallgruppe, netzanschlussleistung_kw: Decimal) -> Self {
        Self {
            fallgruppe,
            netzanschlussleistung_kw,
        }
    }
}

/// One steuerbare Verbrauchseinrichtung, as § 14a counts it — built only by
/// [`steuerbare_verbrauchseinrichtungen`], so it always clears 4,2 kW and a
/// Fallgruppe b or c steuVE is always the whole group.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize))]
pub struct SteuVe {
    fallgruppe: SteuVeFallgruppe,
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::decimal"))]
    netzanschlussleistung_kw: Decimal,
}

impl SteuVe {
    /// The Fallgruppe.
    #[must_use]
    pub const fn fallgruppe(&self) -> SteuVeFallgruppe {
        self.fallgruppe
    }

    /// The Netzanschlussleistung in kW — for Fallgruppe b and c, the sum over
    /// every Anlage of the group.
    #[must_use]
    pub const fn netzanschlussleistung_kw(&self) -> Decimal {
        self.netzanschlussleistung_kw
    }

    /// `true` when this steuVE scales instead of taking the flat floor:
    /// Fallgruppe b or c **and** a Netzanschlussleistung above 11 kW.
    #[must_use]
    pub fn is_scaled(&self) -> bool {
        self.fallgruppe.is_grouped_and_scaled()
            && self.netzanschlussleistung_kw > SKALIERUNG_SCHWELLE_KW
    }
}

/// The steuerbare Verbrauchseinrichtungen behind **one** Netzanschluss, per
/// Anlage 1 Ziff. 2.4.1 and 2.4.2.
///
/// All Wärmepumpen are **one** steuVE of their summed power, all
/// Raumkühlungen another, each only if the sum exceeds 4,2 kW (Ziff. 2.4.2); a
/// Ladepunkt or Stromspeicher counts alone when it exceeds 4,2 kW.
///
/// Order: Wärmepumpe group, Raumkühlung group, then the remaining Anlagen in
/// input order. `None` only when a group sum overflows.
#[must_use]
pub fn steuerbare_verbrauchseinrichtungen(anlagen: &[Anlage]) -> Option<Vec<SteuVe>> {
    let mut out = Vec::new();
    for gruppe in [
        SteuVeFallgruppe::Waermepumpe,
        SteuVeFallgruppe::Raumkuehlung,
    ] {
        let sum = anlagen
            .iter()
            .filter(|a| a.fallgruppe == gruppe)
            .try_fold(Decimal::ZERO, |s, a| {
                s.checked_add(a.netzanschlussleistung_kw)
            })?;
        if sum > STEUVE_SCHWELLE_KW {
            out.push(SteuVe {
                fallgruppe: gruppe,
                netzanschlussleistung_kw: sum,
            });
        }
    }
    out.extend(
        anlagen
            .iter()
            .filter(|a| {
                !a.fallgruppe.is_grouped_and_scaled()
                    && a.netzanschlussleistung_kw > STEUVE_SCHWELLE_KW
            })
            .map(|a| SteuVe {
                fallgruppe: a.fallgruppe,
                netzanschlussleistung_kw: a.netzanschlussleistung_kw,
            }),
    );
    Some(out)
}

// ── Ziff. 4.5.1 — Direktansteuerung ──────────────────────────────────────────

/// `P_min,14a` for **one** directly-controlled steuVE (Ziff. 4.4.a).
///
/// Ziff. 4.5.1: [`MINDESTLEISTUNG_KW`]; for a [scaled](SteuVe::is_scaled)
/// Fallgruppe b or c above 11 kW, Netzanschlussleistung × Skalierungsfaktor.
///
/// ```rust
/// use metering::grid::para14a::{Anlage, Para14aConfig, SteuVeFallgruppe as F, mindestleistung_direktansteuerung, steuerbare_verbrauchseinrichtungen};
/// use rust_decimal::dec;
///
/// let cfg = Para14aConfig::VERMUTUNG;
/// let steuve = steuerbare_verbrauchseinrichtungen(&[
///     Anlage::new(F::Ladepunkt, dec!(22)),
///     Anlage::new(F::Waermepumpe, dec!(20)),
/// ]).unwrap();
/// // The heat pump scales: 0,4 × 20 = 8 kW; the wallbox keeps the flat floor.
/// assert_eq!(mindestleistung_direktansteuerung(&steuve[0], &cfg), dec!(8.0));
/// assert_eq!(mindestleistung_direktansteuerung(&steuve[1], &cfg), dec!(4.2));
/// ```
#[must_use]
pub fn mindestleistung_direktansteuerung(steuve: &SteuVe, config: &Para14aConfig) -> Decimal {
    if steuve.is_scaled() {
        // ≤ the connection itself, since 0 < factor ≤ 1: no overflow.
        steuve.netzanschlussleistung_kw * config.skalierungsfaktor
    } else {
        MINDESTLEISTUNG_KW
    }
}

// ── Ziff. 4.5.2 — Steuerung mittels EMS ──────────────────────────────────────

/// `P_min,14a` for the Anlagen behind one Netzanschluss controlled through
/// **one** EMS (Ziff. 4.4.b) — grouped per Ziff. 2.4.2 here, not by the caller.
///
/// Ziff. 4.5.2:
///
/// ```text
/// b/c over 11 kW present: P_min,14a = Max(0,4 x P_Summe WP; 0,4 x P_Summe Klima) + (n_steuVE − 1) x GZF x 4,2 kW
/// otherwise:              P_min,14a = 4,2 kW + (n_steuVE − 1) x GZF x 4,2 kW
/// ```
///
/// `n_steuVE` counts **after** the Ziff. 2.4.2 grouping, and `0,4` is the
/// configured Skalierungsfaktor.
///
/// `None` when no Anlage forms a steuVE, or on overflow.
///
/// ```rust
/// use metering::grid::para14a::{Anlage, Para14aConfig, SteuVeFallgruppe as F, mindestleistung_ems};
/// use rust_decimal::dec;
///
/// let cfg = Para14aConfig::VERMUTUNG;
/// let three_heat_pumps = [Anlage::new(F::Waermepumpe, dec!(5)); 3];
/// assert_eq!(mindestleistung_ems(&three_heat_pumps, &cfg), Some(dec!(6.0)));
///
/// let two_small = [Anlage::new(F::Waermepumpe, dec!(3)); 2];
/// assert_eq!(mindestleistung_ems(&two_small, &cfg), Some(dec!(4.2)));
///
/// assert_eq!(mindestleistung_ems(&[], &cfg), None);
/// ```
#[must_use]
pub fn mindestleistung_ems(anlagen: &[Anlage], config: &Para14aConfig) -> Option<Decimal> {
    let steuve = steuerbare_verbrauchseinrichtungen(anlagen)?;
    let n = steuve.len();
    if n == 0 {
        return None;
    }
    let sum_of = |gruppe: SteuVeFallgruppe| -> Option<Decimal> {
        steuve
            .iter()
            .filter(|s| s.fallgruppe == gruppe)
            .try_fold(Decimal::ZERO, |acc, s| {
                acc.checked_add(s.netzanschlussleistung_kw)
            })
    };
    let base = if steuve.iter().any(SteuVe::is_scaled) {
        let wp = sum_of(SteuVeFallgruppe::Waermepumpe)?.checked_mul(config.skalierungsfaktor)?;
        let klima =
            sum_of(SteuVeFallgruppe::Raumkuehlung)?.checked_mul(config.skalierungsfaktor)?;
        wp.max(klima)
    } else {
        MINDESTLEISTUNG_KW
    };
    let rest = match config.gleichzeitigkeitsfaktor(n) {
        Some(gzf) => Decimal::from(n - 1)
            .checked_mul(gzf)?
            .checked_mul(MINDESTLEISTUNG_KW)?,
        // n == 1: the term is zero and the table has no entry for it.
        None => Decimal::ZERO,
    };
    base.checked_add(rest)
}

// ── Ziff. 2.3 — netzwirksamer Leistungsbezug ─────────────────────────────────

/// Which share of the grid draw the steuVE are taken to have caused.
///
/// Anlage 1 Ziff. 2.3 defines the netzwirksamer Leistungsbezug as *"derjenige
/// Anteil der über den Netzanschluss aus einem Elektrizitätsverteilernetz der
/// allgemeinen Versorgung entnommenen elektrischen Leistung, der zeitgleich
/// durch eine oder mehrere steuerbare Verbrauchseinrichtungen verursacht
/// wird"* but not how to apportion it when local generation covers part of
/// the load.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Verursachungsregel {
    /// Local generation serves the **uncontrollable** load first, so whatever
    /// grid draw is left is the steuVE's: `min(Netzbezug, P_steuVE)`.
    ///
    /// Conservative: never understates the steuVE share. Needs no figure for
    /// the rest of the installation.
    SteuVeZuletzt,

    /// Generation is shared pro rata, so the steuVE cause their share of the
    /// total draw: `Netzbezug × P_steuVE ÷ (P_steuVE + P_übrige)`.
    ///
    /// Needs the rest of the installation's draw. Never above
    /// [`SteuVeZuletzt`](Self::SteuVeZuletzt).
    Anteilig,
}

impl Verursachungsregel {
    /// Every convention, in declaration order.
    pub const ALL: [Self; 2] = [Self::SteuVeZuletzt, Self::Anteilig];

    /// Stable DB/wire label; the `serde` tag and [`FromStr`](std::str::FromStr) input.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::SteuVeZuletzt => "STEUVE_ZULETZT",
            Self::Anteilig => "ANTEILIG",
        }
    }
}

crate::ids::codes::string_codes! {
    Verursachungsregel;
}

/// The netzwirksamer Leistungsbezug in kW — the share of the grid draw caused
/// by the steuerbare Verbrauchseinrichtungen (Anlage 1 Ziff. 2.3).
///
/// `netzbezug_kw` is the draw **from** the grid; a negative value (export) and
/// a negative `steuve_kw` count as zero. `uebrige_last_kw` is the rest of the
/// installation's load; [`Verursachungsregel::Anteilig`] returns `None` without
/// it and caps the result at `steuve_kw`. `None` also on overflow.
///
/// ```rust
/// use metering::grid::para14a::{Verursachungsregel, netzwirksamer_leistungsbezug};
/// use rust_decimal::dec;
///
/// let conservative = netzwirksamer_leistungsbezug(
///     dec!(10), dec!(6), None, Verursachungsregel::SteuVeZuletzt,
/// );
/// assert_eq!(conservative, Some(dec!(6)));
///
/// let pro_rata = netzwirksamer_leistungsbezug(
///     dec!(10), dec!(6), Some(dec!(8)), Verursachungsregel::Anteilig,
/// );
/// assert!(pro_rata.unwrap() < dec!(4.3));
///
/// assert_eq!(
///     netzwirksamer_leistungsbezug(dec!(10), dec!(6), None, Verursachungsregel::Anteilig),
///     None,
/// );
/// ```
#[must_use]
pub fn netzwirksamer_leistungsbezug(
    netzbezug_kw: Decimal,
    steuve_kw: Decimal,
    uebrige_last_kw: Option<Decimal>,
    regel: Verursachungsregel,
) -> Option<Decimal> {
    let netzbezug = netzbezug_kw.max(Decimal::ZERO);
    let steuve = steuve_kw.max(Decimal::ZERO);

    match regel {
        Verursachungsregel::SteuVeZuletzt => Some(netzbezug.min(steuve)),
        Verursachungsregel::Anteilig => {
            let uebrige = uebrige_last_kw?.max(Decimal::ZERO);
            let gesamt = steuve.checked_add(uebrige)?;
            if gesamt.is_zero() {
                // No load at all draws no power, whatever the meter says.
                return Some(Decimal::ZERO);
            }
            Some(
                netzbezug
                    .checked_mul(steuve)?
                    .checked_div(gesamt)?
                    .min(steuve),
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use SteuVeFallgruppe as F;
    use rust_decimal::dec;

    const CFG: Para14aConfig = Para14aConfig::VERMUTUNG;

    fn steuve(anlagen: &[Anlage]) -> Vec<SteuVe> {
        steuerbare_verbrauchseinrichtungen(anlagen).unwrap()
    }

    /// Three 5 kW heat pumps are *one* steuVE of 15 kW (Ziff. 2.4.2), which
    /// scales: 0,4 × 15 = 6,0 kW — not the ungrouped n = 3 flat branch, 10,5.
    #[test]
    fn three_five_kilowatt_heat_pumps_are_one_steuve_of_fifteen() {
        let anlagen = [Anlage::new(F::Waermepumpe, dec!(5)); 3];
        let s = steuve(&anlagen);
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].netzanschlussleistung_kw(), dec!(15));
        assert_eq!(mindestleistung_ems(&anlagen, &CFG), Some(dec!(6.0)));
        assert_ne!(mindestleistung_ems(&anlagen, &CFG), Some(dec!(10.50)));
    }

    /// Two 3 kW heat pumps: neither alone is a steuVE, together they are one
    /// of 6 kW — not over 11 kW, so the flat 4,2 kW.
    #[test]
    fn two_three_kilowatt_heat_pumps_are_one_steuve_of_six() {
        let anlagen = [Anlage::new(F::Waermepumpe, dec!(3)); 2];
        let s = steuve(&anlagen);
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].netzanschlussleistung_kw(), dec!(6));
        assert_eq!(mindestleistung_ems(&anlagen, &CFG), Some(dec!(4.2)));
        assert_eq!(mindestleistung_direktansteuerung(&s[0], &CFG), dec!(4.2));
    }

    /// Grouping is per Fallgruppe and only for b and c; a Ladepunkt or a
    /// Speicher stands alone and must itself exceed 4,2 kW.
    #[test]
    fn grouping_is_per_fallgruppe_and_only_for_b_and_c() {
        let s = steuve(&[
            Anlage::new(F::Waermepumpe, dec!(3)),
            Anlage::new(F::Raumkuehlung, dec!(3)),
            Anlage::new(F::Ladepunkt, dec!(3)),
            Anlage::new(F::Ladepunkt, dec!(3)),
            Anlage::new(F::Stromspeicher, dec!(4.2)),
        ]);
        assert!(s.is_empty(), "no group and no single device exceeds 4,2 kW");
        assert_eq!(
            mindestleistung_ems(&[Anlage::new(F::Ladepunkt, dec!(4.2))], &CFG),
            None,
            "\"mehr als 4,2 kW\" is strict"
        );

        let s = steuve(&[
            Anlage::new(F::Ladepunkt, dec!(11)),
            Anlage::new(F::Waermepumpe, dec!(2.5)),
            Anlage::new(F::Raumkuehlung, dec!(4)),
            Anlage::new(F::Waermepumpe, dec!(2.5)),
            Anlage::new(F::Raumkuehlung, dec!(1)),
        ]);
        let shape: Vec<_> = s
            .iter()
            .map(|s| (s.fallgruppe(), s.netzanschlussleistung_kw()))
            .collect();
        assert_eq!(
            shape,
            vec![
                (F::Waermepumpe, dec!(5.0)),
                (F::Raumkuehlung, dec!(5)),
                (F::Ladepunkt, dec!(11)),
            ]
        );
    }

    #[test]
    fn only_heat_pumps_and_cooling_scale_above_eleven_kilowatts() {
        for gruppe in [F::Ladepunkt, F::Stromspeicher] {
            let s = steuve(&[Anlage::new(gruppe, dec!(50))]);
            assert_eq!(mindestleistung_direktansteuerung(&s[0], &CFG), dec!(4.2));
        }
        for gruppe in [F::Waermepumpe, F::Raumkuehlung] {
            let s = steuve(&[Anlage::new(gruppe, dec!(50))]);
            assert_eq!(mindestleistung_direktansteuerung(&s[0], &CFG), dec!(20.0));
            // Exactly 11 kW is not "über 11 kW".
            let at = steuve(&[Anlage::new(gruppe, dec!(11))]);
            assert_eq!(mindestleistung_direktansteuerung(&at[0], &CFG), dec!(4.2));
        }
    }

    #[test]
    fn the_flat_branch_matches_the_published_formula() {
        for (n, expected) in [
            (1usize, dec!(4.2)),
            (2, dec!(4.2) + dec!(1) * dec!(0.80) * dec!(4.2)),
            (5, dec!(4.2) + dec!(4) * dec!(0.65) * dec!(4.2)),
            (9, dec!(4.2) + dec!(8) * dec!(0.45) * dec!(4.2)),
            (12, dec!(4.2) + dec!(11) * dec!(0.45) * dec!(4.2)),
        ] {
            let anlagen = vec![Anlage::new(F::Ladepunkt, dec!(11)); n];
            assert_eq!(
                mindestleistung_ems(&anlagen, &CFG),
                Some(expected),
                "n = {n}"
            );
        }
    }

    /// `Max(0,4·ΣWP; 0,4·ΣKlima)`, not `0,4 ×` the two summed, and `n` counts
    /// every steuVE.
    #[test]
    fn the_scaled_branch_takes_the_maximum_of_the_two_group_sums() {
        let anlagen = [
            Anlage::new(F::Waermepumpe, dec!(20)),
            Anlage::new(F::Raumkuehlung, dec!(15)),
            Anlage::new(F::Ladepunkt, dec!(11)),
        ];
        // max(8, 6) + 2 × 0,75 × 4,2.
        let expected = dec!(8.0) + dec!(2) * dec!(0.75) * dec!(4.2);
        assert_eq!(mindestleistung_ems(&anlagen, &CFG), Some(expected));
    }

    /// A small heat pump beside a large one is part of the same group.
    #[test]
    fn the_group_sum_includes_every_heat_pump() {
        let anlagen = [
            Anlage::new(F::Waermepumpe, dec!(20)),
            Anlage::new(F::Waermepumpe, dec!(5)),
        ];
        assert_eq!(mindestleistung_ems(&anlagen, &CFG), Some(dec!(10.0)));
    }

    #[test]
    fn the_gleichzeitigkeitsfaktor_table_is_the_published_one() {
        for (n, gzf) in [
            (2usize, dec!(0.80)),
            (3, dec!(0.75)),
            (4, dec!(0.70)),
            (5, dec!(0.65)),
            (6, dec!(0.60)),
            (7, dec!(0.55)),
            (8, dec!(0.50)),
            (9, dec!(0.45)),
            (1_000, dec!(0.45)),
        ] {
            assert_eq!(CFG.gleichzeitigkeitsfaktor(n), Some(gzf), "n = {n}");
        }
        assert_eq!(CFG.gleichzeitigkeitsfaktor(0), None);
        assert_eq!(CFG.gleichzeitigkeitsfaktor(1), None);
    }

    /// The config holds only what Ziff. 4.5 presumes, and refuses nonsense.
    #[test]
    fn the_presumptions_can_be_replaced_within_bounds() {
        let half = CFG.skalierungsfaktor(dec!(0.5)).unwrap();
        let s = steuve(&[Anlage::new(F::Waermepumpe, dec!(20))]);
        assert_eq!(mindestleistung_direktansteuerung(&s[0], &half), dec!(10.0));
        assert!(CFG.skalierungsfaktor(dec!(0)).is_none());
        assert!(CFG.skalierungsfaktor(dec!(1.1)).is_none());
        assert!(CFG.gleichzeitigkeitsfaktoren([dec!(0.5); 8]).is_some());
        assert!(CFG.gleichzeitigkeitsfaktoren([dec!(-0.5); 8]).is_none());
    }

    #[test]
    fn the_conservative_convention_never_understates_the_share() {
        use Verursachungsregel as R;
        for (netz, steuve, uebrige) in [
            (dec!(10), dec!(6), dec!(8)),
            (dec!(3), dec!(6), dec!(2)),
            (dec!(0), dec!(6), dec!(8)),
            (dec!(14), dec!(6), dec!(8)),
        ] {
            let a = netzwirksamer_leistungsbezug(netz, steuve, None, R::SteuVeZuletzt).unwrap();
            let b = netzwirksamer_leistungsbezug(netz, steuve, Some(uebrige), R::Anteilig).unwrap();
            assert!(a >= b, "{netz}/{steuve}/{uebrige}: {a} < {b}");
            for share in [a, b] {
                assert!(share <= steuve && share <= netz.max(Decimal::ZERO));
                assert!(share >= Decimal::ZERO);
            }
        }
    }

    #[test]
    fn an_exporting_installation_draws_nothing_from_the_grid() {
        use Verursachungsregel as R;
        assert_eq!(
            netzwirksamer_leistungsbezug(dec!(-4), dec!(6), None, R::SteuVeZuletzt),
            Some(Decimal::ZERO),
        );
        assert_eq!(
            netzwirksamer_leistungsbezug(dec!(-4), dec!(6), Some(dec!(8)), R::Anteilig),
            Some(Decimal::ZERO),
        );
    }

    #[test]
    fn pro_rata_refuses_to_guess_the_rest_of_the_installation() {
        assert_eq!(
            netzwirksamer_leistungsbezug(dec!(10), dec!(6), None, Verursachungsregel::Anteilig),
            None,
        );
        assert_eq!(
            netzwirksamer_leistungsbezug(
                dec!(0),
                dec!(0),
                Some(dec!(0)),
                Verursachungsregel::Anteilig
            ),
            Some(Decimal::ZERO),
        );
    }

    /// A reduction may push the netzwirksamer Leistungsbezug down, but never
    /// below the floor.
    #[test]
    fn a_reduction_is_bounded_below_by_the_mindestleistung() {
        let anlagen = [
            Anlage::new(F::Ladepunkt, dec!(11)),
            Anlage::new(F::Waermepumpe, dec!(20)),
        ];
        let floor = mindestleistung_ems(&anlagen, &CFG).unwrap();
        // Max(0,4×20; 0) = 8, plus (2−1) × 0,80 × 4,2 = 3,36.
        assert_eq!(floor, dec!(11.360));
        let drawn = netzwirksamer_leistungsbezug(
            dec!(31),
            dec!(31),
            None,
            Verursachungsregel::SteuVeZuletzt,
        )
        .unwrap();
        assert_eq!(drawn - floor, dec!(19.640), "the abregelbare Leistung");
    }
}
