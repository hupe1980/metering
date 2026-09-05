//! iMSys rollout obligations per § 29 MsbG and the § 45 MsbG Rollout-Fahrplan.
//!
//! Classification drives the WiM device-change processes and the § 45 quota
//! reporting. Verified against `specs/law/msbg.pdf`.
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
//! Three readings the wording decides, and each has cost an implementation:
//!
//! - the Steuerungseinrichtung belongs to **Nr. 2**, so both letters owe one —
//!   [`RolloutObligation::requires_steuerungseinrichtung`];
//! - Nr. 2b is **quota-conditional** —
//!   [`RolloutObligation::is_quota_conditional`];
//! - the grounds are **cumulative** (*"sowie"*), so
//!   [`classify_rollout_obligation`] returns every ground that applies.
//!
//! ## What Abs. 3 and Abs. 5 add
//!
//! **Abs. 5** lifts the Steuerungseinrichtung — and only that — where feed-in is
//! permanently limited to 0 % **and** declared in Textform ([`FeedInWaiver`]).
//! **Abs. 3** owes at least a moderne Messeinrichtung everywhere Abs. 1 does not
//! reach, by [`MME_DEADLINE`]: an Optionsfall is the *iMSys* answer, not the
//! whole one.

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
/// date, so only this one is a constant.
pub const MME_DEADLINE: Date = date!(2032 - 12 - 31);

// ── Classification ────────────────────────────────────────────────────────────

/// One ground on which a Messstelle falls due for an intelligentes Messsystem.
///
/// The grounds are cumulative — see [`RolloutAssessment`], which carries every
/// one that applies. This enum names them; it does not rank them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "SCREAMING_SNAKE_CASE"))]
pub enum RolloutObligation {
    /// § 29 Abs. 1 Nr. 1: annual consumption above 6 000 kWh.
    PflichtConsumption,
    /// § 29 Abs. 1 Nr. 2a: a § 14a EnWG agreement exists — so the Nr. 2
    /// Steuerungseinrichtung is owed as well.
    PflichtSteuerbare14a,
    /// § 29 Abs. 1 Nr. 2b: a plant above 7 kW installed capacity.
    ///
    /// Owed **to the extent** the § 45 Abs. 1 quotas require it, and with the
    /// Nr. 2 Steuerungseinrichtung unless the Abs. 5 waiver applies. See
    /// [`is_quota_conditional`](Self::is_quota_conditional).
    PflichtGeneration,
    /// § 29 Abs. 2: an optionaler Einbaufall — an iMSys is permitted, not
    /// required.
    ///
    /// Not "nothing is required": § 29 Abs. 3 still owes at least a moderne
    /// Messeinrichtung by [`MME_DEADLINE`].
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

    /// Stable DB/wire label. Matches the `serde` tag and
    /// [`FromStr`](std::str::FromStr) input.
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
    /// **Both** cases of § 29 Abs. 1 **Nr. 2**, not only the § 14a one: the
    /// Steuerungseinrichtung sits in the Nummer, which covers a) and b) alike.
    /// The § 29 Abs. 5 waiver can still remove it for a plant — that is a fact
    /// about the plant rather than about the ground, so
    /// [`RolloutAssessment`] applies it.
    #[must_use]
    pub const fn requires_steuerungseinrichtung(self) -> bool {
        matches!(self, Self::PflichtSteuerbare14a | Self::PflichtGeneration)
    }

    /// `true` when the duty applies only *"soweit dies erforderlich ist"* to
    /// meet the § 45 Abs. 1 quotas — Nr. 2b, and only Nr. 2b.
    ///
    /// Whether a particular plant is due depends on how much installed capacity
    /// the Messstellenbetreiber has already equipped in its own Netzgebiet, so
    /// no library can answer it. Reporting the condition is the honest half:
    /// a caller that treats every plant above 7 kW as immediately due will
    /// build a queue the statute does not require.
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
/// Both conditions or neither: the maximum Wirkleistungseinspeisung permanently
/// limited to **0 %** of installed capacity at the Verknüpfungspunkt, *and* a
/// declaration in Textform to the grundzuständiger Messstellenbetreiber that the
/// plant will never feed in. One without the other is not a waiver, which is why
/// this is a type rather than a `bool`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct FeedInWaiver {
    /// *"die maximale Wirkleistungseinspeisung dauerhaft auf 0 Prozent der
    /// installierten Leistung begrenzt"*.
    pub feed_in_limited_to_zero: bool,
    /// *"gegenüber dem grundzuständigen Messstellenbetreiber in Textform
    /// erklärt hat, sicherzustellen, dass seine Anlage dauerhaft keinen Strom in
    /// die Elektrizitätsversorgungsnetze einspeist"*.
    pub declared_in_textform: bool,
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

    /// `true` only when **both** conditions of Abs. 5 Satz 1 are met.
    #[must_use]
    pub const fn applies(self) -> bool {
        self.feed_in_limited_to_zero && self.declared_in_textform
    }
}

// ── RolloutAssessment ────────────────────────────────────────────────────────

/// What § 29 MsbG owes at one Messstelle.
///
/// Every ground that applies, and the two duties that follow from them. The
/// grounds are cumulative because the statute's *"sowie"* makes them so: a
/// household above 6 000 kWh **with** a § 14a agreement is Nr. 1 *and* Nr. 2a,
/// and reporting only the first would understate what is owed.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
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
    ///
    /// The Messstelle is in the Nr. 2b population; whether it is due *this*
    /// year is the Messstellenbetreiber's portfolio question.
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

/// Classify a Messstelle against § 29 MsbG.
///
/// `installed_generation_kw` is the plant's installed capacity, `waiver` the
/// § 29 Abs. 5 declaration where one exists ([`FeedInWaiver::NONE`] otherwise).
///
/// Thresholds are **strict**, as the statute writes them: *"mehr als 6 000
/// Kilowattstunden"*, *"mehr als 7 Kilowatt"*. Exactly 6 000 kWh is not a
/// Pflichteinbaufall.
///
/// ```rust
/// use metering::rollout::{FeedInWaiver, RolloutObligation, classify_rollout_obligation};
/// use rust_decimal::dec;
///
/// // A heat-pump household that also draws a lot: two grounds, not one.
/// let both = classify_rollout_obligation(dec!(9000), None, true, FeedInWaiver::NONE);
/// assert!(both.has(RolloutObligation::PflichtConsumption));
/// assert!(both.has(RolloutObligation::PflichtSteuerbare14a));
/// assert!(both.steuerungseinrichtung_required);
///
/// // A 12 kW roof: due under Nr. 2b, but only to the extent the § 45 quotas need it.
/// let roof = classify_rollout_obligation(dec!(2000), Some(dec!(12)), false, FeedInWaiver::NONE);
/// assert_eq!(roof.primary(), RolloutObligation::PflichtGeneration);
/// assert!(roof.quota_conditional);
/// assert!(roof.steuerungseinrichtung_required);
///
/// // ...and with the Abs. 5 waiver, the iMSys stays owed and the control unit does not.
/// let never_feeds_in =
///     classify_rollout_obligation(dec!(2000), Some(dec!(12)), false, FeedInWaiver::GRANTED);
/// assert!(never_feeds_in.imsys_required());
/// assert!(!never_feeds_in.steuerungseinrichtung_required);
/// ```
#[must_use]
pub fn classify_rollout_obligation(
    annual_consumption_kwh: Decimal,
    installed_generation_kw: Option<Decimal>,
    has_14a_agreement: bool,
    waiver: FeedInWaiver,
) -> RolloutAssessment {
    let mut grounds = Vec::new();
    if annual_consumption_kwh > Decimal::from(PFLICHT_CONSUMPTION_KWH_PER_YEAR) {
        grounds.push(RolloutObligation::PflichtConsumption);
    }
    if has_14a_agreement {
        grounds.push(RolloutObligation::PflichtSteuerbare14a);
    }
    let is_plant =
        installed_generation_kw.is_some_and(|kw| kw > Decimal::from(PFLICHT_GENERATION_KW));
    if is_plant {
        grounds.push(RolloutObligation::PflichtGeneration);
    }
    if grounds.is_empty() {
        grounds.push(RolloutObligation::Optionsfall);
    }

    // Abs. 5 lifts the Steuerungseinrichtung for the *plant* ground only. A
    // § 14a agreement is a Letztverbraucher fact and the waiver does not reach
    // it — a delivery point that steers a heat pump still needs the control
    // unit however its roof is wired.
    let steuerungseinrichtung_required = grounds.iter().any(|g| {
        g.requires_steuerungseinrichtung()
            && !(*g == RolloutObligation::PflichtGeneration && waiver.applies())
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
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "SCREAMING_SNAKE_CASE"))]
pub enum QuotaScope {
    /// Share of the total stock of agendagebundene Messstellen.
    TotalStock,
    /// Share of the Messstellen newly falling due within the window.
    NewInWindow,
}

impl QuotaScope {
    /// Every scope, in declaration order.
    pub const ALL: [Self; 2] = [Self::TotalStock, Self::NewInWindow];

    /// Stable DB/wire label. Matches the `serde` tag and
    /// [`FromStr`](std::str::FromStr) input.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::TotalStock => "TOTAL_STOCK",
            Self::NewInWindow => "NEW_IN_WINDOW",
        }
    }
}

crate::codes::string_codes! {
    RolloutObligation;
    QuotaScope;
}

/// One milestone of the §45 Abs. 1 MsbG Rollout-Fahrplan.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct RolloutMilestone {
    /// Window start (None for the stock quotas, which have no flow window).
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::iso_date_option"))]
    pub window_from: Option<Date>,
    /// Deadline by which the quota must be met.
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::iso_date"))]
    pub deadline: Date,
    /// Required share in percent.
    pub quota_pct: u8,
    /// Stock or flow quota.
    pub scope: QuotaScope,
}

/// The § 45 Abs. 1 **Nr. 4** MsbG milestones — the ordinary Letztverbraucher
/// schedule.
///
/// 20 % of the stock by end-2025; 90 % of the Messstellen newly falling due in
/// each of the 25.02.2025–2026, 2027/28 and 2029/30 windows; 90 % of the total
/// stock by end-2032.
///
/// **§ 45 Abs. 1 has four schedules, and this is one of them.** Nr. 4 covers
/// Letztverbraucher in the cases of § 30 Abs. 1 Nr. 2 to 5 and § 30 Abs. 2 —
/// which is the bulk of a grundzuständiger Messstellenbetreiber's portfolio.
/// The other three are deliberately not modelled here:
///
/// | Track | Population | Starts | Shape |
/// |---|---|---|---|
/// | Nr. 1 | Anlagenbetreiber, § 30 Abs. 1 Nr. 1 (the large ones) | 2028 | share of **installed capacity** newly commissioned in a window |
/// | Nr. 2 | other Anlagenbetreiber under § 30 Abs. 1 | 2025 | as Nr. 1, plus 50 % of the 2018–25.02.2025 stock by end-2028 |
/// | Nr. 3 | Letztverbraucher, § 30 Abs. 1 Nr. 1 | 2028 | share **je Einbaufallgruppe** |
///
/// Nr. 1 and Nr. 2 are measured in **kilowatts**, not in Messstellen, and Nr. 3
/// splits by Einbaufallgruppe. Modelling them behind the same
/// [`RolloutMilestone`] type would make four different denominators look like
/// one; a caller who needs them states its own table.
///
/// Verified against `specs/law/msbg.pdf`.
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

    fn plain(consumption: Decimal, kw: Option<Decimal>, steuve: bool) -> RolloutAssessment {
        classify_rollout_obligation(consumption, kw, steuve, FeedInWaiver::NONE)
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

    /// The statute joins Nr. 1 and Nr. 2 with "sowie". A delivery point can be
    /// both, and an assessment that reported only the first would understate
    /// what is owed at it.
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
        // No upper bracket exists in the current § 29.
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
    /// A library cannot know a portfolio's progress, so it reports the
    /// condition instead of pretending the plant is due today.
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
        let waived =
            classify_rollout_obligation(dec!(0), Some(dec!(12)), false, FeedInWaiver::GRANTED);
        assert!(waived.imsys_required());
        assert!(!waived.steuerungseinrichtung_required);

        // A § 14a agreement is a Letztverbraucher fact; the plant waiver does
        // not reach it.
        let steered =
            classify_rollout_obligation(dec!(0), Some(dec!(12)), true, FeedInWaiver::GRANTED);
        assert!(steered.steuerungseinrichtung_required);
    }

    /// Both conditions of Abs. 5 Satz 1 or neither: a 0 % limitation without
    /// the Textform declaration is not a waiver.
    #[test]
    fn half_a_waiver_is_no_waiver() {
        let half = FeedInWaiver {
            feed_in_limited_to_zero: true,
            declared_in_textform: false,
        };
        assert!(!half.applies());
        assert!(
            classify_rollout_obligation(dec!(0), Some(dec!(12)), false, half)
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

    /// An Optionsfall is not "nothing is owed" — § 29 Abs. 3 still wants a
    /// moderne Messeinrichtung, by a date the crate carries.
    #[test]
    fn an_optionsfall_still_owes_a_moderne_messeinrichtung() {
        let a = plain(dec!(500), None, false);
        assert!(!a.imsys_required());
        assert_eq!(MME_DEADLINE, date!(2032 - 12 - 31));
    }
}
