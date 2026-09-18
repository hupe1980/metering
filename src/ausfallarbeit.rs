//! Ausfallarbeit — the energy a Redispatch measure prevented, or forced.
//!
//! ## Legal basis
//!
//! - **§ 13a Abs. 1a EnWG** (i. V. m. § 14 Abs. 1 bzw. Abs. 1c Satz 1) — the
//!   bilanzieller Ausgleich a Redispatch measure entitles the
//!   Bilanzkreisverantwortlicher to; **§ 13a Abs. 2** the financial one the
//!   Anlagenbetreiber gets.
//! - **BNetzA BK6-23-241** (Beschluss 07.05.2026) and its Anlage
//!   *"Bilanzieller Ausgleich von Redispatch-Maßnahmen (BilAReM)"*, Kapitel 3.
//!   It *"ersetzen die Regelungen in den Festlegungen BK6-20-059, BK6-20-060
//!   und BK6-20-061"*, and the annexed exchange processes apply from
//!   01.10.2026.
//! - **BDEW-Leitfaden zur Berechnung der Ausfallarbeit Redispatch 2.0**
//!   (Mai 2020) — the industry description of the same three
//!   Abrechnungsvarianten.
//!
//! ## What the quantity is
//!
//! [BilAReM Kap. 3]: *"Ausfallarbeit ist – arbeitsbezogen – die Differenz
//! zwischen der theoretischen Erzeugung einer TR und dem Wert der
//! Leistungslimitierung"*. Two consequences the sign convention rests on, from
//! the same sentence: *"bei negativem Redispatch ist die Ausfallarbeit positiv,
//! bei positivem Redispatch ist die Ausfallarbeit negativ (Mehrarbeit)"*.
//!
//! So it is **signed**, not an absolute shortfall — a positive Redispatch
//! makes a plant produce *more* than it would have, and that is Mehrarbeit
//! carried as a negative Ausfallarbeit. Every published formula below then
//! clamps on the side its own direction allows.
//!
//! Everything is per **Viertelstunde** and per **Technische Ressource**:
//! *"Die Ausfallarbeit wird für jede TR bestimmt"*, and *"Soweit in diesem
//! Kapitel Leistungswerte genannt werden, sind Viertelstundenmittelwerte
//! gemeint"*.
//!
//! ## What this module computes, and what it takes
//!
//! | Variante | Datengrundlage | Here |
//! |---|---|---|
//! | Spitzabrechnung (fluktuierend) | *"gemessene Wetterdaten der TR"* | the theoretical series is **supplied** |
//! | vereinfachte Spitzabrechnung | *"mit Referenzmesswerten oder Wetterdaten für den Standort"* | supplied |
//! | Spitzabrechnung (nicht-fluktuierend) | Ex-ante-Planungsdaten | [`spitz_nicht_fluktuierend_kwh`] |
//! | Pauschal-Abrechnung | the last quarter-hour, or an Anlagenfaktor | [`pauschal_wind_kwh`], [`pauschal_solar_kwh`], [`pauschal_nicht_fluktuierend_kwh`] |
//!
//! Turning a measured wind speed or irradiance into a theoretical power needs a
//! power curve and a site model — data an operator holds and a Wetterdienst
//! supplies. This crate does the arithmetic and takes that series as an
//! argument, exactly as it takes SLP value tables ([`crate::load_profile`]).
//! The Wind-Bin-Verfahren for Windenergieanlagen auf See is out for the same
//! reason.
//!
//! ## The Pauschal-Abrechnung has an end date
//!
//! [`PAUSCHAL_BESTANDSSCHUTZ_ENDE`]. The Beschluss is explicit about why: the
//! method *"ist nicht geeignet, die Ausfallarbeit von Redispatch-Maßnahmen mit
//! diesen Anlagen ausreichend genau zu bestimmen"*, because it assumes the
//! primary energy available during the measure is unchanged — *"Das mag bei
//! kurzen Redispatch-Maßnahmen im Einzelfall näherungsweise zutreffen, bei
//! längeren Maßnahmen aber nicht"*. It is modelled because it is in force,
//! with the date on the constant rather than in a comment.

use rust_decimal::{Decimal, dec};
use time::{Date, Month, OffsetDateTime, UtcOffset};

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use crate::interval::MeterInterval;

// ── direction and case ────────────────────────────────────────────────────────

/// Which way a Redispatch measure moved the plant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "SCREAMING_SNAKE_CASE"))]
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

    /// Stable DB/wire label. Matches the `serde` tag and
    /// [`FromStr`](std::str::FromStr) input.
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
/// [BilAReM Kap. 3.1] keeps them apart, and the Beschluss says why: in the
/// Aufforderungsfall *"trägt im Aufforderungsfall der BKV des LF das Risiko,
/// dass die Redispatch-Anweisung korrekt umgesetzt wird"* — so the limit is the
/// instruction met against what actually happened. In the Duldungsfall the
/// operator intervened directly and what happened **is** the limit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "SCREAMING_SNAKE_CASE"))]
pub enum Redispatchfall {
    /// The Anlagenbetreiber was instructed and implemented it themselves.
    Aufforderung,
    /// The Netzbetreiber intervened; the plant tolerated it.
    Duldung,
}

impl Redispatchfall {
    /// Both cases, in declaration order.
    pub const ALL: [Self; 2] = [Self::Aufforderung, Self::Duldung];

    /// Stable DB/wire label. Matches the `serde` tag and
    /// [`FromStr`](std::str::FromStr) input.
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
/// [BilAReM Kap. 3.2.1] for fluktuierende Erzeugung, Kap. 3.3 for the
/// non-fluctuating case, which has only two.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "SCREAMING_SNAKE_CASE"))]
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
    /// Closed to new Technische Ressourcen and ending for the rest — see
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

    /// Stable DB/wire label. Matches the `serde` tag and
    /// [`FromStr`](std::str::FromStr) input.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Spitzabrechnung => "SPITZABRECHNUNG",
            Self::VereinfachteSpitzabrechnung => "VEREINFACHTE_SPITZABRECHNUNG",
            Self::PauschalAbrechnung => "PAUSCHAL_ABRECHNUNG",
        }
    }

    /// `true` when a Technische Ressource may still be assigned to this variant
    /// on `at`.
    ///
    /// Only the Pauschal-Abrechnung ever answers `false`, and only from
    /// [`PAUSCHAL_BESTANDSSCHUTZ_ENDE`] onwards. **This is not the whole
    /// admissibility test**: a TR also has to have been in the Pauschal-
    /// Abrechnung at the Bekanntmachung of the Festlegung to be in it at all,
    /// and a TR in the Planwertmodell is excluded outright — both facts about a
    /// portfolio, which this crate cannot see. It answers the part that is a
    /// date.
    #[must_use]
    pub fn is_available_on(self, at: Date) -> bool {
        !matches!(self, Self::PauschalAbrechnung) || at <= PAUSCHAL_BESTANDSSCHUTZ_ENDE
    }
}

crate::codes::string_codes! {
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
///
/// A date rather than a comment, because a settlement run for 2029 that still
/// uses the Pauschal figures is computing a number nobody may bill on.
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
/// `vorgabe_kw` is the Netzbetreiber's figure from the Redispatch-Abrufinformation
/// — the durchschnittliche Mindesterzeugung `P_min,i` for a positive call and
/// the durchschnittliche Höchsterzeugung `P_max,i` for a negative one. It is
/// ignored in the Duldungsfall, where the instruction was not the plant's to
/// implement; pass it anyway and the answer does not change.
///
/// For a Referenzprofilverfahren, and for a measure with beidseitiger
/// Fixierung, use [`leistungslimitierung_referenzprofil_kw`] instead.
///
/// ```rust
/// use metering::ausfallarbeit::{Redispatchfall, Redispatchrichtung, leistungslimitierung_kw};
/// use rust_decimal::dec;
///
/// // Told to cap at 300 kW, actually ran at 420: the cap is what counts.
/// assert_eq!(
///     leistungslimitierung_kw(
///         Redispatchfall::Aufforderung, Redispatchrichtung::Negativ,
///         dec!(420), dec!(300),
///     ),
///     dec!(420),
/// );
/// // ...because a negative call takes the *maximum*: the BKV carries the risk
/// // that the instruction was implemented, so over-delivery is not the grid's.
///
/// // Under Duldung the operator intervened, so what happened is the limit.
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

/// `P_lim,i` under a Referenzprofilverfahren, in kW — [BilAReM Kap. 3.1].
///
/// *"Bei Verwendung eines Referenzprofilverfahrens gilt abweichend im Duldungs-
/// und Aufforderungsfall"*: the Netzbetreiber's own figure is the limit, with
/// no reference to what the plant actually did — `P_max,i` for a negative call,
/// `P_min,i` for a positive one. There is no `P_ist` in either, which is why
/// this is a separate function rather than a flag on the other: a caller with
/// no measurement has nothing to pass.
///
/// *"Das gilt auch, wenn der Netzbetreiber einen bestimmten Leistungswert
/// vorgibt, von dem nicht abgewichen werden darf (Redispatch-Maßnahme mit
/// beidseitiger Fixierung)."*
#[must_use]
pub const fn leistungslimitierung_referenzprofil_kw(
    _richtung: Redispatchrichtung,
    vorgabe_kw: Decimal,
) -> Decimal {
    // Both directions take the Netzbetreiber's figure; the direction decides
    // only which figure that is (`P_max` or `P_min`), and the caller supplies
    // it. The parameter stays so a call site reads the same as the other
    // function's and states which measure it is settling.
    vorgabe_kw
}

// ── Kapitel 3.2.4 — der Anlagenfaktor ────────────────────────────────────────

/// The Anlagenfaktor `AF` for a solar Pauschal-Abrechnung — [BilAReM Kap. 3.2.4.3].
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
/// **The table is in UTC+1, not in local time.** The Festlegung prints
/// *"Uhrzeit (UTC+1)"*, which is MEZ all year — so in summer the bands sit an
/// hour off the wall clock, and a caller that reads them as Europe/Berlin local
/// time shifts every summer band by an hour. That is the one thing about this
/// table worth getting right, and it is why this takes an instant rather than a
/// time of day.
///
/// The seasons are calendar dates, so the boundary is the same instant for
/// every year and a leap February needs no special case.
///
/// ```rust
/// use metering::ausfallarbeit::anlagenfaktor;
/// use rust_decimal::dec;
/// use time::macros::datetime;
///
/// // 12:00 UTC+1 on a June day is the middle summer band.
/// assert_eq!(anlagenfaktor(datetime!(2026-06-15 11:00 UTC)), dec!(0.6189));
/// // The same wall-clock hour in Berlin (13:00 CEST) is 12:00 UTC+1 — still
/// // the middle band, but only because the offset was applied.
/// assert_eq!(anlagenfaktor(datetime!(2026-06-15 18:30 UTC)), dec!(0.0000));
/// // Winter closes at 16:45 UTC+1, not on the hour.
/// assert_eq!(anlagenfaktor(datetime!(2026-01-15 15:30 UTC)), dec!(0.2796));
/// assert_eq!(anlagenfaktor(datetime!(2026-01-15 15:50 UTC)), dec!(0.0000));
/// ```
#[must_use]
pub fn anlagenfaktor(at: OffsetDateTime) -> Decimal {
    // UTC+1 — "MEZ", fixed all year. `from_hms` cannot fail for a whole hour
    // inside the valid range, and the fallback is UTC rather than a panic.
    let mez = at.to_offset(UtcOffset::from_hms(1, 0, 0).unwrap_or(UtcOffset::UTC));
    let minutes = i32::from(mez.hour()) * 60 + i32::from(mez.minute());
    let month = mez.month() as u8;

    // "Sommer 01.03.–31.10." — the rest is winter.
    if (3..=10).contains(&month) {
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

/// Ausfallarbeit under the Pauschal-Abrechnung for a **Windenergieanlage an
/// Land**, in kWh — [BilAReM Kap. 3.2.2.3].
///
/// ```text
/// W_A,i = max{0; [min(P_0 ; P_inst ; P_mbA,i ; P_bean,i) − P_lim,i] × ¼h}
/// ```
///
/// *"Bei der Pauschal-Abrechnung entspricht die Ausfallarbeit grundsätzlich der
/// Differenz zwischen dem letzten vollständig gemessenen Leistungsmittelwert
/// vor der Redispatch-Maßnahme […] und dem Wert der Leistungslimitierung durch
/// die Redispatch-Anweisung […]. Liegt für die betroffene Viertelstunde eine
/// marktbedingte Anpassung oder eine Nichtbeanspruchbarkeit vor, ist der
/// niedrigste Wert […] maßgeblich."*
///
/// The elisions are the formula's own variable names, which the Festlegung
/// sets in maths italic and a text extractor returns with the subscripts
/// detached; they are `P_0`, `P_lim,i`, and `P_0, P_mbA,i und P_bean,i`.
///
/// `p_mba_kw` and `p_bean_kw` are `None` where no marktbedingte Anpassung or
/// Nichtbeanspruchbarkeit applied — absent rather than infinite, because a
/// caller has no sentinel to reach for and `min` with a missing term is not the
/// same statement as `min` with an unbounded one.
///
/// Kapitel 3.2 applies *"nur für den Fall des negativen Redispatch"*, which is
/// why this clamps at zero and has no direction parameter.
#[must_use]
pub fn pauschal_wind_kwh(
    p0_kw: Decimal,
    p_inst_kw: Decimal,
    p_mba_kw: Option<Decimal>,
    p_bean_kw: Option<Decimal>,
    p_lim_kw: Decimal,
) -> Decimal {
    let theoretisch = min_of([Some(p0_kw), Some(p_inst_kw), p_mba_kw, p_bean_kw]);
    ((theoretisch - p_lim_kw) * VIERTELSTUNDE_H).max(Decimal::ZERO)
}

/// Ausfallarbeit under the Pauschal-Abrechnung for a **Solaranlage**, in kWh —
/// [BilAReM Kap. 3.2.4.3].
///
/// ```text
/// W_A,i = max{0; [min(AF × P_inst ; P_WR ; P_mbA,i ; P_bean,i) − P_lim,i] × ¼h}
/// ```
///
/// The solar variant does not carry a last-quarter-hour value forward at all —
/// its theoretical power is the installed nominal power times the
/// [`anlagenfaktor`], bounded by the inverter. `P_inst` is *"die Summe der
/// Nennleistung der Module"*, and `P_WR` the *"Wechselrichterleistung je TR"*.
#[must_use]
pub fn pauschal_solar_kwh(
    anlagenfaktor: Decimal,
    p_inst_kw: Decimal,
    p_wr_kw: Decimal,
    p_mba_kw: Option<Decimal>,
    p_bean_kw: Option<Decimal>,
    p_lim_kw: Decimal,
) -> Decimal {
    let theoretisch = min_of([
        Some(anlagenfaktor * p_inst_kw),
        Some(p_wr_kw),
        p_mba_kw,
        p_bean_kw,
    ]);
    ((theoretisch - p_lim_kw) * VIERTELSTUNDE_H).max(Decimal::ZERO)
}

/// Ausfallarbeit under the Pauschal-Abrechnung for a plant with
/// **nicht-fluktuierender Erzeugung**, in kWh — [BilAReM Kap. 3.3.2].
///
/// ```text
/// positiv:  W_A,i = min{0; (P_0 − min(P_lim,i ; P_bean,i)) × ¼h}
/// negativ:  W_A,i = max{0; (min(P_0 ; P_bean,i) − P_lim,i) × ¼h}
/// ```
///
/// **The Nichtbeanspruchbarkeit changes sides with the direction**, and that is
/// the Festlegung's own asymmetry rather than a simplification: for a negative
/// call it bounds the theoretical power, for a positive one it bounds the
/// limit. Reading it as *"`P_bean` always caps `P_0`"* gives a different number
/// whenever both apply.
#[must_use]
pub fn pauschal_nicht_fluktuierend_kwh(
    richtung: Redispatchrichtung,
    p0_kw: Decimal,
    p_bean_kw: Option<Decimal>,
    p_lim_kw: Decimal,
) -> Decimal {
    match richtung {
        Redispatchrichtung::Positiv => {
            let limit = min_of([Some(p_lim_kw), p_bean_kw]);
            ((p0_kw - limit) * VIERTELSTUNDE_H).min(Decimal::ZERO)
        }
        Redispatchrichtung::Negativ => {
            let theoretisch = min_of([Some(p0_kw), p_bean_kw]);
            ((theoretisch - p_lim_kw) * VIERTELSTUNDE_H).max(Decimal::ZERO)
        }
    }
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
/// `p_plan_kw` is the Ex-ante-Planungsdaten figure, and the Festlegung puts one
/// condition on it the caller owns: *"soweit die Einspeisung aufgrund sonstiger
/// Gründe (z. B. ungeplante Nichtverfügbarkeit) beeinträchtigt ist, sind diese
/// bei der Bestimmung von P_plan,i zu berücksichtigen"*.
///
/// The same two lines serve the fluctuating Spitzabrechnung, where `p_plan_kw`
/// is the weather-derived theoretical power instead — the arithmetic does not
/// change with where the number came from, and this crate does not produce that
/// number either way.
#[must_use]
pub fn spitz_nicht_fluktuierend_kwh(
    richtung: Redispatchrichtung,
    p_plan_kw: Decimal,
    p_lim_kw: Decimal,
) -> Decimal {
    let raw = (p_plan_kw - p_lim_kw) * VIERTELSTUNDE_H;
    match richtung {
        Redispatchrichtung::Positiv => raw.min(Decimal::ZERO),
        Redispatchrichtung::Negativ => raw.max(Decimal::ZERO),
    }
}

/// The smallest of the terms that are present. At least one must be.
fn min_of<const N: usize>(terms: [Option<Decimal>; N]) -> Decimal {
    terms
        .into_iter()
        .flatten()
        .reduce(Decimal::min)
        .unwrap_or(Decimal::ZERO)
}

// ── P_0 — the last undisturbed quarter-hour ──────────────────────────────────

/// `P_0` in kW: the average power of the last fully measured quarter-hour
/// before `massnahme_beginn` — [BilAReM Kap. 3.2.2.3, Kap. 3.3.2].
///
/// *"gemessener Leistungsmittelwert der TR in der letzten vollständig
/// gemessenen Viertelstunde vor der Redispatch-Maßnahme, in der uneingeschränkt
/// eingespeist werden konnte"*.
///
/// Three conditions, and this function answers two of them. It takes the latest
/// interval that ends at or before the measure began (*"vor der
/// Redispatch-Maßnahme"*, fully outside it) and whose quality is exactly
/// [`Measured`](crate::QualityFlag::Measured).
///
/// **Measured, not billable.** An Ersatzwert is billable — that is what
/// Ersatzwertbildung is for — and it is not *"vollständig gemessen"*. Carrying
/// a substituted value forward as `P_0` would make the Pauschal-Abrechnung a
/// projection of a projection, and the compensation it sizes would rest on a
/// figure no meter produced. `is_billable()` is the wrong test here and the
/// right one nearly everywhere else in this crate, which is why this says so.
///
/// **It cannot answer the third condition**: whether the plant could feed
/// *uneingeschränkt* in that quarter-hour is a fact about curtailment,
/// marktbedingte Anpassung and Nichtbeanspruchbarkeit that no interval carries.
/// Filter the series for that before calling, or check the returned interval.
///
/// `None` when no interval qualifies — which is a real answer: a Pauschal-
/// Abrechnung with no undisturbed quarter-hour behind it has no `P_0`, and
/// inventing one would put an invented power into a settled quantity.
///
/// The unit is **kW**. Kapitel 3.2.2.3's legend prints *"in kWh"* for `P_0`
/// while Kapitel 3.3.2's prints *"in kW"* for the same symbol; the formulas
/// settle it, since `[… − P_lim,i] × ¼h` is only dimensionally a work if every
/// term in the bracket is a power.
#[must_use]
pub fn p0_kw(series: &[MeterInterval], massnahme_beginn: OffsetDateTime) -> Option<&MeterInterval> {
    series
        .iter()
        .filter(|iv| iv.to <= massnahme_beginn && iv.quality == crate::QualityFlag::Measured)
        .max_by_key(|iv| iv.to)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interval::QualityFlag;
    use time::macros::{date, datetime};

    // ── Kapitel 3.1 ──────────────────────────────────────────────────────────

    /// The two Aufforderungsfall formulas, and the asymmetry between them.
    #[test]
    fn the_aufforderungsfall_takes_the_min_going_up_and_the_max_coming_down() {
        use Redispatchfall::Aufforderung as A;
        use Redispatchrichtung::{Negativ, Positiv};

        // Positive call: min{P_ist ; P_min}. Under-delivery is the plant's.
        assert_eq!(
            leistungslimitierung_kw(A, Positiv, dec!(80), dec!(300)),
            dec!(80)
        );
        assert_eq!(
            leistungslimitierung_kw(A, Positiv, dec!(400), dec!(300)),
            dec!(300)
        );

        // Negative call: max{P_ist ; P_max}. Over-delivery is the plant's.
        assert_eq!(
            leistungslimitierung_kw(A, Negativ, dec!(420), dec!(300)),
            dec!(420)
        );
        assert_eq!(
            leistungslimitierung_kw(A, Negativ, dec!(200), dec!(300)),
            dec!(300)
        );
    }

    /// In the Duldungsfall the operator intervened, so what happened is the
    /// limit and the instruction does not enter at all.
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

    // ── Kapitel 3.2 / 3.3 ────────────────────────────────────────────────────

    /// The worked shape of Kap. 3.2.2.3: a turbine running at 900 kW is capped
    /// at 300, so three quarters of a megawatt-quarter-hour did not happen.
    #[test]
    fn a_curtailed_turbine_loses_the_difference_over_the_quarter_hour() {
        let w = pauschal_wind_kwh(dec!(900), dec!(1000), None, None, dec!(300));
        assert_eq!(w, dec!(150.00)); // (900 − 300) × ¼ h
    }

    /// `P_inst` bounds `P_0`: a last quarter-hour above the installed power is
    /// not evidence of what the plant could have produced.
    #[test]
    fn the_installed_power_bounds_the_carried_forward_value() {
        let w = pauschal_wind_kwh(dec!(1200), dec!(1000), None, None, dec!(300));
        assert_eq!(w, dec!(175.00)); // (1000 − 300) × ¼ h
    }

    /// A marktbedingte Anpassung or a Nichtbeanspruchbarkeit lowers the
    /// theoretical power — *"ist der niedrigste Wert … maßgeblich"*.
    #[test]
    fn the_lowest_of_the_bounds_is_the_one_that_counts() {
        let w = pauschal_wind_kwh(
            dec!(900),
            dec!(1000),
            Some(dec!(600)),
            Some(dec!(700)),
            dec!(300),
        );
        assert_eq!(w, dec!(75.00)); // (600 − 300) × ¼ h
    }

    /// Never negative: Kapitel 3.2 covers the negative Redispatch only.
    #[test]
    fn a_limit_above_the_theoretical_power_is_no_ausfallarbeit() {
        assert_eq!(
            pauschal_wind_kwh(dec!(300), dec!(1000), None, None, dec!(900)),
            Decimal::ZERO
        );
    }

    /// Solar takes its theoretical power from the Anlagenfaktor, bounded by the
    /// inverter rather than by a carried-forward measurement.
    #[test]
    fn the_solar_variant_is_the_anlagenfaktor_bounded_by_the_inverter() {
        // 0,6189 × 1 000 kWp = 618,9 kW, under a 700 kW inverter.
        let w = pauschal_solar_kwh(dec!(0.6189), dec!(1000), dec!(700), None, None, dec!(100));
        assert_eq!(w, dec!(129.725000)); // (618,9 − 100) × ¼ h

        // The same array behind a 500 kW inverter cannot have produced 618,9.
        let clipped =
            pauschal_solar_kwh(dec!(0.6189), dec!(1000), dec!(500), None, None, dec!(100));
        assert_eq!(clipped, dec!(100.00)); // (500 − 100) × ¼ h
    }

    /// The Festlegung's own asymmetry: `P_bean` bounds the theoretical power
    /// for a negative call and the *limit* for a positive one. Reading it as
    /// "P_bean always caps P_0" gives a different number when both apply.
    #[test]
    fn the_nichtbeanspruchbarkeit_changes_sides_with_the_direction() {
        use Redispatchrichtung::{Negativ, Positiv};

        // Negativ: min(P_0 ; P_bean) − P_lim = min(900; 600) − 300 = 300.
        assert_eq!(
            pauschal_nicht_fluktuierend_kwh(Negativ, dec!(900), Some(dec!(600)), dec!(300)),
            dec!(75.00),
        );
        // Positiv: P_0 − min(P_lim ; P_bean) = 900 − min(1500; 600) = 300…
        // …and a positive Redispatch's Ausfallarbeit is clamped at or below 0,
        // so a plant that was already above the binding figure yields nothing.
        assert_eq!(
            pauschal_nicht_fluktuierend_kwh(Positiv, dec!(900), Some(dec!(600)), dec!(1500)),
            Decimal::ZERO,
        );
        // Genuine Mehrarbeit: pushed from 300 up to 900 kW.
        assert_eq!(
            pauschal_nicht_fluktuierend_kwh(Positiv, dec!(300), None, dec!(900)),
            dec!(-150.00),
        );
    }

    /// A positive Redispatch produces **Mehrarbeit**, carried as a negative
    /// Ausfallarbeit — the sign convention of Kapitel 3.
    #[test]
    fn a_positive_redispatch_yields_negative_ausfallarbeit() {
        use Redispatchrichtung::{Negativ, Positiv};
        assert_eq!(
            spitz_nicht_fluktuierend_kwh(Positiv, dec!(200), dec!(600)),
            dec!(-100.00),
        );
        assert_eq!(
            spitz_nicht_fluktuierend_kwh(Negativ, dec!(600), dec!(200)),
            dec!(100.00),
        );
        // Each direction clamps on the side its own sign forbids.
        assert_eq!(
            spitz_nicht_fluktuierend_kwh(Positiv, dec!(600), dec!(200)),
            Decimal::ZERO,
        );
        assert_eq!(
            spitz_nicht_fluktuierend_kwh(Negativ, dec!(200), dec!(600)),
            Decimal::ZERO,
        );
    }

    // ── the Anlagenfaktor table ──────────────────────────────────────────────

    /// Every published band, read at its own clock.
    #[test]
    fn the_anlagenfaktor_reproduces_the_published_table() {
        // Sommer, in UTC (UTC+1 = UTC + 1 h).
        let sommer = |h: i64, m: i64| {
            anlagenfaktor(datetime!(2026-06-15 0:00 UTC) + time::Duration::minutes(h * 60 + m))
        };
        assert_eq!(sommer(5, 0), dec!(0.2456)); //  6:00 MEZ
        assert_eq!(sommer(8, 0), dec!(0.6189)); //  9:00 MEZ
        assert_eq!(sommer(14, 0), dec!(0.2456)); // 15:00 MEZ
        assert_eq!(sommer(18, 0), dec!(0.0000)); // 19:00 MEZ
        assert_eq!(sommer(4, 59), dec!(0.0000)); //  5:59 MEZ

        let winter = |h: i64, m: i64| {
            anlagenfaktor(datetime!(2026-01-15 0:00 UTC) + time::Duration::minutes(h * 60 + m))
        };
        assert_eq!(winter(8, 0), dec!(0.2796)); //  9:00 MEZ
        assert_eq!(winter(9, 0), dec!(0.5030)); // 10:00 MEZ
        assert_eq!(winter(13, 0), dec!(0.2796)); // 14:00 MEZ
        assert_eq!(winter(15, 44), dec!(0.2796)); // 16:44 MEZ
        assert_eq!(winter(15, 45), dec!(0.0000)); // 16:45 MEZ
    }

    /// The table is UTC+1 year-round, so a summer instant is read an hour off
    /// the Berlin wall clock. Getting this wrong shifts every summer band.
    #[test]
    fn the_table_is_mez_not_local_time() {
        // 08:30 Europe/Berlin in June is 06:30 UTC — 07:30 MEZ, the early band.
        assert_eq!(anlagenfaktor(datetime!(2026-06-15 6:30 UTC)), dec!(0.2456));
        // Read as local time it would be 08:30, still the early band — but
        // 10:30 Berlin (08:30 UTC, 09:30 MEZ) is already the middle one.
        assert_eq!(anlagenfaktor(datetime!(2026-06-15 8:30 UTC)), dec!(0.6189));
    }

    /// The season boundary is a calendar date, so it needs no leap-year case.
    #[test]
    fn the_seasons_switch_on_the_first_of_march_and_november() {
        assert_eq!(anlagenfaktor(datetime!(2026-02-28 9:00 UTC)), dec!(0.5030)); // winter
        assert_eq!(anlagenfaktor(datetime!(2026-03-01 9:00 UTC)), dec!(0.6189)); // summer
        assert_eq!(anlagenfaktor(datetime!(2026-10-31 9:00 UTC)), dec!(0.6189)); // summer
        assert_eq!(anlagenfaktor(datetime!(2026-11-01 9:00 UTC)), dec!(0.5030)); // winter
        assert_eq!(anlagenfaktor(datetime!(2028-02-29 9:00 UTC)), dec!(0.5030)); // leap
    }

    // ── P_0 ──────────────────────────────────────────────────────────────────

    fn iv(from: OffsetDateTime, kwh: Decimal, quality: QualityFlag) -> MeterInterval {
        MeterInterval {
            from,
            to: from + time::Duration::minutes(15),
            value: kwh,
            quality,
            obis_code: None,
        }
    }

    /// The last **fully measured** quarter-hour that ends before the measure —
    /// not the last one, and not one that overlaps it.
    #[test]
    fn p0_is_the_last_billable_quarter_hour_wholly_before_the_measure() {
        let base = datetime!(2026-06-01 10:00 UTC);
        let series = vec![
            iv(base, dec!(100), QualityFlag::Measured),
            iv(
                base + time::Duration::minutes(15),
                dec!(120),
                QualityFlag::Measured,
            ),
            // A substituted value is *billable* and is not "vollständig
            // gemessen" — the distinction this filter turns on.
            iv(
                base + time::Duration::minutes(30),
                dec!(130),
                QualityFlag::Substituted,
            ),
            // Inside the measure.
            iv(
                base + time::Duration::minutes(45),
                dec!(40),
                QualityFlag::Measured,
            ),
        ];
        let start = base + time::Duration::minutes(45);
        let p0 = p0_kw(&series, start).expect("one qualifies");
        assert_eq!(p0.value, dec!(120));
        assert_eq!(p0.demand_kw(), Some(dec!(480))); // 120 kWh in a quarter-hour
    }

    /// No undisturbed quarter-hour behind the measure means no `P_0`, and the
    /// answer says so rather than inventing a power.
    #[test]
    fn p0_refuses_when_nothing_qualifies() {
        let base = datetime!(2026-06-01 10:00 UTC);
        let only_faulty = vec![iv(base, dec!(100), QualityFlag::Faulty)];
        assert!(p0_kw(&only_faulty, base + time::Duration::minutes(15)).is_none());
        assert!(p0_kw(&[], base).is_none());

        // Billable but not measured is still not a `P_0`.
        for quality in [
            QualityFlag::Substituted,
            QualityFlag::Estimated,
            QualityFlag::Calculated,
        ] {
            assert!(quality.is_billable(), "{quality} is billable");
            let series = vec![iv(base, dec!(100), quality)];
            assert!(
                p0_kw(&series, base + time::Duration::minutes(15)).is_none(),
                "{quality} must not be carried forward as P_0"
            );
        }
    }

    // ── the sunset ───────────────────────────────────────────────────────────

    /// The Pauschal-Abrechnung ends with 31.12.2028; the other two do not end.
    #[test]
    fn the_pauschal_variant_expires_and_the_others_do_not() {
        let p = Abrechnungsvariante::PauschalAbrechnung;
        assert!(p.is_available_on(date!(2028 - 12 - 31)));
        assert!(!p.is_available_on(date!(2029 - 01 - 01)));
        assert_eq!(PAUSCHAL_BESTANDSSCHUTZ_ENDE, date!(2028 - 12 - 31));

        for other in [
            Abrechnungsvariante::Spitzabrechnung,
            Abrechnungsvariante::VereinfachteSpitzabrechnung,
        ] {
            assert!(other.is_available_on(date!(2029 - 01 - 01)));
        }
    }
}
