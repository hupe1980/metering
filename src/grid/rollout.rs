//! iMSys rollout obligations per § 29 MsbG and the § 45 MsbG Rollout-Fahrplan.
//!
//! Start at [`classify_rollout_obligation`]. Verified against the consolidated
//! MsbG (gesetze-im-internet.de).
//!
//! ## § 29 Abs. 1 MsbG
//!
//! - **Nr. 1**: Letztverbraucher *"mit einem Jahresstromverbrauch von mehr als
//!   6 000 Kilowattstunden"*.
//! - **Nr. 2**: *"mit intelligenten Messsystemen und einer Steuerungseinrichtung
//!   am Netzanschlusspunkt"* — at **a)** Letztverbraucher with a § 14a EnWG
//!   agreement, at **b)** Anlagenbetreiber *"mit einer installierten Leistung
//!   von mehr als 7 Kilowatt"*.
//!
//! - the Steuerungseinrichtung belongs to **Nr. 2**, so both letters owe one —
//!   [`RolloutObligation::requires_steuerungseinrichtung`];
//! - Nr. 2b is **quota-conditional** —
//!   [`RolloutObligation::is_quota_conditional`];
//! - the grounds are **cumulative** (*"sowie"*), so
//!   [`classify_rollout_obligation`] returns every ground that applies.
//!
//! **Abs. 5** lifts only the Steuerungseinrichtung of a plant ([`FeedInWaiver`]).
//! **Abs. 3** owes at least a moderne Messeinrichtung wherever Abs. 1 does not
//! reach, by [`MME_DEADLINE`].

use rust_decimal::Decimal;
use time::Date;
use time::macros::date;

/// The consumption threshold of § 29 Abs. 1 Nr. 1 MsbG (kWh per year).
pub const PFLICHT_CONSUMPTION_KWH_PER_YEAR: u32 = 6_000;

/// The generation threshold of § 29 Abs. 1 Nr. 2b MsbG (kW installed).
pub const PFLICHT_GENERATION_KW: u32 = 7;

/// The § 29 Abs. 3 MsbG deadline for a moderne Messeinrichtung everywhere the
/// iMSys duty does not reach: **31 December 2032**.
///
/// *"Die Ausstattung hat bis zum Ablauf des 31. Dezember 2032, bei Neubauten
/// und Gebäuden, die einer größeren Renovierung […] unterzogen werden, bis zur
/// Fertigstellung des Gebäudes zu erfolgen."* The building case has no fixed
/// date.
pub const MME_DEADLINE: Date = date!(2032 - 12 - 31);

// ── Classification ────────────────────────────────────────────────────────────

/// One ground on which a Messstelle falls due for an intelligentes Messsystem.
///
/// The grounds are cumulative; [`RolloutAssessment`] carries every one that
/// applies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum RolloutObligation {
    /// § 29 Abs. 1 Nr. 1: annual consumption above 6 000 kWh.
    PflichtConsumption,
    /// § 29 Abs. 1 Nr. 2a: a § 14a EnWG agreement exists.
    PflichtSteuerbare14a,
    /// § 29 Abs. 1 Nr. 2b: a plant above 7 kW installed capacity.
    ///
    /// Owed only **to the extent** the § 45 Abs. 1 quotas require it
    /// ([`is_quota_conditional`](Self::is_quota_conditional)).
    PflichtGeneration,
    /// § 29 Abs. 2: an optionaler Einbaufall — an iMSys is permitted, not
    /// required; § 29 Abs. 3 still owes a moderne Messeinrichtung by
    /// [`MME_DEADLINE`].
    Optionsfall,
}

impl RolloutObligation {
    /// Every ground, in the statute's own order.
    pub const ALL: [Self; 4] = [
        Self::PflichtConsumption,
        Self::PflichtSteuerbare14a,
        Self::PflichtGeneration,
        Self::Optionsfall,
    ];

    /// Stable DB/wire label; the `serde` tag and [`FromStr`](std::str::FromStr) input.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::PflichtConsumption => "PFLICHT_CONSUMPTION",
            Self::PflichtSteuerbare14a => "PFLICHT_STEUERBARE14A",
            Self::PflichtGeneration => "PFLICHT_GENERATION",
            Self::Optionsfall => "OPTIONSFALL",
        }
    }

    /// `true` for every mandatory case of § 29 Abs. 1 MsbG.
    #[must_use]
    pub const fn is_pflichteinbaufall(self) -> bool {
        !matches!(self, Self::Optionsfall)
    }

    /// `true` when the ground also owes a Steuerungseinrichtung at the
    /// Netzanschlusspunkt.
    ///
    /// **Both** letters of § 29 Abs. 1 **Nr. 2**. The § 29 Abs. 5 waiver is
    /// applied in [`RolloutAssessment`], not here.
    #[must_use]
    pub const fn requires_steuerungseinrichtung(self) -> bool {
        matches!(self, Self::PflichtSteuerbare14a | Self::PflichtGeneration)
    }

    /// `true` when the duty applies only *"soweit dies erforderlich ist"* to
    /// meet the § 45 Abs. 1 quotas — Nr. 2b, and only Nr. 2b.
    ///
    /// Whether a given plant is due depends on the Messstellenbetreiber's
    /// portfolio progress, which only the caller knows.
    #[must_use]
    pub const fn is_quota_conditional(self) -> bool {
        matches!(self, Self::PflichtGeneration)
    }

    /// Statutory basis, for audit output.
    #[must_use]
    pub const fn legal_basis(self) -> &'static str {
        match self {
            Self::PflichtConsumption => "§ 29 Abs. 1 Nr. 1 MsbG",
            Self::PflichtSteuerbare14a => "§ 29 Abs. 1 Nr. 2 Buchst. a MsbG",
            Self::PflichtGeneration => "§ 29 Abs. 1 Nr. 2 Buchst. b MsbG",
            Self::Optionsfall => "§ 29 Abs. 2 MsbG",
        }
    }
}

// ── the § 29 Abs. 5 waiver ───────────────────────────────────────────────────

/// Whether a plant operator has taken the § 29 Abs. 5 MsbG waiver.
///
/// Applies only when **both** conditions hold: feed-in permanently limited to
/// 0 % of installed capacity, *and* a Textform declaration to the
/// grundzuständiger Messstellenbetreiber.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct FeedInWaiver {
    /// *"die maximale Wirkleistungseinspeisung dauerhaft auf 0 Prozent der
    /// installierten Leistung begrenzt"*.
    feed_in_limited_to_zero: bool,
    /// *"gegenüber dem grundzuständigen Messstellenbetreiber in Textform
    /// erklärt hat, sicherzustellen, dass seine Anlage dauerhaft keinen Strom in
    /// die Elektrizitätsversorgungsnetze einspeist"*.
    declared_in_textform: bool,
}

impl FeedInWaiver {
    /// Neither condition met — the ordinary case.
    pub const NONE: Self = Self {
        feed_in_limited_to_zero: false,
        declared_in_textform: false,
    };

    /// Both conditions met.
    pub const GRANTED: Self = Self {
        feed_in_limited_to_zero: true,
        declared_in_textform: true,
    };

    /// The two conditions as found — either may hold without the other.
    #[must_use]
    pub const fn new(feed_in_limited_to_zero: bool, declared_in_textform: bool) -> Self {
        Self {
            feed_in_limited_to_zero,
            declared_in_textform,
        }
    }

    /// Whether the maximum Wirkleistungseinspeisung is permanently limited to
    /// 0 % of installed capacity.
    #[must_use]
    pub const fn feed_in_limited_to_zero(self) -> bool {
        self.feed_in_limited_to_zero
    }

    /// Whether the operator declared in Textform that the plant never feeds in.
    #[must_use]
    pub const fn declared_in_textform(self) -> bool {
        self.declared_in_textform
    }

    /// `true` only when **both** conditions of Abs. 5 Satz 1 are met.
    #[must_use]
    pub const fn applies(self) -> bool {
        self.feed_in_limited_to_zero && self.declared_in_textform
    }
}

// ── RolloutAssessment ────────────────────────────────────────────────────────

/// What § 29 MsbG owes at one Messstelle.
///
/// Every ground that applies (cumulative, *"sowie"*), and the two duties that
/// follow from them.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub struct RolloutAssessment {
    /// Every ground that applies, in the statute's order.
    ///
    /// Exactly one element — [`Optionsfall`](RolloutObligation::Optionsfall) —
    /// when none of the Pflicht grounds do.
    pub grounds: Vec<RolloutObligation>,
    /// `true` when a Steuerungseinrichtung is owed at the Netzanschlusspunkt,
    /// after the § 29 Abs. 5 waiver has been applied.
    pub steuerungseinrichtung_required: bool,
    /// `true` when the only Pflicht ground is the quota-conditional Nr. 2b.
    pub quota_conditional: bool,
}

impl RolloutAssessment {
    /// `true` when an intelligentes Messsystem is owed at all.
    #[must_use]
    pub fn imsys_required(&self) -> bool {
        self.grounds.iter().any(|g| g.is_pflichteinbaufall())
    }

    /// `true` when this ground applies.
    #[must_use]
    pub fn has(&self, ground: RolloutObligation) -> bool {
        self.grounds.contains(&ground)
    }

    /// The ground to name when only one can be, in the statute's own order.
    ///
    /// Never `None`: an assessment always carries at least
    /// [`Optionsfall`](RolloutObligation::Optionsfall).
    #[must_use]
    pub fn primary(&self) -> RolloutObligation {
        self.grounds
            .first()
            .copied()
            .unwrap_or(RolloutObligation::Optionsfall)
    }
}

/// The Jahresstromverbrauch § 29 Abs. 1 Nr. 1 MsbG is measured by.
///
/// § 30 Abs. 4 MsbG: *"Zur Bemessung des Jahresstromverbrauchs an einem
/// Zählpunkt nach den Absätzen 1 und 3 ist der Durchschnittswert der jeweils
/// letzten drei erfassten Jahresverbrauchswerte maßgeblich. Solange noch keine
/// drei Jahreswerte nach Satz 1 vorliegen, erfolgt eine Zuordnung zur
/// Verbrauchsgruppe entsprechend der Jahresverbrauchsprognose des
/// Netzbetreibers."*
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Jahresverbrauch {
    /// The last three recorded Jahresverbrauchswerte, kWh; compared as
    /// `sum > 3 × 6 000` (an overflowing sum counts as above).
    Erfasst([Decimal; 3]),
    /// The Netzbetreiber's Jahresverbrauchsprognose, kWh, while fewer than
    /// three years are recorded.
    Prognose(Decimal),
}

impl Jahresverbrauch {
    /// `true` when the consumption is *"mehr als 6 000 Kilowattstunden"*.
    fn above(self, threshold_kwh: u32) -> bool {
        let threshold = Decimal::from(threshold_kwh);
        match self {
            Self::Erfasst(years) => years
                .iter()
                .try_fold(Decimal::ZERO, |s, y| s.checked_add(*y))
                .is_none_or(|sum| sum > threshold * Decimal::from(3u32)),
            Self::Prognose(kwh) => kwh > threshold,
        }
    }
}

/// What § 29 MsbG asks of one Messstelle.
///
/// ```rust
/// use metering::grid::rollout::{Jahresverbrauch, Messstelle, RolloutObligation, classify_rollout_obligation};
/// use rust_decimal::dec;
///
/// // A heat-pump household whose last three years average 7 000 kWh.
/// let hp = Messstelle::new(Jahresverbrauch::Erfasst([dec!(6500), dec!(7000), dec!(7500)]))
///     .vereinbarung_14a(true);
/// let a = classify_rollout_obligation(&hp);
/// assert!(a.has(RolloutObligation::PflichtConsumption));
/// assert!(a.has(RolloutObligation::PflichtSteuerbare14a));
///
/// // One heavy year is not a mean above 6 000 kWh.
/// let once = Messstelle::new(Jahresverbrauch::Erfasst([dec!(9000), dec!(4000), dec!(4000)]));
/// assert!(!classify_rollout_obligation(&once).imsys_required());
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct Messstelle {
    jahresverbrauch: Jahresverbrauch,
    installierte_leistung_kw: Option<Decimal>,
    vereinbarung_14a: bool,
    einspeiseverzicht: FeedInWaiver,
}

impl Messstelle {
    /// A Messstelle of a Letztverbraucher with no plant, no § 14a agreement
    /// and no waiver.
    #[must_use]
    pub const fn new(jahresverbrauch: Jahresverbrauch) -> Self {
        Self {
            jahresverbrauch,
            installierte_leistung_kw: None,
            vereinbarung_14a: false,
            einspeiseverzicht: FeedInWaiver::NONE,
        }
    }

    /// The installed capacity of a plant behind the Messstelle, kW.
    #[must_use]
    pub const fn installierte_leistung(mut self, kw: Decimal) -> Self {
        self.installierte_leistung_kw = Some(kw);
        self
    }

    /// Whether a Vereinbarung nach § 14a EnWG exists.
    #[must_use]
    pub const fn vereinbarung_14a(mut self, exists: bool) -> Self {
        self.vereinbarung_14a = exists;
        self
    }

    /// The § 29 Abs. 5 declaration, where one exists.
    #[must_use]
    pub const fn einspeiseverzicht(mut self, waiver: FeedInWaiver) -> Self {
        self.einspeiseverzicht = waiver;
        self
    }
}

/// Classify a Messstelle against § 29 MsbG.
///
/// Thresholds are **strict**, as the statute writes them: *"mehr als 6 000
/// Kilowattstunden"*, *"mehr als 7 Kilowatt"*.
///
/// ```rust
/// use metering::grid::rollout::{FeedInWaiver, Jahresverbrauch, Messstelle, RolloutObligation, classify_rollout_obligation};
/// use rust_decimal::dec;
///
/// // A 12 kW roof: due under Nr. 2b, but only to the extent the § 45 quotas need it.
/// let roof = Messstelle::new(Jahresverbrauch::Prognose(dec!(2000))).installierte_leistung(dec!(12));
/// let a = classify_rollout_obligation(&roof);
/// assert_eq!(a.primary(), RolloutObligation::PflichtGeneration);
/// assert!(a.quota_conditional);
/// assert!(a.steuerungseinrichtung_required);
///
/// // ...and with the Abs. 5 waiver the iMSys stays owed and the control unit does not.
/// let waived = classify_rollout_obligation(&roof.einspeiseverzicht(FeedInWaiver::GRANTED));
/// assert!(waived.imsys_required());
/// assert!(!waived.steuerungseinrichtung_required);
/// ```
#[must_use]
pub fn classify_rollout_obligation(messstelle: &Messstelle) -> RolloutAssessment {
    let mut grounds = Vec::new();
    if messstelle
        .jahresverbrauch
        .above(PFLICHT_CONSUMPTION_KWH_PER_YEAR)
    {
        grounds.push(RolloutObligation::PflichtConsumption);
    }
    if messstelle.vereinbarung_14a {
        grounds.push(RolloutObligation::PflichtSteuerbare14a);
    }
    if messstelle
        .installierte_leistung_kw
        .is_some_and(|kw| kw > Decimal::from(PFLICHT_GENERATION_KW))
    {
        grounds.push(RolloutObligation::PflichtGeneration);
    }
    if grounds.is_empty() {
        grounds.push(RolloutObligation::Optionsfall);
    }

    // Abs. 5 reaches the plant ground only, not a § 14a agreement.
    let waiver = messstelle.einspeiseverzicht.applies();
    let steuerungseinrichtung_required = grounds.iter().any(|g| {
        g.requires_steuerungseinrichtung()
            && !(*g == RolloutObligation::PflichtGeneration && waiver)
    });

    RolloutAssessment {
        quota_conditional: grounds == [RolloutObligation::PflichtGeneration],
        grounds,
        steuerungseinrichtung_required,
    }
}

// ── §45 Rollout-Fahrplan ──────────────────────────────────────────────────────

/// What a §45 Abs. 1 MsbG quota is measured against.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum QuotaScope {
    /// Share of the total stock of agendagebundene Messstellen.
    TotalStock,
    /// Share of the Messstellen newly falling due within the window.
    NewInWindow,
}

impl QuotaScope {
    /// Every scope, in declaration order.
    pub const ALL: [Self; 2] = [Self::TotalStock, Self::NewInWindow];

    /// Stable DB/wire label; the `serde` tag and [`FromStr`](std::str::FromStr) input.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::TotalStock => "TOTAL_STOCK",
            Self::NewInWindow => "NEW_IN_WINDOW",
        }
    }
}

crate::ids::codes::string_codes! {
    RolloutObligation;
    QuotaScope;
}

/// One milestone of the §45 Abs. 1 MsbG Rollout-Fahrplan.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct RolloutMilestone {
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::iso_date_option"))]
    window_from: Option<Date>,
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::iso_date"))]
    deadline: Date,
    quota_pct: u8,
    scope: QuotaScope,
}

impl RolloutMilestone {
    /// Window start (`None` for the stock quotas, which have no flow window).
    #[must_use]
    pub const fn window_from(&self) -> Option<Date> {
        self.window_from
    }

    /// Deadline by which the quota must be met.
    #[must_use]
    pub const fn deadline(&self) -> Date {
        self.deadline
    }

    /// Required share in percent.
    #[must_use]
    pub const fn quota_pct(&self) -> u8 {
        self.quota_pct
    }

    /// Stock or flow quota.
    #[must_use]
    pub const fn scope(&self) -> QuotaScope {
        self.scope
    }
}

/// The § 45 Abs. 1 **Nr. 4** MsbG milestones — the ordinary Letztverbraucher
/// schedule.
///
/// 20 % of the stock by end-2025; 90 % of the Messstellen newly falling due in
/// each of the 25.02.2025–2026, 2027/28 and 2029/30 windows; 90 % of the total
/// stock by end-2032.
///
/// Nr. 4 covers Letztverbraucher in the cases of § 30 Abs. 1 Nr. 2 to 5 and
/// § 30 Abs. 2. The other tracks of § 45 Abs. 1 are not modelled:
///
/// | Track | Population | Starts | Shape |
/// |---|---|---|---|
/// | Nr. 1 | Anlagenbetreiber, § 30 Abs. 1 Nr. 1 (the large ones) | 2028 | share of **installed capacity** newly commissioned in a window |
/// | Nr. 2 | other Anlagenbetreiber under § 30 Abs. 1 | 2025 | as Nr. 1, plus 50 % of the 2018–25.02.2025 stock by end-2028 |
/// | Nr. 3 | Letztverbraucher, § 30 Abs. 1 Nr. 1 | 2028 | share **je Einbaufallgruppe** |
///
/// Their denominators differ (kW, Einbaufallgruppe), so a caller who needs
/// them states its own table.
pub const ROLLOUT_MILESTONES: [RolloutMilestone; 5] = [
    RolloutMilestone {
        window_from: None,
        deadline: date!(2025 - 12 - 31),
        quota_pct: 20,
        scope: QuotaScope::TotalStock,
    },
    RolloutMilestone {
        window_from: Some(date!(2025 - 02 - 25)),
        deadline: date!(2026 - 12 - 31),
        quota_pct: 90,
        scope: QuotaScope::NewInWindow,
    },
    RolloutMilestone {
        window_from: Some(date!(2027 - 01 - 01)),
        deadline: date!(2028 - 12 - 31),
        quota_pct: 90,
        scope: QuotaScope::NewInWindow,
    },
    RolloutMilestone {
        window_from: Some(date!(2029 - 01 - 01)),
        deadline: date!(2030 - 12 - 31),
        quota_pct: 90,
        scope: QuotaScope::NewInWindow,
    },
    RolloutMilestone {
        window_from: None,
        deadline: date!(2032 - 12 - 31),
        quota_pct: 90,
        scope: QuotaScope::TotalStock,
    },
];

/// The milestone whose deadline is next due on `today` (or `None` after 2032).
#[must_use]
pub fn next_milestone(today: Date) -> Option<&'static RolloutMilestone> {
    ROLLOUT_MILESTONES.iter().find(|m| m.deadline >= today)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal::dec;

    fn messstelle(consumption: Decimal, kw: Option<Decimal>, steuve: bool) -> Messstelle {
        let m = Messstelle::new(Jahresverbrauch::Prognose(consumption)).vereinbarung_14a(steuve);
        match kw {
            Some(kw) => m.installierte_leistung(kw),
            None => m,
        }
    }

    fn plain(consumption: Decimal, kw: Option<Decimal>, steuve: bool) -> RolloutAssessment {
        classify_rollout_obligation(&messstelle(consumption, kw, steuve))
    }

    #[test]
    fn consumption_over_6000_kwh_is_pflicht_nr1() {
        let a = plain(dec!(6001), None, false);
        assert_eq!(a.grounds, vec![RolloutObligation::PflichtConsumption]);
        assert!(a.imsys_required());
        assert!(!a.steuerungseinrichtung_required);
        assert!(!a.quota_conditional);
    }

    #[test]
    fn exactly_6000_kwh_is_not_mandatory() {
        // § 29 Abs. 1 Nr. 1: "mehr als 6 000 Kilowattstunden" — strict.
        let a = plain(dec!(6000), None, false);
        assert_eq!(a.grounds, vec![RolloutObligation::Optionsfall]);
        assert!(!a.imsys_required());
    }

    /// The statute joins Nr. 1 and Nr. 2 with "sowie": a delivery point can be
    /// both.
    #[test]
    fn the_grounds_are_cumulative() {
        let a = plain(dec!(12000), Some(dec!(20)), true);
        assert_eq!(
            a.grounds,
            vec![
                RolloutObligation::PflichtConsumption,
                RolloutObligation::PflichtSteuerbare14a,
                RolloutObligation::PflichtGeneration,
            ]
        );
        assert_eq!(a.primary(), RolloutObligation::PflichtConsumption);
        assert!(a.steuerungseinrichtung_required);
        // Not quota-conditional: Nr. 1 and Nr. 2a are due regardless of the
        // portfolio's progress.
        assert!(!a.quota_conditional);
    }

    /// The Steuerungseinrichtung sits in Nummer 2, which covers both letters —
    /// a plant above 7 kW owes one exactly as a § 14a delivery point does.
    #[test]
    fn both_letters_of_nummer_2_owe_a_steuerungseinrichtung() {
        assert!(RolloutObligation::PflichtSteuerbare14a.requires_steuerungseinrichtung());
        assert!(RolloutObligation::PflichtGeneration.requires_steuerungseinrichtung());
        assert!(!RolloutObligation::PflichtConsumption.requires_steuerungseinrichtung());
        assert!(plain(dec!(0), Some(dec!(12)), false).steuerungseinrichtung_required);
    }

    #[test]
    fn generation_over_7_kw_is_pflicht_nr2b_with_no_upper_cap() {
        assert_eq!(
            plain(dec!(0), Some(dec!(7.1)), false).primary(),
            RolloutObligation::PflichtGeneration
        );
        // § 29 has no upper bracket.
        assert_eq!(
            plain(dec!(0), Some(dec!(950)), false).primary(),
            RolloutObligation::PflichtGeneration
        );
        assert_eq!(
            plain(dec!(0), Some(dec!(7)), false).primary(),
            RolloutObligation::Optionsfall
        );
    }

    /// Nr. 2b applies "soweit dies erforderlich ist" to meet the § 45 quotas.
    #[test]
    fn the_plant_ground_reports_its_condition() {
        assert!(RolloutObligation::PflichtGeneration.is_quota_conditional());
        assert!(!RolloutObligation::PflichtConsumption.is_quota_conditional());
        assert!(plain(dec!(0), Some(dec!(12)), false).quota_conditional);
        // ...but not when another, unconditional ground applies as well.
        assert!(!plain(dec!(9000), Some(dec!(12)), false).quota_conditional);
    }

    /// § 29 Abs. 5 lifts the Steuerungseinrichtung, and only for the plant.
    /// The iMSys itself stays owed.
    #[test]
    fn the_abs_5_waiver_lifts_only_the_control_unit() {
        let waived = classify_rollout_obligation(
            &messstelle(dec!(0), Some(dec!(12)), false).einspeiseverzicht(FeedInWaiver::GRANTED),
        );
        assert!(waived.imsys_required());
        assert!(!waived.steuerungseinrichtung_required);

        // A § 14a agreement is a Letztverbraucher fact; the plant waiver does
        // not reach it.
        let steered = classify_rollout_obligation(
            &messstelle(dec!(0), Some(dec!(12)), true).einspeiseverzicht(FeedInWaiver::GRANTED),
        );
        assert!(steered.steuerungseinrichtung_required);
    }

    /// Both conditions of Abs. 5 Satz 1 or neither: a 0 % limitation without
    /// the Textform declaration is not a waiver.
    #[test]
    fn half_a_waiver_is_no_waiver() {
        let half = FeedInWaiver::new(true, false);
        assert!(!half.applies());
        assert!(
            classify_rollout_obligation(
                &messstelle(dec!(0), Some(dec!(12)), false).einspeiseverzicht(half)
            )
            .steuerungseinrichtung_required
        );
    }

    #[test]
    fn milestones_are_ordered_and_end_2032() {
        assert!(
            ROLLOUT_MILESTONES
                .windows(2)
                .all(|w| w[0].deadline <= w[1].deadline)
        );
        assert_eq!(
            next_milestone(date!(2026 - 07 - 01)).unwrap().deadline,
            date!(2026 - 12 - 31)
        );
        assert!(next_milestone(date!(2033 - 01 - 01)).is_none());
    }

    /// The § 45 Abs. 1 Nr. 4 windows, as the statute prints them.
    #[test]
    fn the_milestone_windows_are_the_statutes_own() {
        let windows: Vec<_> = ROLLOUT_MILESTONES
            .iter()
            .map(|m| (m.window_from, m.deadline, m.quota_pct, m.scope))
            .collect();
        assert_eq!(
            windows,
            vec![
                (None, date!(2025 - 12 - 31), 20, QuotaScope::TotalStock),
                (
                    Some(date!(2025 - 02 - 25)),
                    date!(2026 - 12 - 31),
                    90,
                    QuotaScope::NewInWindow
                ),
                (
                    Some(date!(2027 - 01 - 01)),
                    date!(2028 - 12 - 31),
                    90,
                    QuotaScope::NewInWindow
                ),
                (
                    Some(date!(2029 - 01 - 01)),
                    date!(2030 - 12 - 31),
                    90,
                    QuotaScope::NewInWindow
                ),
                (None, date!(2032 - 12 - 31), 90, QuotaScope::TotalStock),
            ]
        );
    }

    /// § 30 Abs. 4: the mean of the last three recorded years decides, not
    /// one of them — and the comparison is strict on the mean.
    #[test]
    fn the_three_year_mean_decides() {
        let erfasst = |a, b, c| {
            classify_rollout_obligation(&Messstelle::new(Jahresverbrauch::Erfasst([a, b, c])))
                .has(RolloutObligation::PflichtConsumption)
        };
        assert!(
            !erfasst(dec!(6000), dec!(6000), dec!(6000)),
            "exactly 6 000 is not more"
        );
        assert!(erfasst(dec!(6000), dec!(6000), dec!(6000.01)));
        assert!(
            !erfasst(dec!(12000), dec!(3000), dec!(2999)),
            "one heavy year"
        );
        assert!(erfasst(dec!(12000), dec!(3000), dec!(3001)));
    }

    /// An Optionsfall is not "nothing is owed" — § 29 Abs. 3 still wants a
    /// moderne Messeinrichtung, by a date the crate carries.
    #[test]
    fn an_optionsfall_still_owes_a_moderne_messeinrichtung() {
        let a = plain(dec!(500), None, false);
        assert!(!a.imsys_required());
        assert_eq!(MME_DEADLINE, date!(2032 - 12 - 31));
    }
}
