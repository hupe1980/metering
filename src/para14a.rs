//! § 14a EnWG netzorientierte Steuerung — the two powers a control decision
//! turns on.
//!
//! ## What this module is
//!
//! BNetzA **BK6-22-300** (27.11.2023, in force 01.01.2024) lets a
//! Netzbetreiber reduce the *netzwirksamer Leistungsbezug* of steuerbare
//! Verbrauchseinrichtungen in an overloaded Netzbereich, and guarantees the
//! operator a floor while it does so. Two quantities decide everything:
//!
//! | Quantity | What it is | Source |
//! |---|---|---|
//! | [`netzwirksamer_leistungsbezug`] | the share of the grid draw the steuVE cause | Anlage 1 Ziff. 2.3 |
//! | [`mindestleistung_direktansteuerung`] / [`mindestleistung_ems`] | the floor `P_min,14a` below which it may not be pushed | Anlage 1 Ziff. 4.5.1 / 4.5.2 |
//!
//! Both are powers in kW derived from nameplate figures and metered draw —
//! quantities, not money — which is why they live here rather than in a
//! billing layer. The Netzentgelt *modules* that reward participation are a
//! separate Festlegung (BK8-22/010-A); Modul 3's tariff windows are
//! [`crate::zaehlzeit`].
//!
//! ## The floor is exactly citable; the share is not
//!
//! `P_min,14a` is printed in the Festlegung as a formula with its own
//! Gleichzeitigkeitsfaktor table, and this module reproduces it verbatim.
//!
//! The *netzwirksamer Leistungsbezug* is only **defined** there — Ziff. 2.3
//! says which share it is, not how to compute it when local generation covers
//! part of the load. VDE FNN's *Bewertung der Mindestleistung* (V1.0, April
//! 2025) points on to *Netzbetrieb mit Flexibilitäten* Kap. 4.1.2 for that,
//! which is not a freely citable text. So the apportionment is a
//! [`Verursachungsregel`] the caller chooses, with each convention's
//! assumption spelled out — the same treatment the crate gives G 685's final
//! rounding and VDE-AR-N 4400's thresholds. What this module guarantees is the
//! arithmetic, not conformance with a document it cannot quote.
//!
//! ## A missing sub-measurement
//!
//! Ziff. 4.7 makes a separate Zählpunkt for the steuVE optional, so a
//! sub-measurement often does not exist. The conservative substitute is the
//! device's **Netzanschlussleistung** — assume it draws its rated power — which
//! can only overstate the steuVE share and so can only make a guard fire
//! early. That is a caller's choice and this module does not make it: pass
//! `measured.unwrap_or(nennleistung)` and the convention is visible at the call
//! site rather than buried.

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
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "SCREAMING_SNAKE_CASE"))]
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

    /// Stable DB/wire label. Matches the `serde` tag and
    /// [`FromStr`](std::str::FromStr) input.
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

    /// `true` for the two Fallgruppen the > 11 kW scaling rule applies to.
    ///
    /// Ziff. 4.5.1 Satz 2 and Ziff. 4.5.2 Satz 3 both name *"Ziffern 2.4.1.b
    /// sowie 2.4.1.c"* — Wärmepumpe and Raumkühlung — and no others. A
    /// Ladepunkt or a Stromspeicher keeps the flat floor however large it is.
    #[must_use]
    pub const fn is_scaled_above_threshold(self) -> bool {
        matches!(self, Self::Waermepumpe | Self::Raumkuehlung)
    }
}

crate::codes::string_codes! {
    SteuVeFallgruppe;
}

/// One steuerbare Verbrauchseinrichtung, as § 14a counts it.
///
/// Where several Anlagen of Fallgruppe **b** or **c** sit behind one
/// Netzanschluss, Ziff. 2.4.2 treats them as **one** steuVE whose
/// Netzanschlussleistung is their sum. Group them before building this, so
/// `netzanschlussleistung_kw` is already the grouped figure and
/// [`mindestleistung_ems`]'s count of `n_steuVE` is the count the Festlegung
/// means.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct SteuVe {
    /// Which of the four Fallgruppen this is.
    pub fallgruppe: SteuVeFallgruppe,
    /// Netzanschlussleistung in kW — grouped per Ziff. 2.4.2 where that
    /// applies.
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::decimal"))]
    pub netzanschlussleistung_kw: Decimal,
}

impl SteuVe {
    /// A steuVE of `fallgruppe` with `netzanschlussleistung_kw` kW.
    #[must_use]
    pub const fn new(fallgruppe: SteuVeFallgruppe, netzanschlussleistung_kw: Decimal) -> Self {
        Self {
            fallgruppe,
            netzanschlussleistung_kw,
        }
    }

    /// `true` when this is a steuerbare Verbrauchseinrichtung at all.
    ///
    /// Ziff. 2.4.1 admits the four Fallgruppen only *"mit einer
    /// Netzanschlussleistung von mehr als 4,2 Kilowatt (kW)"*, so a smaller
    /// device is not a steuVE, is not counted by `n_steuVE`, and has no
    /// Mindestleistung. The bound is strict: exactly 4,2 kW is not *more than*
    /// 4,2 kW.
    ///
    /// Where Ziff. 2.4.2 groups several Anlagen of Fallgruppe b or c behind one
    /// Netzanschluss, it is the **group's** sum that has to clear the
    /// threshold — so group first, and this reads the grouped figure.
    #[must_use]
    pub fn is_steuerbar(&self) -> bool {
        self.netzanschlussleistung_kw > STEUVE_SCHWELLE_KW
    }
}

// ── the published parameters ─────────────────────────────────────────────────

/// The flat Mindestleistung of Ziff. 4.5.1 Satz 1: **4,2 kW** per steuVE.
///
/// Numerically the same as [`STEUVE_SCHWELLE_KW`], and a different provision:
/// one is the power a device must *exceed* to be a steuVE, the other the power
/// it must be left. They coincide, which is why they are named separately —
/// an *anderweitige Empfehlung* could move the second without touching the
/// first.
pub const MINDESTLEISTUNG_KW: Decimal = Decimal::from_parts(42, 0, 0, false, 1);

/// The Netzanschlussleistung a device must **exceed** to be a steuerbare
/// Verbrauchseinrichtung at all: **4,2 kW** (Ziff. 2.4.1).
///
/// See [`SteuVe::is_steuerbar`], and [`MINDESTLEISTUNG_KW`] for the other
/// 4,2 kW.
pub const STEUVE_SCHWELLE_KW: Decimal = Decimal::from_parts(42, 0, 0, false, 1);

/// The Netzanschlussleistung above which Wärmepumpe and Raumkühlung scale
/// instead: **11 kW** (Ziff. 4.5.1 Satz 2, Ziff. 4.5.2 Satz 3).
pub const SKALIERUNG_SCHWELLE_KW: Decimal = Decimal::from_parts(11, 0, 0, false, 0);

/// The Skalierungsfaktor presumed appropriate: **0,4**.
///
/// Ziff. 4.5.1 Satz 3: *"Bis zum Inkrafttreten einer anderweitigen Empfehlung
/// wird die Angemessenheit vermutet, wenn der Skalierungsfaktor 0,4 beträgt."*
/// A presumption, not a constant — which is why [`Para14aConfig`] carries it.
pub const SKALIERUNGSFAKTOR: Decimal = Decimal::from_parts(4, 0, 0, false, 1);

/// The Gleichzeitigkeitsfaktoren of Ziff. 4.5.2, for `n_steuVE` = 2, 3, … ≥ 9.
///
/// Printed in the Festlegung as a table:
///
/// | `n_steuVE` | 2 | 3 | 4 | 5 | 6 | 7 | 8 | ≥ 9 |
/// |---|---|---|---|---|---|---|---|---|
/// | GZF | 0,8 | 0,75 | 0,7 | 0,65 | 0,6 | 0,55 | 0,5 | 0,45 |
///
/// The whole of Ziff. 4.5.2 is a presumption — *"Bis zum Inkrafttreten einer
/// anderweitigen Empfehlung wird die Angemessenheit vermutet, wenn die
/// Berechnung wie nachstehend erfolgt"* — so an operator holding a newer
/// Empfehlung computes with its own table and the parts of the formula this
/// module exposes.
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

/// The Gleichzeitigkeitsfaktor for `n` EMS-controlled steuVE.
///
/// `None` for `n < 2`: the table starts at two, and at `n = 1` the term it
/// multiplies is `(1 − 1) = 0`, so no factor is needed. Everything from nine
/// upwards is 0,45.
///
/// ```rust
/// use metering::para14a::gleichzeitigkeitsfaktor;
/// use rust_decimal::dec;
///
/// assert_eq!(gleichzeitigkeitsfaktor(2), Some(dec!(0.80)));
/// assert_eq!(gleichzeitigkeitsfaktor(8), Some(dec!(0.50)));
/// assert_eq!(gleichzeitigkeitsfaktor(9),  gleichzeitigkeitsfaktor(400));
/// assert_eq!(gleichzeitigkeitsfaktor(1), None, "the table starts at two");
/// ```
#[must_use]
pub fn gleichzeitigkeitsfaktor(n_steuve: u32) -> Option<Decimal> {
    if n_steuve < 2 {
        return None;
    }
    let index = (n_steuve as usize - 2).min(GLEICHZEITIGKEITSFAKTOREN.len() - 1);
    GLEICHZEITIGKEITSFAKTOREN.get(index).copied()
}

/// The parameters of Ziff. 4.5 that an *anderweitige Empfehlung* may replace.
///
/// [`Default`] is the Festlegung's own presumption. Every field is a number
/// the text either fixes (4,2 kW, 11 kW) or presumes appropriate until
/// superseded (0,4); carrying them here rather than inlining them keeps the
/// arithmetic usable the day one of them moves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct Para14aConfig {
    /// The flat floor per steuVE, kW. Ziff. 4.5.1 Satz 1: 4,2.
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::decimal"))]
    pub mindestleistung_kw: Decimal,
    /// The Netzanschlussleistung above which b/c scale instead, kW: 11.
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::decimal"))]
    pub skalierung_schwelle_kw: Decimal,
    /// The Skalierungsfaktor: 0,4.
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::decimal"))]
    pub skalierungsfaktor: Decimal,
}

impl Default for Para14aConfig {
    fn default() -> Self {
        Self {
            mindestleistung_kw: MINDESTLEISTUNG_KW,
            skalierung_schwelle_kw: SKALIERUNG_SCHWELLE_KW,
            skalierungsfaktor: SKALIERUNGSFAKTOR,
        }
    }
}

impl Para14aConfig {
    /// `true` when `device` falls under the scaling rule — Fallgruppe b or c,
    /// **and** a Netzanschlussleistung above the threshold.
    #[must_use]
    fn is_scaled(&self, device: &SteuVe) -> bool {
        device.fallgruppe.is_scaled_above_threshold()
            && device.netzanschlussleistung_kw > self.skalierung_schwelle_kw
    }
}

// ── Ziff. 4.5.1 — Direktansteuerung ──────────────────────────────────────────

/// `P_min,14a` for **one** directly-controlled steuVE (Ziff. 4.4.a).
///
/// Ziff. 4.5.1, verbatim:
///
/// > ¹Für jede steuerbare Verbrauchseinrichtung im Sinne der Ziffer 2.4.1., die
/// > gemäß Ziffer 4.4.a. (Direktansteuerung) angesteuert wird, beträgt die
/// > Mindestleistung 4,2 kW. ²Abweichend vom vorstehenden Satz ergibt sich die
/// > Mindestleistung für jede steuerbare Verbrauchseinrichtung im Sinne der
/// > Ziffern 2.4.1.b. sowie 2.4.1.c. \[…\], die gemäß Ziffer 4.4.a.
/// > (Direktansteuerung) angesteuert wird und eine Netzanschlussleistung über
/// > 11 kW aufweist, aus der Multiplikation der Netzanschlussleistung der
/// > steuerbaren Verbrauchseinrichtung mit einem angemessenen
/// > Skalierungsfaktor.
///
/// Note where the scaling *does not* apply: a Ladepunkt or a Stromspeicher
/// keeps 4,2 kW however large it is, and a 20 kW Wärmepumpe scales while a
/// 20 kW Ladepunkt does not.
///
/// `None` when `device` is not a steuVE — see [`SteuVe::is_steuerbar`]. A
/// device at or below 4,2 kW has no Mindestleistung because it has no
/// Teilnahmeverpflichtung; returning the flat 4,2 kW for it would answer a
/// question the Festlegung does not ask.
///
/// ```rust
/// use metering::para14a::{Para14aConfig, SteuVe, SteuVeFallgruppe, mindestleistung_direktansteuerung};
/// use rust_decimal::dec;
///
/// let cfg = Para14aConfig::default();
///
/// // An ordinary wallbox: the flat floor.
/// let wallbox = SteuVe::new(SteuVeFallgruppe::Ladepunkt, dec!(22));
/// assert_eq!(mindestleistung_direktansteuerung(&wallbox, &cfg), Some(dec!(4.2)));
///
/// // A 20 kW heat pump scales instead: 0,4 × 20 = 8 kW.
/// let wp = SteuVe::new(SteuVeFallgruppe::Waermepumpe, dec!(20));
/// assert_eq!(mindestleistung_direktansteuerung(&wp, &cfg), Some(dec!(8.0)));
///
/// // …but only above 11 kW.
/// let small = SteuVe::new(SteuVeFallgruppe::Waermepumpe, dec!(9));
/// assert_eq!(mindestleistung_direktansteuerung(&small, &cfg), Some(dec!(4.2)));
///
/// // …and a 3 kW device is not a steuVE at all.
/// let tiny = SteuVe::new(SteuVeFallgruppe::Waermepumpe, dec!(3));
/// assert_eq!(mindestleistung_direktansteuerung(&tiny, &cfg), None);
/// ```
#[must_use]
pub fn mindestleistung_direktansteuerung(
    device: &SteuVe,
    config: &Para14aConfig,
) -> Option<Decimal> {
    if !device.is_steuerbar() {
        return None;
    }
    Some(if config.is_scaled(device) {
        device.netzanschlussleistung_kw * config.skalierungsfaktor
    } else {
        config.mindestleistung_kw
    })
}

// ── Ziff. 4.5.2 — Steuerung mittels EMS ──────────────────────────────────────

/// `P_min,14a` for a set of steuVE controlled through **one** EMS
/// (Ziff. 4.4.b).
///
/// Ziff. 4.5.2, verbatim:
///
/// > ³Sofern Anlagen im Sinne der Ziffern 2.4.1.b sowie 2.4.1.c (jeweils
/// > i.V.m. Ziffer 2.4.2), mit einer Netzanschlussleistung über 11 kW
/// > Bestandteil der Steuerung nach Ziffer 4.4.b sind, gilt:
/// >
/// > `P_min,14a = Max(0,4 x P_Summe WP; 0,4 x P_Summe Klima) + (n_steuVE − 1) x GZF x 4,2 kW`
/// >
/// > ⁴Ansonsten gilt:
/// >
/// > `P_min,14a = 4,2 kW + (n_steuVE − 1) x GZF x 4,2 kW`
///
/// Two traps. The first term of the upper branch is a **maximum of two group
/// sums**, not `0,4 ×` everything: `P_Summe WP` sums Fallgruppe b, `P_Summe
/// Klima` sums Fallgruppe c, and the larger scaled sum wins. And `n_steuVE` is
/// *"Anzahl aller steuerbarer Verbrauchseinrichtungen, die nach Ziffer 4.4.b
/// angesteuert werden"* — **all** of them, not only the scaled ones. Either
/// mistake overstates the floor, which denies the Netzbetreiber reduction
/// headroom it is entitled to.
///
/// `None` for an empty set, and `None` when any entry is not a steuVE:
/// Ziff. 2.4.1 admits only devices above 4,2 kW, and a smaller one would
/// inflate `n_steuVE` and the floor with it. Filter with
/// [`SteuVe::is_steuerbar`].
///
/// ```rust
/// use metering::para14a::{Para14aConfig, SteuVe, SteuVeFallgruppe as F, mindestleistung_ems};
/// use rust_decimal::dec;
///
/// let cfg = Para14aConfig::default();
///
/// // Three ordinary steuVE: 4,2 + (3−1) × 0,75 × 4,2 = 10,5 kW.
/// let plain = [
///     SteuVe::new(F::Ladepunkt, dec!(11)),
///     SteuVe::new(F::Waermepumpe, dec!(9)),
///     SteuVe::new(F::Stromspeicher, dec!(10)),
/// ];
/// assert_eq!(mindestleistung_ems(&plain, &cfg), Some(dec!(10.500)));
///
/// // Swap in a 20 kW heat pump: Max(0,4×20; 0,4×0) = 8 replaces the 4,2.
/// let scaled = [
///     SteuVe::new(F::Ladepunkt, dec!(11)),
///     SteuVe::new(F::Waermepumpe, dec!(20)),
///     SteuVe::new(F::Stromspeicher, dec!(10)),
/// ];
/// assert_eq!(mindestleistung_ems(&scaled, &cfg), Some(dec!(14.300)));
///
/// assert_eq!(mindestleistung_ems(&[], &cfg), None);
/// ```
#[must_use]
pub fn mindestleistung_ems(devices: &[SteuVe], config: &Para14aConfig) -> Option<Decimal> {
    let n = u32::try_from(devices.len()).ok()?;
    if n == 0 || !devices.iter().all(SteuVe::is_steuerbar) {
        return None;
    }

    let sum_of = |gruppe: SteuVeFallgruppe| -> Decimal {
        devices
            .iter()
            .filter(|d| d.fallgruppe == gruppe)
            .map(|d| d.netzanschlussleistung_kw)
            .sum()
    };

    // The branch condition is the *presence* of a scaled b/c Anlage, not the
    // size of the sums: a set of three 5 kW heat pumps that were not grouped
    // under Ziff. 2.4.2 sums to 15 kW and still takes the flat branch.
    let base = if devices.iter().any(|d| config.is_scaled(d)) {
        let wp = config.skalierungsfaktor * sum_of(SteuVeFallgruppe::Waermepumpe);
        let klima = config.skalierungsfaktor * sum_of(SteuVeFallgruppe::Raumkuehlung);
        wp.max(klima)
    } else {
        config.mindestleistung_kw
    };

    let rest = match gleichzeitigkeitsfaktor(n) {
        Some(gzf) => Decimal::from(n - 1) * gzf * config.mindestleistung_kw,
        // n == 1: the term is zero and the table has no entry for it.
        None => Decimal::ZERO,
    };
    Some(base + rest)
}

// ── Ziff. 2.3 — netzwirksamer Leistungsbezug ─────────────────────────────────

/// Which share of the grid draw the steuVE are taken to have caused.
///
/// Anlage 1 Ziff. 2.3 defines the netzwirksamer Leistungsbezug as *"derjenige
/// Anteil der über den Netzanschluss aus einem Elektrizitätsverteilernetz der
/// allgemeinen Versorgung entnommenen elektrischen Leistung, der zeitgleich
/// durch eine oder mehrere steuerbare Verbrauchseinrichtungen verursacht
/// wird"* — and stops
/// there. When local generation covers part of the load, *which* part of the
/// remaining grid draw the steuVE caused is an apportionment the Festlegung
/// does not perform, so it is named here rather than assumed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "SCREAMING_SNAKE_CASE"))]
pub enum Verursachungsregel {
    /// Local generation serves the **uncontrollable** load first, so whatever
    /// grid draw is left is the steuVE's: `min(Netzbezug, P_steuVE)`.
    ///
    /// The conservative reading, and the default. It never understates the
    /// steuVE share, so a guard built on it fires early rather than late — the
    /// safe direction when the consequence of being wrong is an overloaded
    /// Betriebsmittel. Needs no figure for the rest of the installation.
    ///
    /// The `serde` tag is spelled out: `SCREAMING_SNAKE_CASE` would split the
    /// market's own abbreviation into `STEU_VE_ZULETZT`, and the contract here
    /// is that the tag and `as_str` are one string.
    #[cfg_attr(feature = "serde", serde(rename = "STEUVE_ZULETZT"))]
    #[default]
    SteuVeZuletzt,

    /// Generation is shared pro rata, so the steuVE cause their share of the
    /// total draw: `Netzbezug × P_steuVE ÷ (P_steuVE + P_übrige)`.
    ///
    /// Needs the rest of the installation's draw, and reports `None` without
    /// it. Lower than [`SteuVeZuletzt`](Self::SteuVeZuletzt) whenever local
    /// generation is running.
    Anteilig,
}

impl Verursachungsregel {
    /// Every convention, in declaration order.
    pub const ALL: [Self; 2] = [Self::SteuVeZuletzt, Self::Anteilig];

    /// Stable DB/wire label. Matches the `serde` tag and
    /// [`FromStr`](std::str::FromStr) input.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::SteuVeZuletzt => "STEUVE_ZULETZT",
            Self::Anteilig => "ANTEILIG",
        }
    }
}

crate::codes::string_codes! {
    Verursachungsregel;
}

/// The netzwirksamer Leistungsbezug in kW — the share of the grid draw caused
/// by the steuerbare Verbrauchseinrichtungen (Anlage 1 Ziff. 2.3).
///
/// `netzbezug_kw` is the power drawn **from** the grid over the Netzanschluss.
/// An installation that is exporting draws nothing, so a negative value is
/// read as zero rather than as a negative share.
///
/// `uebrige_last_kw` is everything else behind the Netzanschluss. Only
/// [`Verursachungsregel::Anteilig`] needs it; supplying `None` to that
/// convention returns `None` rather than quietly falling back to the other
/// one, because the two answer different questions and a silent substitution
/// would put the wrong number in front of a control decision.
///
/// ## Worked
///
/// A house drawing 10 kW from the grid, of which a wallbox accounts for 6 kW
/// and the rest of the house 8 kW, with 4 kW of PV running:
///
/// - `SteuVeZuletzt` — the PV covers the house first, so all 6 kW of the
///   wallbox is still grid draw: `min(10, 6) = 6 kW`.
/// - `Anteilig` — the wallbox is 6 of 14 kW of load, so it carries
///   `10 × 6/14 ≈ 4,29 kW`.
///
/// Both are defensible readings of *"zeitgleich verursacht"*; the Festlegung
/// picks neither, so this function does not either.
///
/// ```rust
/// use metering::para14a::{Verursachungsregel, netzwirksamer_leistungsbezug};
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
/// // The pro-rata convention will not guess the rest of the installation.
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
            let gesamt = steuve + uebrige;
            if gesamt.is_zero() {
                // No load at all draws no power, whatever the meter says.
                return Some(Decimal::ZERO);
            }
            Some((netzbezug * steuve / gesamt).min(steuve))
        }
    }
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use SteuVeFallgruppe as F;
    use rust_decimal::dec;

    fn cfg() -> Para14aConfig {
        Para14aConfig::default()
    }

    // ── Ziff. 4.5.1 ──────────────────────────────────────────────────────────

    /// The scaling rule names Fallgruppen b and c and no others, so a large
    /// Ladepunkt and a large Speicher keep the flat 4,2 kW.
    #[test]
    fn only_heat_pumps_and_cooling_scale_above_eleven_kilowatts() {
        for gruppe in [F::Ladepunkt, F::Stromspeicher] {
            let big = SteuVe::new(gruppe, dec!(50));
            assert_eq!(
                mindestleistung_direktansteuerung(&big, &cfg()),
                Some(dec!(4.2)),
                "{gruppe} must not scale"
            );
        }
        for gruppe in [F::Waermepumpe, F::Raumkuehlung] {
            let big = SteuVe::new(gruppe, dec!(50));
            assert_eq!(
                mindestleistung_direktansteuerung(&big, &cfg()),
                Some(dec!(20.0))
            );
            // The threshold is strict: exactly 11 kW is not "über 11 kW".
            let at = SteuVe::new(gruppe, dec!(11));
            assert_eq!(
                mindestleistung_direktansteuerung(&at, &cfg()),
                Some(dec!(4.2))
            );
        }
    }

    /// Ziff. 2.4.1 admits only devices *"mit einer Netzanschlussleistung von
    /// mehr als 4,2 Kilowatt"*. A smaller one is not a steuVE, so it has no
    /// Mindestleistung — and, more consequentially, it must not be counted by
    /// `n_steuVE`, where its presence would raise the floor for every other
    /// device in the set and quietly cost the Netzbetreiber headroom.
    #[test]
    fn a_device_below_the_threshold_is_not_a_steuve() {
        let tiny = SteuVe::new(F::Waermepumpe, dec!(3));
        let boundary = SteuVe::new(F::Ladepunkt, dec!(4.2));
        let real = SteuVe::new(F::Ladepunkt, dec!(11));

        assert!(!tiny.is_steuerbar());
        assert!(!boundary.is_steuerbar(), "\"mehr als 4,2 kW\" is strict");
        assert!(real.is_steuerbar());

        assert_eq!(mindestleistung_direktansteuerung(&tiny, &cfg()), None);
        assert_eq!(mindestleistung_direktansteuerung(&boundary, &cfg()), None);

        // One stray entry refuses the whole set rather than inflating it.
        assert_eq!(mindestleistung_ems(&[real, tiny], &cfg()), None);
        assert_eq!(
            mindestleistung_ems(&[real], &cfg()),
            Some(dec!(4.2)),
            "the qualifying device alone still answers",
        );

        // The wrong answer this refuses: counting it would have given
        // 4,2 + (2−1) × 0,80 × 4,2 = 7,56 kW instead of 4,2 kW.
        assert_ne!(mindestleistung_ems(&[real, tiny], &cfg()), Some(dec!(7.56)));
    }

    // ── Ziff. 4.5.2 ──────────────────────────────────────────────────────────

    #[test]
    fn the_flat_branch_matches_the_published_formula() {
        // 4,2 + (n−1) × GZF × 4,2 for a set with nothing over 11 kW.
        let cases = [
            (1usize, dec!(4.2)),
            (2, dec!(4.2) + dec!(1) * dec!(0.80) * dec!(4.2)),
            (5, dec!(4.2) + dec!(4) * dec!(0.65) * dec!(4.2)),
            (9, dec!(4.2) + dec!(8) * dec!(0.45) * dec!(4.2)),
            (12, dec!(4.2) + dec!(11) * dec!(0.45) * dec!(4.2)),
        ];
        for (n, expected) in cases {
            let devices = vec![SteuVe::new(F::Ladepunkt, dec!(11)); n];
            assert_eq!(
                mindestleistung_ems(&devices, &cfg()),
                Some(expected),
                "n = {n}"
            );
        }
    }

    /// The upper branch is `Max(0,4·ΣWP; 0,4·ΣKlima)`, **not** `0,4 ×` the two
    /// summed. Adding them overstates the floor on any installation carrying
    /// both, which denies the Netzbetreiber headroom it is entitled to.
    #[test]
    fn the_scaled_branch_takes_the_maximum_of_the_two_group_sums() {
        let devices = [
            SteuVe::new(F::Waermepumpe, dec!(20)),  // ΣWP    = 20 → 8
            SteuVe::new(F::Raumkuehlung, dec!(15)), // ΣKlima = 15 → 6
        ];
        let gzf = dec!(0.80);
        let expected = dec!(8.0) + dec!(1) * gzf * dec!(4.2);
        assert_eq!(mindestleistung_ems(&devices, &cfg()), Some(expected));

        // Summing the groups instead would give 0,4 × 35 = 14, not 8.
        let wrong = dec!(14.0) + dec!(1) * gzf * dec!(4.2);
        assert_ne!(mindestleistung_ems(&devices, &cfg()), Some(wrong));
    }

    /// The group sums run over *all* Anlagen of the Fallgruppe, while the
    /// branch is chosen by whether one of them is over the threshold.
    #[test]
    fn the_sums_cover_the_whole_group_but_the_branch_needs_one_over_the_threshold() {
        // Three 5 kW heat pumps, not grouped under Ziff. 2.4.2: none is over
        // 11 kW, so the flat branch applies even though they sum to 15.
        let ungrouped = vec![SteuVe::new(F::Waermepumpe, dec!(5)); 3];
        let flat = dec!(4.2) + dec!(2) * dec!(0.75) * dec!(4.2);
        assert_eq!(mindestleistung_ems(&ungrouped, &cfg()), Some(flat));

        // Grouped per Ziff. 2.4.2 they are one steuVE of 15 kW, which is.
        let grouped = [SteuVe::new(F::Waermepumpe, dec!(15))];
        assert_eq!(mindestleistung_ems(&grouped, &cfg()), Some(dec!(6.0)));

        // A small heat pump beside a large one still counts into ΣWP.
        let mixed = [
            SteuVe::new(F::Waermepumpe, dec!(20)),
            SteuVe::new(F::Waermepumpe, dec!(5)),
        ];
        let expected = dec!(0.4) * dec!(25) + dec!(1) * dec!(0.80) * dec!(4.2);
        assert_eq!(mindestleistung_ems(&mixed, &cfg()), Some(expected));
    }

    #[test]
    fn an_empty_set_has_no_floor() {
        assert_eq!(mindestleistung_ems(&[], &cfg()), None);
    }

    #[test]
    fn the_gleichzeitigkeitsfaktor_table_is_the_published_one() {
        let published = [
            (2u32, dec!(0.80)),
            (3, dec!(0.75)),
            (4, dec!(0.70)),
            (5, dec!(0.65)),
            (6, dec!(0.60)),
            (7, dec!(0.55)),
            (8, dec!(0.50)),
            (9, dec!(0.45)),
        ];
        for (n, gzf) in published {
            assert_eq!(gleichzeitigkeitsfaktor(n), Some(gzf), "n = {n}");
        }
        // ">= 9" is a floor, not a row.
        for n in [10u32, 25, 1_000] {
            assert_eq!(gleichzeitigkeitsfaktor(n), Some(dec!(0.45)), "n = {n}");
        }
        assert_eq!(gleichzeitigkeitsfaktor(0), None);
        assert_eq!(gleichzeitigkeitsfaktor(1), None);
    }

    /// An EMS floor is never below the single-device floor it generalises.
    #[test]
    fn the_ems_floor_is_monotone_in_the_device_count() {
        let mut previous = Decimal::ZERO;
        for n in 1..=12usize {
            let devices = vec![SteuVe::new(F::Ladepunkt, dec!(11)); n];
            let floor = mindestleistung_ems(&devices, &cfg()).expect("non-empty");
            assert!(floor >= previous, "n = {n}: {floor} < {previous}");
            assert!(floor >= dec!(4.2), "n = {n}: below the single-device floor");
            previous = floor;
        }
    }

    // ── Ziff. 2.3 ────────────────────────────────────────────────────────────

    #[test]
    fn the_conservative_convention_never_understates_the_share() {
        use Verursachungsregel as R;
        let cases = [
            // (Netzbezug, steuVE, übrige)
            (dec!(10), dec!(6), dec!(8)),
            (dec!(3), dec!(6), dec!(2)),
            (dec!(0), dec!(6), dec!(8)),
            (dec!(14), dec!(6), dec!(8)),
        ];
        for (netz, steuve, uebrige) in cases {
            let a = netzwirksamer_leistungsbezug(netz, steuve, None, R::SteuVeZuletzt).unwrap();
            let b = netzwirksamer_leistungsbezug(netz, steuve, Some(uebrige), R::Anteilig).unwrap();
            assert!(a >= b, "{netz}/{steuve}/{uebrige}: {a} < {b}");
            // Neither convention attributes more than the steuVE actually drew,
            // nor more than came out of the grid.
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
        // A dead installation is zero, not a division by zero.
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

    /// The whole point of the pairing: a reduction may push the netzwirksamer
    /// Leistungsbezug down, but never below the floor.
    #[test]
    fn a_reduction_is_bounded_below_by_the_mindestleistung() {
        let devices = [
            SteuVe::new(F::Ladepunkt, dec!(11)),
            SteuVe::new(F::Waermepumpe, dec!(20)),
        ];
        let floor = mindestleistung_ems(&devices, &cfg()).unwrap();
        // Max(0,4×20; 0) = 8, plus (2−1) × 0,80 × 4,2 = 3,36.
        assert_eq!(floor, dec!(11.360));

        let drawn = netzwirksamer_leistungsbezug(
            dec!(31),
            dec!(31),
            None,
            Verursachungsregel::SteuVeZuletzt,
        )
        .unwrap();
        assert!(drawn > floor, "there is headroom to reduce");
        assert_eq!(drawn - floor, dec!(19.640), "the abregelbare Leistung");
    }
}
