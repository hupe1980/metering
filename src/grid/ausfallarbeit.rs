//! Ausfallarbeit — the energy a Redispatch measure prevented, or forced.
//!
//! ## Legal basis
//!
//! - **§ 13a Abs. 1a EnWG** — the bilanzieller Ausgleich owed to the
//!   Bilanzkreisverantwortlicher; **§ 13a Abs. 2** the financial one owed to the
//!   Anlagenbetreiber.
//! - **BNetzA BK6-23-241** (Beschluss 07.05.2026), Anlage
//!   *"Bilanzieller Ausgleich von Redispatch-Maßnahmen (BilAReM)"*, Kapitel 3;
//!   exchange processes apply from 01.10.2026.
//! - **BDEW-Leitfaden zur Berechnung der Ausfallarbeit Redispatch 2.0**
//!   (Mai 2020).
//!
//! ## The quantity
//!
//! [BilAReM Kap. 3]: *"Ausfallarbeit ist – arbeitsbezogen – die Differenz
//! zwischen der theoretischen Erzeugung einer TR und dem Wert der
//! Leistungslimitierung"*, and *"bei negativem Redispatch ist die Ausfallarbeit positiv,
//! bei positivem Redispatch ist die Ausfallarbeit negativ (Mehrarbeit)"*.
//! So it is **signed**; each formula clamps on the side its direction allows.
//!
//! Everything is per **Viertelstunde** and per **Technische Ressource**:
//! *"Die Ausfallarbeit wird für jede TR bestimmt"*, and *"Soweit in diesem
//! Kapitel Leistungswerte genannt werden, sind Viertelstundenmittelwerte
//! gemeint"*.
//!
//! ## Start here
//!
//! | Variante | Datengrundlage | Here |
//! |---|---|---|
//! | Spitzabrechnung (fluktuierend) | *"gemessene Wetterdaten der TR"* | [`spitz_fluktuierend_kwh`], theoretical power **supplied** |
//! | vereinfachte Spitzabrechnung | *"mit Referenzmesswerten oder Wetterdaten für den Standort"* | the same formula, other input data |
//! | Spitzabrechnung (nicht-fluktuierend) | Ex-ante-Planungsdaten | [`spitz_nicht_fluktuierend_kwh`] |
//! | Pauschal-Abrechnung | the last quarter-hour, or an Anlagenfaktor | [`pauschal_wind_kwh`], [`pauschal_solar_kwh`], [`pauschal_nicht_fluktuierend_kwh`] |
//!
//! [`leistungslimitierung_kw`] gives `P_lim,i`, [`p0_kw`] gives `P_0`. The
//! theoretical power from wind speed or irradiance (power curve, site model) is
//! the caller's input; the Wind-Bin-Verfahren for Windenergieanlagen auf See is
//! not covered. The Pauschal-Abrechnung ends with [`PAUSCHAL_BESTANDSSCHUTZ_ENDE`].

use rust_decimal::{Decimal, dec};
use time::{Date, Month, OffsetDateTime, macros::offset};

use crate::precision::{AUSFALLARBEIT_DP, AUSFALLARBEIT_STRATEGY};
use crate::series::interval::{MeterInterval, QualityFlag};

// ── direction and case ────────────────────────────────────────────────────────

/// Which way a Redispatch measure moved the plant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Redispatchrichtung {
    /// The plant was told to produce **more**. The Ausfallarbeit is negative —
    /// *Mehrarbeit*.
    Positiv,
    /// The plant was told to produce **less**. The Ausfallarbeit is positive.
    Negativ,
}

impl Redispatchrichtung {
    /// Both directions, in declaration order.
    pub const ALL: [Self; 2] = [Self::Positiv, Self::Negativ];

    /// Stable DB/wire label; the `serde` tag and [`FromStr`](std::str::FromStr) input.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Positiv => "POSITIV",
            Self::Negativ => "NEGATIV",
        }
    }
}

/// Whether the plant acted on the instruction or the grid operator did.
///
/// [BilAReM Kap. 3.1]. In the Aufforderungsfall *"trägt im Aufforderungsfall der BKV des LF das Risiko,
/// dass die Redispatch-Anweisung korrekt umgesetzt wird"*; in the Duldungsfall
/// what actually happened **is** the limit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Redispatchfall {
    /// The Anlagenbetreiber was instructed and implemented it themselves.
    Aufforderung,
    /// The Netzbetreiber intervened; the plant tolerated it.
    Duldung,
}

impl Redispatchfall {
    /// Both cases, in declaration order.
    pub const ALL: [Self; 2] = [Self::Aufforderung, Self::Duldung];

    /// Stable DB/wire label; the `serde` tag and [`FromStr`](std::str::FromStr) input.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Aufforderung => "AUFFORDERUNG",
            Self::Duldung => "DULDUNG",
        }
    }
}

// ── Abrechnungsvariante ───────────────────────────────────────────────────────

/// How the theoretical generation is determined.
///
/// [BilAReM Kap. 3.2.1] (fluktuierend); Kap. 3.3 (nicht-fluktuierend) has
/// only Spitz- and Pauschal-Abrechnung.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Abrechnungsvariante {
    /// *"gemessene Wetterdaten der TR"* — or, for a non-fluctuating plant,
    /// Ex-ante-Planungsdaten.
    Spitzabrechnung,
    /// *"mit Referenzmesswerten oder Wetterdaten für den Standort"*.
    ///
    /// The default: *"Trifft der Anlagenbetreiber keine Zuordnungsentscheidung,
    /// findet die vereinfachte Spitzabrechnung Anwendung."* Not available for a
    /// non-fluctuating plant.
    VereinfachteSpitzabrechnung,
    /// *"Fortschreiben der letzten Viertelstunde vor der Redispatch-Maßnahme
    /// oder das Produkt aus Anlagenfaktor und installierter Nennleistung"*.
    ///
    /// Closed to new Technische Ressourcen; ends with
    /// [`PAUSCHAL_BESTANDSSCHUTZ_ENDE`].
    PauschalAbrechnung,
}

impl Abrechnungsvariante {
    /// Every variant, in declaration order.
    pub const ALL: [Self; 3] = [
        Self::Spitzabrechnung,
        Self::VereinfachteSpitzabrechnung,
        Self::PauschalAbrechnung,
    ];

    /// Stable DB/wire label; the `serde` tag and [`FromStr`](std::str::FromStr) input.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Spitzabrechnung => "SPITZABRECHNUNG",
            Self::VereinfachteSpitzabrechnung => "VEREINFACHTE_SPITZABRECHNUNG",
            Self::PauschalAbrechnung => "PAUSCHAL_ABRECHNUNG",
        }
    }
}

crate::ids::codes::string_codes! {
    Redispatchrichtung;
    Redispatchfall;
    Abrechnungsvariante;
}

/// The last day a Technische Ressource may stay in the Pauschal-Abrechnung:
/// **31.12.2028**.
///
/// [BK6-23-241, Kap. 3.2.1 der BilAReM]: such a TR may remain
/// *"bis zum 31.12.2028 in der Pauschal-Abrechnung"*, and *"Ab dem 01.01.2029
/// werden diese TR der vereinfachten Spitzabrechnung zugeordnet, wenn nicht der
/// Anlagenbetreiber spätestens bis zum 30.11.2028 die Spitzabrechnung
/// festlegt."*
pub const PAUSCHAL_BESTANDSSCHUTZ_ENDE: Date =
    match Date::from_calendar_date(2028, Month::December, 31) {
        Ok(d) => d,
        Err(_) => unreachable!(),
    };

// ── Kapitel 3.1 — der Wert der Leistungslimitierung ──────────────────────────

/// The Wert der Leistungslimitierung `P_lim,i`, in kW — [BilAReM Kap. 3.1].
///
/// *"Der Wert der Leistungslimitierung beschreibt die Leistung, die aufgrund
/// des Redispatch-Abrufs gefahren wurde."*
///
/// | Fall | Richtung | `P_lim,i` |
/// |---|---|---|
/// | Aufforderung | positiv | `min{P_ist,i ; P_min,i}` |
/// | Aufforderung | negativ | `max{P_ist,i ; P_max,i}` |
/// | Duldung | either | `P_ist,i` |
///
/// `vorgabe_kw` is the Netzbetreiber's `P_min,i` (positive call) or `P_max,i`
/// (negative call); ignored in the Duldungsfall.
///
/// Not for the Referenzprofilverfahren or a beidseitige Fixierung: there
/// *"Bei Verwendung
/// eines Referenzprofilverfahrens gilt abweichend im Duldungs- und
/// Aufforderungsfall"* `P_lim,i = P_max,i` / `P_min,i` — pass the
/// Netzbetreiber's figure straight on as `p_lim_kw`.
///
/// ```rust
/// use metering::grid::ausfallarbeit::{Redispatchfall, Redispatchrichtung, leistungslimitierung_kw};
/// use rust_decimal::dec;
///
/// // Told to cap at 300 kW, ran at 420: a negative call takes the maximum.
/// assert_eq!(
///     leistungslimitierung_kw(
///         Redispatchfall::Aufforderung, Redispatchrichtung::Negativ,
///         dec!(420), dec!(300),
///     ),
///     dec!(420),
/// );
///
/// // Duldung: what happened is the limit.
/// assert_eq!(
///     leistungslimitierung_kw(
///         Redispatchfall::Duldung, Redispatchrichtung::Negativ,
///         dec!(420), dec!(300),
///     ),
///     dec!(420),
/// );
/// ```
#[must_use]
pub fn leistungslimitierung_kw(
    fall: Redispatchfall,
    richtung: Redispatchrichtung,
    p_ist_kw: Decimal,
    vorgabe_kw: Decimal,
) -> Decimal {
    match (fall, richtung) {
        (Redispatchfall::Duldung, _) => p_ist_kw,
        (Redispatchfall::Aufforderung, Redispatchrichtung::Positiv) => p_ist_kw.min(vorgabe_kw),
        (Redispatchfall::Aufforderung, Redispatchrichtung::Negativ) => p_ist_kw.max(vorgabe_kw),
    }
}

// ── Kapitel 3.2.4.3 — der Anlagenfaktor ──────────────────────────────────────

/// The start of a quarter-hour: UTC minute 0, 15, 30 or 45, no seconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Viertelstunde(OffsetDateTime);

impl Viertelstunde {
    /// The quarter-hour starting at `start`; `None` when `start` is not on a
    /// quarter-hour boundary.
    #[must_use]
    pub fn new(start: OffsetDateTime) -> Option<Self> {
        let utc = start.to_offset(offset!(UTC));
        (utc.minute().is_multiple_of(15) && utc.second() == 0 && utc.nanosecond() == 0)
            .then_some(Self(start))
    }

    /// The instant the quarter-hour starts.
    #[must_use]
    pub const fn start(self) -> OffsetDateTime {
        self.0
    }
}

/// The Anlagenfaktor `AF` of the solar Pauschal-Abrechnung for one
/// quarter-hour — [BilAReM Kap. 3.2.4.3].
///
/// *"Zur Bestimmung der theoretischen Leistung in der Viertelstunde wird die
/// installierte Leistung der TR mit dem Anlagenfaktor multipliziert."*
///
/// | Jahreszeit | Uhrzeit (UTC+1) | `AF` |
/// |---|---|---|
/// | Sommer (01.03.–31.10.) | 19:00–6:00 | 0,0000 |
/// | | 6:00–9:00 | 0,2456 |
/// | | 9:00–15:00 | 0,6189 |
/// | | 15:00–19:00 | 0,2456 |
/// | Winter (01.11.–28./29.02.) | 16:45–9:00 | 0,0000 |
/// | | 9:00–10:00 | 0,2796 |
/// | | 10:00–14:00 | 0,5030 |
/// | | 14:00–16:45 | 0,2796 |
///
/// **Times and season are UTC+1 (MEZ) all year**, not Berlin local time. Every
/// band edge is a quarter-hour boundary, so the [`Viertelstunde`] start decides
/// the whole quarter-hour.
///
/// ```rust
/// use metering::grid::ausfallarbeit::{Viertelstunde, anlagenfaktor};
/// use rust_decimal::dec;
/// use time::macros::datetime;
///
/// let q = |t| Viertelstunde::new(t).unwrap();
/// // 12:00 UTC+1 on a June day is the middle summer band.
/// assert_eq!(anlagenfaktor(q(datetime!(2026-06-15 11:00 UTC))), dec!(0.6189));
/// // 19:30 UTC+1 is outside every summer band.
/// assert_eq!(anlagenfaktor(q(datetime!(2026-06-15 18:30 UTC))), dec!(0.0000));
/// // Winter closes at 16:45 UTC+1, not on the hour.
/// assert_eq!(anlagenfaktor(q(datetime!(2026-01-15 15:30 UTC))), dec!(0.2796));
/// assert_eq!(anlagenfaktor(q(datetime!(2026-01-15 15:45 UTC))), dec!(0.0000));
/// ```
#[must_use]
pub fn anlagenfaktor(viertelstunde: Viertelstunde) -> Decimal {
    let mez = viertelstunde.start().to_offset(offset!(+1));
    let minutes = u16::from(mez.hour()) * 60 + u16::from(mez.minute());
    if (3..=10).contains(&u8::from(mez.month())) {
        match minutes {
            m if (6 * 60..9 * 60).contains(&m) => dec!(0.2456),
            m if (9 * 60..15 * 60).contains(&m) => dec!(0.6189),
            m if (15 * 60..19 * 60).contains(&m) => dec!(0.2456),
            _ => dec!(0.0000),
        }
    } else {
        match minutes {
            m if (9 * 60..10 * 60).contains(&m) => dec!(0.2796),
            m if (10 * 60..14 * 60).contains(&m) => dec!(0.5030),
            m if (14 * 60..16 * 60 + 45).contains(&m) => dec!(0.2796),
            _ => dec!(0.0000),
        }
    }
}

// ── the published Ausfallarbeit formulas ─────────────────────────────────────

/// Quarter of an hour, the factor every BilAReM formula ends with.
const VIERTELSTUNDE_H: Decimal = dec!(0.25);

/// The smallest of `first` and the terms that are present.
fn min_of<const N: usize>(first: Decimal, rest: [Option<Decimal>; N]) -> Decimal {
    rest.into_iter().flatten().fold(first, Decimal::min)
}

/// `(theoretisch − p_lim) × ¼ h`, checked.
fn work(theoretisch: Decimal, p_lim_kw: Decimal) -> Option<Decimal> {
    theoretisch
        .checked_sub(p_lim_kw)?
        .checked_mul(VIERTELSTUNDE_H)
}

/// The theoretical power of a **fluctuating** plant under the Spitzabrechnung,
/// before the bounds — Kap. 3.2.2.1 (wind), Kap. 3.2.4.1 (solar).
///
/// Also the vereinfachte Spitzabrechnung (Kap. 3.2.2.2, 3.2.4.2), with weather
/// data from a provider or reference plant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Fluktuierend {
    /// Windenergieanlage an Land, Kap. 3.2.2.1: `KF × P_theo,i`.
    Wind {
        /// `P_theo,i` — from the certified Leistungskennlinie and the measured
        /// wind speed of the quarter-hour, kW.
        p_theo_kw: Decimal,
        /// `P_VZ,ist` — measured mean power of the four reference
        /// quarter-hours, kW.
        p_vz_ist_kw: Decimal,
        /// `P_VZ,theo` — theoretical mean power of the same four quarter-hours,
        /// kW. `KF = P_VZ,ist ÷ P_VZ,theo`.
        p_vz_theo_kw: Decimal,
    },
    /// Solaranlage, Kap. 3.2.4.1: `P_VZ,ist ÷ G_VZ × G_i`, bounded by `P_WR`.
    Solar {
        /// `G_i` — mean irradiance of the quarter-hour, kW/m².
        g_i_kw_m2: Decimal,
        /// `P_VZ,ist` — mean actual feed-in over the Vergleichszeitraum, kW.
        p_vz_ist_kw: Decimal,
        /// `G_VZ` — mean irradiance over the Vergleichszeitraum, kW/m².
        g_vz_kw_m2: Decimal,
        /// `P_WR` — Wechselrichterleistung je TR, kW.
        p_wr_kw: Decimal,
    },
}

/// Ausfallarbeit under the **Spitzabrechnung for fluctuating generation**, in
/// kWh — [BilAReM Kap. 3.2.2.1] (wind) and [Kap. 3.2.4.1] (solar).
///
/// ```text
/// Wind:   W_A,i = max{0; (min(KF × P_theo,i ; P_mbA,i ; P_bean,i) − P_lim,i) × ¼h}
/// Solar:  W_A,i = max{0; (min(P_VZ,ist ÷ G_VZ × G_i ; P_WR ; P_mbA,i ; P_bean,i) − P_lim,i) × ¼h}
/// ```
///
/// with the product capped at the Nennleistung: *"Das Produkt ist in diesem Fall auf die
/// Nennleistung der TR zu begrenzen."*
///
/// The product is formed multiply-first as one quotient and the result rounded
/// to [`AUSFALLARBEIT_DP`] ([`AUSFALLARBEIT_STRATEGY`]). `p_mba_kw` / `p_bean_kw`
/// are `None` where no marktbedingte Anpassung / Nichtbeanspruchbarkeit
/// applied. Kap. 3.2 covers negative Redispatch only, hence the clamp at zero.
///
/// `None` when `P_VZ,theo` or `G_VZ` is not positive, or on overflow.
///
/// ```rust
/// use metering::grid::ausfallarbeit::{Fluktuierend, spitz_fluktuierend_kwh};
/// use rust_decimal::dec;
///
/// // A 3 MW turbine: the power curve says 2 000 kW, the reference quarter-hours
/// // ran at 90 % of theirs (KF = 0,9), capped to 600 kW.
/// let wind = Fluktuierend::Wind {
///     p_theo_kw: dec!(2000),
///     p_vz_ist_kw: dec!(1800),
///     p_vz_theo_kw: dec!(2000),
/// };
/// assert_eq!(
///     spitz_fluktuierend_kwh(wind, dec!(3000), None, None, dec!(600)),
///     Some(dec!(300.000)), // (0,9 × 2 000 − 600) × ¼ h
/// );
/// ```
#[must_use]
pub fn spitz_fluktuierend_kwh(
    theoretisch: Fluktuierend,
    nennleistung_kw: Decimal,
    p_mba_kw: Option<Decimal>,
    p_bean_kw: Option<Decimal>,
    p_lim_kw: Decimal,
) -> Option<Decimal> {
    let (product, p_wr_kw) = match theoretisch {
        Fluktuierend::Wind {
            p_theo_kw,
            p_vz_ist_kw,
            p_vz_theo_kw,
        } => {
            if p_vz_theo_kw <= Decimal::ZERO {
                return None;
            }
            (
                p_vz_ist_kw
                    .checked_mul(p_theo_kw)?
                    .checked_div(p_vz_theo_kw)?,
                None,
            )
        }
        Fluktuierend::Solar {
            g_i_kw_m2,
            p_vz_ist_kw,
            g_vz_kw_m2,
            p_wr_kw,
        } => {
            if g_vz_kw_m2 <= Decimal::ZERO {
                return None;
            }
            (
                p_vz_ist_kw
                    .checked_mul(g_i_kw_m2)?
                    .checked_div(g_vz_kw_m2)?,
                Some(p_wr_kw),
            )
        }
    };
    let capped = product.min(nennleistung_kw);
    let theoretisch = min_of(capped, [p_wr_kw, p_mba_kw, p_bean_kw]);
    Some(
        work(theoretisch, p_lim_kw)?
            .max(Decimal::ZERO)
            .round_dp_with_strategy(AUSFALLARBEIT_DP, AUSFALLARBEIT_STRATEGY),
    )
}

/// Ausfallarbeit under the Pauschal-Abrechnung for a **Windenergieanlage an
/// Land**, in kWh — [BilAReM Kap. 3.2.2.3].
///
/// ```text
/// W_A,i = max{0; [min(P_0 ; P_inst ; P_mbA,i ; P_bean,i) − P_lim,i] × ¼h}
/// ```
///
/// `P_0` is [`p0_kw`]. `None` on overflow.
#[must_use]
pub fn pauschal_wind_kwh(
    p0_kw: Decimal,
    p_inst_kw: Decimal,
    p_mba_kw: Option<Decimal>,
    p_bean_kw: Option<Decimal>,
    p_lim_kw: Decimal,
) -> Option<Decimal> {
    let theoretisch = min_of(p0_kw, [Some(p_inst_kw), p_mba_kw, p_bean_kw]);
    Some(work(theoretisch, p_lim_kw)?.max(Decimal::ZERO))
}

/// Ausfallarbeit under the Pauschal-Abrechnung for a **Solaranlage**, in kWh —
/// [BilAReM Kap. 3.2.4.3].
///
/// ```text
/// W_A,i = max{0; [min(AF × P_inst ; P_WR ; P_mbA,i ; P_bean,i) − P_lim,i] × ¼h}
/// ```
///
/// `AF` is [`anlagenfaktor`]. `None` on overflow.
#[must_use]
pub fn pauschal_solar_kwh(
    anlagenfaktor: Decimal,
    p_inst_kw: Decimal,
    p_wr_kw: Decimal,
    p_mba_kw: Option<Decimal>,
    p_bean_kw: Option<Decimal>,
    p_lim_kw: Decimal,
) -> Option<Decimal> {
    let theoretisch = min_of(
        anlagenfaktor.checked_mul(p_inst_kw)?,
        [Some(p_wr_kw), p_mba_kw, p_bean_kw],
    );
    Some(work(theoretisch, p_lim_kw)?.max(Decimal::ZERO))
}

/// Ausfallarbeit under the Pauschal-Abrechnung for a plant with
/// **nicht-fluktuierender Erzeugung**, in kWh — [BilAReM Kap. 3.3.2].
///
/// ```text
/// positiv:  W_A,i = min{0; (P_0 − min(P_lim,i ; P_bean,i)) × ¼h}
/// negativ:  W_A,i = max{0; (min(P_0 ; P_bean,i) − P_lim,i) × ¼h}
/// ```
///
/// The Nichtbeanspruchbarkeit changes sides with the direction, as published.
/// `None` on overflow.
#[must_use]
pub fn pauschal_nicht_fluktuierend_kwh(
    richtung: Redispatchrichtung,
    p0_kw: Decimal,
    p_bean_kw: Option<Decimal>,
    p_lim_kw: Decimal,
) -> Option<Decimal> {
    Some(match richtung {
        Redispatchrichtung::Positiv => {
            work(p0_kw, min_of(p_lim_kw, [p_bean_kw]))?.min(Decimal::ZERO)
        }
        Redispatchrichtung::Negativ => {
            work(min_of(p0_kw, [p_bean_kw]), p_lim_kw)?.max(Decimal::ZERO)
        }
    })
}

/// Ausfallarbeit under the Spitzabrechnung for a plant with
/// **nicht-fluktuierender Erzeugung**, in kWh — [BilAReM Kap. 3.3.1].
///
/// *"Bei der Spitzabrechnung ist die Ausfallarbeit die Differenz zwischen der
/// geplanten Fahrweise und der Fahrweise aufgrund des Werts der
/// Leistungslimitierung."*
///
/// ```text
/// positiv:  W_A,i = min{0; (P_plan,i − P_lim,i) × ¼h}
/// negativ:  W_A,i = max{0; (P_plan,i − P_lim,i) × ¼h}
/// ```
///
/// `p_plan_kw` is the Ex-ante-Planungsdaten figure. `None` on overflow.
#[must_use]
pub fn spitz_nicht_fluktuierend_kwh(
    richtung: Redispatchrichtung,
    p_plan_kw: Decimal,
    p_lim_kw: Decimal,
) -> Option<Decimal> {
    let raw = work(p_plan_kw, p_lim_kw)?;
    Some(match richtung {
        Redispatchrichtung::Positiv => raw.min(Decimal::ZERO),
        Redispatchrichtung::Negativ => raw.max(Decimal::ZERO),
    })
}

// ── P_0 — the last undisturbed quarter-hour ──────────────────────────────────

/// `P_0` in **kW**: the mean power of the last fully measured quarter-hour
/// before `massnahme_beginn` — [BilAReM Kap. 3.2.2.3, Kap. 3.3.2].
///
/// *"gemessener Leistungsmittelwert der TR in der letzten vollständig
/// gemessenen Viertelstunde vor der Redispatch-Maßnahme, in der uneingeschränkt
/// eingespeist werden konnte"*.
///
/// The latest 15-minute interval ending at or before the measure with quality
/// exactly [`Measured`](QualityFlag::Measured), as [`MeterInterval::demand_kw`].
/// Whether the plant fed in *"uneingeschränkt"* is not in the series: filter
/// curtailed or unavailable quarter-hours out first.
///
/// `None` when no interval qualifies. The unit is kW, as Kap. 3.3.2 prints for
/// the same symbol (Kap. 3.2.2.3's legend prints kWh, which would not make
/// `[… − P_lim,i] × ¼h` a work).
#[must_use]
pub fn p0_kw(series: &[MeterInterval], massnahme_beginn: OffsetDateTime) -> Option<Decimal> {
    series
        .iter()
        .filter(|iv| {
            iv.to() <= massnahme_beginn
                && iv.quality() == QualityFlag::Measured
                && iv.duration() == time::Duration::minutes(15)
        })
        .max_by_key(|iv| iv.to())?
        .demand_kw()
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::{date, datetime};

    #[test]
    fn the_aufforderungsfall_takes_the_min_going_up_and_the_max_coming_down() {
        use Redispatchfall::Aufforderung as A;
        use Redispatchrichtung::{Negativ, Positiv};
        assert_eq!(
            leistungslimitierung_kw(A, Positiv, dec!(80), dec!(300)),
            dec!(80)
        );
        assert_eq!(
            leistungslimitierung_kw(A, Positiv, dec!(400), dec!(300)),
            dec!(300)
        );
        assert_eq!(
            leistungslimitierung_kw(A, Negativ, dec!(420), dec!(300)),
            dec!(420)
        );
        assert_eq!(
            leistungslimitierung_kw(A, Negativ, dec!(200), dec!(300)),
            dec!(300)
        );
    }

    #[test]
    fn the_duldungsfall_is_what_actually_happened() {
        for richtung in Redispatchrichtung::ALL {
            for vorgabe in [dec!(0), dec!(300), dec!(9999)] {
                assert_eq!(
                    leistungslimitierung_kw(Redispatchfall::Duldung, richtung, dec!(420), vorgabe),
                    dec!(420),
                );
            }
        }
    }

    fn wind(p_theo: Decimal, ist: Decimal, theo: Decimal) -> Fluktuierend {
        Fluktuierend::Wind {
            p_theo_kw: p_theo,
            p_vz_ist_kw: ist,
            p_vz_theo_kw: theo,
        }
    }

    /// The 3.2.2.1 formula with every term: `KF × P_theo` against `P_mbA`,
    /// `P_bean` and the Nennleistung cap.
    #[test]
    fn the_wind_spitzabrechnung_applies_kf_the_bounds_and_the_cap() {
        // KF = 1 800 / 2 000 = 0,9; 0,9 × 2 000 = 1 800.
        let w = wind(dec!(2000), dec!(1800), dec!(2000));
        assert_eq!(
            spitz_fluktuierend_kwh(w, dec!(3000), None, None, dec!(600)),
            Some(dec!(300.000))
        );
        // P_mbA = 1 400 is lower and wins: (1 400 − 600) × ¼.
        assert_eq!(
            spitz_fluktuierend_kwh(w, dec!(3000), Some(dec!(1400)), Some(dec!(1600)), dec!(600)),
            Some(dec!(200.000))
        );
        // P_bean = 1 000 lower still.
        assert_eq!(
            spitz_fluktuierend_kwh(w, dec!(3000), Some(dec!(1400)), Some(dec!(1000)), dec!(600)),
            Some(dec!(100.000))
        );
        // KF = 1,5 pushes 1,5 × 2 400 = 3 600 above a 3 000 kW Nennleistung:
        // "auf die Nennleistung der TR zu begrenzen" → (3 000 − 600) × ¼.
        let over = wind(dec!(2400), dec!(1500), dec!(1000));
        assert_eq!(
            spitz_fluktuierend_kwh(over, dec!(3000), None, None, dec!(600)),
            Some(dec!(600.000))
        );
        // Negative Redispatch only: a limit above the theoretical power is zero.
        assert_eq!(
            spitz_fluktuierend_kwh(w, dec!(3000), None, None, dec!(2500)),
            Some(dec!(0.000))
        );
        // No reference power, no KF.
        assert_eq!(
            spitz_fluktuierend_kwh(
                wind(dec!(1), dec!(1), dec!(0)),
                dec!(3),
                None,
                None,
                dec!(0)
            ),
            None
        );
    }

    /// A non-terminating KF is formed as one quotient and rounded once.
    #[test]
    fn a_non_terminating_kf_is_cut_once_to_watt_hours() {
        // KF = 1 000 / 3 000; × 2 000 = 666,666… kW; − 0 → × ¼ = 166,666…
        let w = wind(dec!(2000), dec!(1000), dec!(3000));
        assert_eq!(
            spitz_fluktuierend_kwh(w, dec!(5000), None, None, dec!(0)),
            Some(dec!(166.667))
        );
    }

    #[test]
    fn the_solar_spitzabrechnung_scales_the_irradiance_and_respects_the_inverter() {
        let solar = |p_wr| Fluktuierend::Solar {
            g_i_kw_m2: dec!(0.8),
            p_vz_ist_kw: dec!(600),
            g_vz_kw_m2: dec!(0.6),
            p_wr_kw: p_wr,
        };
        // 600 / 0,6 × 0,8 = 800 kW.
        assert_eq!(
            spitz_fluktuierend_kwh(solar(dec!(1000)), dec!(1000), None, None, dec!(200)),
            Some(dec!(150.000))
        );
        // A 700 kW inverter bounds it.
        assert_eq!(
            spitz_fluktuierend_kwh(solar(dec!(700)), dec!(1000), None, None, dec!(200)),
            Some(dec!(125.000))
        );
        // …and the Nennleistung caps the product.
        assert_eq!(
            spitz_fluktuierend_kwh(solar(dec!(1000)), dec!(750), None, None, dec!(200)),
            Some(dec!(137.500))
        );
        let dark = Fluktuierend::Solar {
            g_i_kw_m2: dec!(0.8),
            p_vz_ist_kw: dec!(600),
            g_vz_kw_m2: dec!(0),
            p_wr_kw: dec!(700),
        };
        assert_eq!(
            spitz_fluktuierend_kwh(dark, dec!(1000), None, None, dec!(0)),
            None
        );
    }

    #[test]
    fn the_wind_pauschal_takes_the_lowest_bound() {
        assert_eq!(
            pauschal_wind_kwh(dec!(900), dec!(1000), None, None, dec!(300)),
            Some(dec!(150.00))
        );
        assert_eq!(
            pauschal_wind_kwh(dec!(1200), dec!(1000), None, None, dec!(300)),
            Some(dec!(175.00))
        );
        assert_eq!(
            pauschal_wind_kwh(
                dec!(900),
                dec!(1000),
                Some(dec!(600)),
                Some(dec!(700)),
                dec!(300)
            ),
            Some(dec!(75.00))
        );
        assert_eq!(
            pauschal_wind_kwh(dec!(300), dec!(1000), None, None, dec!(900)),
            Some(Decimal::ZERO)
        );
    }

    #[test]
    fn the_solar_pauschal_is_the_anlagenfaktor_bounded_by_the_inverter() {
        assert_eq!(
            pauschal_solar_kwh(dec!(0.6189), dec!(1000), dec!(700), None, None, dec!(100)),
            Some(dec!(129.725000))
        );
        assert_eq!(
            pauschal_solar_kwh(dec!(0.6189), dec!(1000), dec!(500), None, None, dec!(100)),
            Some(dec!(100.00))
        );
    }

    #[test]
    fn the_nichtbeanspruchbarkeit_changes_sides_with_the_direction() {
        use Redispatchrichtung::{Negativ, Positiv};
        assert_eq!(
            pauschal_nicht_fluktuierend_kwh(Negativ, dec!(900), Some(dec!(600)), dec!(300)),
            Some(dec!(75.00)),
        );
        assert_eq!(
            pauschal_nicht_fluktuierend_kwh(Positiv, dec!(900), Some(dec!(600)), dec!(1500)),
            Some(Decimal::ZERO),
        );
        assert_eq!(
            pauschal_nicht_fluktuierend_kwh(Positiv, dec!(300), None, dec!(900)),
            Some(dec!(-150.00)),
        );
    }

    #[test]
    fn a_positive_redispatch_yields_negative_ausfallarbeit() {
        use Redispatchrichtung::{Negativ, Positiv};
        assert_eq!(
            spitz_nicht_fluktuierend_kwh(Positiv, dec!(200), dec!(600)),
            Some(dec!(-100.00))
        );
        assert_eq!(
            spitz_nicht_fluktuierend_kwh(Negativ, dec!(600), dec!(200)),
            Some(dec!(100.00))
        );
        assert_eq!(
            spitz_nicht_fluktuierend_kwh(Positiv, dec!(600), dec!(200)),
            Some(Decimal::ZERO)
        );
        assert_eq!(
            spitz_nicht_fluktuierend_kwh(Negativ, dec!(200), dec!(600)),
            Some(Decimal::ZERO)
        );
        assert_eq!(
            spitz_nicht_fluktuierend_kwh(Negativ, Decimal::MAX, Decimal::MIN),
            None,
            "overflow is refused, not a panic"
        );
    }

    fn q(t: OffsetDateTime) -> Viertelstunde {
        Viertelstunde::new(t).unwrap()
    }

    #[test]
    fn the_anlagenfaktor_reproduces_the_published_table() {
        let sommer = |m: i64| {
            anlagenfaktor(q(
                datetime!(2026-06-15 0:00 UTC) + time::Duration::minutes(m)
            ))
        };
        assert_eq!(sommer(5 * 60), dec!(0.2456)); //  6:00 MEZ
        assert_eq!(sommer(8 * 60), dec!(0.6189)); //  9:00 MEZ
        assert_eq!(sommer(14 * 60), dec!(0.2456)); // 15:00 MEZ
        assert_eq!(sommer(18 * 60), dec!(0.0000)); // 19:00 MEZ
        assert_eq!(sommer(4 * 60 + 45), dec!(0.0000)); //  5:45 MEZ
        let winter = |m: i64| {
            anlagenfaktor(q(
                datetime!(2026-01-15 0:00 UTC) + time::Duration::minutes(m)
            ))
        };
        assert_eq!(winter(8 * 60), dec!(0.2796)); //  9:00 MEZ
        assert_eq!(winter(9 * 60), dec!(0.5030)); // 10:00 MEZ
        assert_eq!(winter(13 * 60), dec!(0.2796)); // 14:00 MEZ
        assert_eq!(winter(15 * 60 + 30), dec!(0.2796)); // 16:30 MEZ
        assert_eq!(winter(15 * 60 + 45), dec!(0.0000)); // 16:45 MEZ
    }

    /// UTC+1 all year: 10:30 CEST (08:30 UTC) is 09:30 MEZ, the middle band.
    #[test]
    fn the_table_is_mez_not_local_time() {
        assert_eq!(
            anlagenfaktor(q(datetime!(2026-06-15 6:30 UTC))),
            dec!(0.2456)
        );
        assert_eq!(
            anlagenfaktor(q(datetime!(2026-06-15 8:30 UTC))),
            dec!(0.6189)
        );
    }

    /// Summer is 01.03.–31.10., leap day included in winter.
    #[test]
    fn the_seasons_switch_on_the_first_of_march_and_november_in_mez() {
        assert_eq!(
            anlagenfaktor(q(datetime!(2026-02-28 9:00 UTC))),
            dec!(0.5030)
        );
        assert_eq!(
            anlagenfaktor(q(datetime!(2026-03-01 9:00 UTC))),
            dec!(0.6189)
        );
        assert_eq!(
            anlagenfaktor(q(datetime!(2026-11-01 9:00 UTC))),
            dec!(0.5030)
        );
        assert_eq!(
            anlagenfaktor(q(datetime!(2028-02-29 9:00 UTC))),
            dec!(0.5030)
        );
    }

    #[test]
    fn a_viertelstunde_starts_on_a_quarter_hour() {
        assert!(Viertelstunde::new(datetime!(2026-06-15 8:45 UTC)).is_some());
        assert!(Viertelstunde::new(datetime!(2026-06-15 8:50 UTC)).is_none());
        assert!(Viertelstunde::new(datetime!(2026-06-15 8:45:01 UTC)).is_none());
        // A Berlin-offset instant on the quarter-hour is one too.
        assert!(Viertelstunde::new(datetime!(2026-06-15 10:45 +2)).is_some());
    }

    fn iv(from: OffsetDateTime, kwh: Decimal, quality: QualityFlag) -> MeterInterval {
        MeterInterval::quarter_hour(from, kwh, quality).unwrap()
    }

    /// The last **fully measured** quarter-hour wholly before the measure, as
    /// a power: 120 kWh in a quarter-hour is 480 kW.
    #[test]
    fn p0_is_the_power_of_the_last_measured_quarter_hour_before_the_measure() {
        let base = datetime!(2026-06-01 10:00 UTC);
        let m = |n: i64| base + time::Duration::minutes(15 * n);
        let series = vec![
            iv(m(0), dec!(100), QualityFlag::Measured),
            iv(m(1), dec!(120), QualityFlag::Measured),
            iv(m(2), dec!(130), QualityFlag::Substituted),
            iv(m(3), dec!(40), QualityFlag::Measured),
        ];
        assert_eq!(p0_kw(&series, m(3)), Some(dec!(480)));
    }

    #[test]
    fn p0_refuses_what_is_not_a_measured_quarter_hour() {
        let base = datetime!(2026-06-01 10:00 UTC);
        let after = base + time::Duration::hours(1);
        assert_eq!(
            p0_kw(&[iv(base, dec!(100), QualityFlag::Faulty)], after),
            None
        );
        assert_eq!(p0_kw(&[], base), None);
        for quality in [
            QualityFlag::Substituted,
            QualityFlag::Estimated,
            QualityFlag::Calculated,
        ] {
            assert_eq!(
                p0_kw(&[iv(base, dec!(100), quality)], after),
                None,
                "{quality}"
            );
        }
        // An hour is not a quarter-hour.
        let hour = MeterInterval::hour(base, dec!(100), QualityFlag::Measured).unwrap();
        assert_eq!(p0_kw(&[hour], after), None);
    }

    #[test]
    fn the_pauschal_bestandsschutz_ends_with_2028() {
        assert_eq!(PAUSCHAL_BESTANDSSCHUTZ_ENDE, date!(2028 - 12 - 31));
    }
}
