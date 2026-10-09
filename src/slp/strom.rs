//! German Standard Load Profiles for **electricity** (Standardlastprofile Strom).
//! Gas profiles (a temperature-driven daily function) are in [`crate::slp::gas`].
//!
//! ## Legal basis
//!
//! - **VDEW Repräsentative Lastprofile** (1999): the original profiles.
//! - **BDEW "Hinweise zu den aktualisierten Standardlastprofilen Strom"**
//!   (17.03.2025): the 2025 revision. Its use is **voluntary** — *"Jedem
//!   Netzbetreiber steht es weiterhin frei, bei der Bilanzierung auf die
//!   aktualisierten Profile aus dem Jahr 2025, die alten Profile aus dem Jahr
//!   1999, eigene Profile oder eine Mischung der verschiedenen Optionen
//!   zurückzugreifen."*
//! - **GPKE / MaBiS**, in the consolidated Lesefassung of BNetzA **BK6-24-174**:
//!   SLP Marktlokationen are balanced and billed against a profile.
//!
//! ## Profile families
//!
//! | Family | Usage |
//! |---|---|
//! | H0 | Residential households |
//! | G0–G6 | Commercial, various sub-types |
//! | L0–L2 | Agricultural (Landwirtschaft) |
//! | H25/G25/L25/P25/S25 | The 2025 revision |
//!
//! Start here: [`DynamicSlpProfile::value_at`].

use std::collections::BTreeMap;

use rust_decimal::{Decimal, dec};
use time::OffsetDateTime;

use crate::precision::{
    DYNAMIZATION_DP, DYNAMIZATION_STRATEGY, DYNAMIZED_VALUE_DP, DYNAMIZED_VALUE_STRATEGY,
};
use crate::time::holiday::SlpCalendar;
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// A German electricity Standard Load Profile.
///
/// The 1999 VDEW generation and the 2025 revision. Gas profiles are
/// [`GasProfile`](crate::slp::gas::GasProfile).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum LoadProfile {
    /// H0 — Haushalt (residential household).
    H0,
    /// G0 — Gewerbe allgemein (general commercial).
    G0,
    /// G1 — Gewerbe werktags 8–18 Uhr.
    G1,
    /// G2 — Gewerbe mit starkem bis überwiegendem Verbrauch in den
    /// Abendstunden.
    G2,
    /// G3 — Gewerbe durchlaufend.
    G3,
    /// G4 — Laden/Friseur.
    G4,
    /// G5 — Bäckerei mit Backstube.
    G5,
    /// G6 — Wochenendbetrieb.
    G6,
    /// L0 — Landwirtschaftsbetriebe allgemein.
    L0,
    /// L1 — Landwirtschaftsbetriebe mit Milchwirtschaft/Nebenerwerbs-Tierzucht.
    L1,
    /// L2 — übrige Landwirtschaftsbetriebe.
    L2,
    /// H25 — aktualisiertes Haushaltsprofil (successor to H0). Entdynamisiert.
    H25,
    /// G25 — aktualisiertes Gewerbeprofil (successor to G0–G6). No Dynamisierung.
    G25,
    /// L25 — aktualisiertes Landwirtschaftsprofil (successor to L0–L2). No Dynamisierung.
    L25,
    /// P25 — Kombinationsprofil PV (household with PV). Entdynamisiert.
    P25,
    /// S25 — Kombinationsprofil PV + Speicher. Entdynamisiert.
    S25,
}

impl LoadProfile {
    /// Every variant, in declaration order.
    pub const ALL: [Self; 16] = [
        Self::H0,
        Self::G0,
        Self::G1,
        Self::G2,
        Self::G3,
        Self::G4,
        Self::G5,
        Self::G6,
        Self::L0,
        Self::L1,
        Self::L2,
        Self::H25,
        Self::G25,
        Self::L25,
        Self::P25,
        Self::S25,
    ];

    /// The canonical BDEW profile identifier.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::H0 => "H0",
            Self::G0 => "G0",
            Self::G1 => "G1",
            Self::G2 => "G2",
            Self::G3 => "G3",
            Self::G4 => "G4",
            Self::G5 => "G5",
            Self::G6 => "G6",
            Self::L0 => "L0",
            Self::L1 => "L1",
            Self::L2 => "L2",
            Self::H25 => "H25",
            Self::G25 => "G25",
            Self::L25 => "L25",
            Self::P25 => "P25",
            Self::S25 => "S25",
        }
    }

    /// `true` for the household profiles (H0, H25, P25, S25).
    #[must_use]
    pub const fn is_residential(self) -> bool {
        matches!(self, Self::H0 | Self::H25 | Self::P25 | Self::S25)
    }

    /// `true` for the commercial profiles (G0–G6, G25).
    #[must_use]
    pub const fn is_commercial(self) -> bool {
        matches!(
            self,
            Self::G0 | Self::G1 | Self::G2 | Self::G3 | Self::G4 | Self::G5 | Self::G6 | Self::G25
        )
    }

    /// `true` for the agricultural profiles (L0–L2, L25).
    #[must_use]
    pub const fn is_agricultural(self) -> bool {
        matches!(self, Self::L0 | Self::L1 | Self::L2 | Self::L25)
    }

    /// `true` for profiles delivered "entdynamisiert", to which the
    /// Dynamisierungsfunktion must be applied (H0, H25, P25, S25).
    ///
    /// G25 and L25 carry none, verbatim: *"Das Profil enthält keine
    /// Dynamisierung und die Dynamisierungsfunktion ist hier nicht
    /// anzuwenden."* Of the 1999 profiles only H0 is dynamised.
    ///
    /// Source: BDEW *Hinweise zu den aktualisierten Standardlastprofilen
    /// Strom*, 17.03.2025, §§ 2.1–2.5.
    #[must_use]
    pub const fn requires_dynamization(self) -> bool {
        matches!(self, Self::H0 | Self::H25 | Self::P25 | Self::S25)
    }
}

// ── Dynamisierung ─────────────────────────────────────────────────────────────

/// A Dynamisierungsfunktion — a quartic in the day of the year.
///
/// ```text
/// f(t) = a·t⁴ + b·t³ + c·t² + d·t + e      t = day of year, 1 = 1 January
/// ```
///
/// Rounding per BDEW *Hinweise zu den aktualisierten Standardlastprofilen Strom*
/// (17.03.2025), §2.1, verbatim: *"Eine Rundung der Dynamisierungsfaktoren auf
/// vier Nachkommastellen wird empfohlen. Das Ergebnis wird auf drei
/// Nachkommastellen gerundet."* — factors to four decimal places, the dynamized
/// value to three, as [`factor`](Self::factor) and [`apply`](Self::apply) do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct Dynamization {
    /// Quartic coefficients `(a, b, c, d, e)`, highest power first.
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::decimal_array"))]
    pub coefficients: [Decimal; 5],
}

impl Dynamization {
    /// The BDEW Dynamisierungsfunktion of the H0/H25 profiles:
    ///
    /// ```text
    /// F(t) = −3,92E-10·t⁴ + 3,20E-7·t³ − 7,02E-5·t² + 2,10E-3·t + 1,24
    /// ```
    ///
    /// Source: BDEW *Hinweise zu den aktualisierten Standardlastprofilen
    /// Strom* (17.03.2025) p. 4, identical to VDEW M-32/99 (1999) p. 33. Check
    /// values (VDEW M-05/2000, p. 19–20): F(1) = 1,2420, F(202) = 0,7847,
    /// F(365) = 1,2572, F(366) = 1,2597.
    pub const BDEW: Self = Self {
        coefficients: [
            dec!(-0.000000000392),
            dec!(0.00000032),
            dec!(-0.0000702),
            dec!(0.0021),
            dec!(1.24),
        ],
    };

    /// Dynamization factor for `day_of_year`, rounded to 4 decimal places.
    ///
    /// `None` outside **1..=366** (the quartic is fitted to one year) or on
    /// overflow of a supplied polynomial.
    ///
    /// ```rust
    /// use metering::slp::strom::Dynamization;
    ///
    /// let d = Dynamization::BDEW;
    /// assert!(d.factor(1).is_some());
    /// assert!(d.factor(366).is_some());
    /// assert_eq!(d.factor(0), None, "day numbers are 1-based");
    /// assert_eq!(d.factor(367), None, "no year is that long");
    /// ```
    #[must_use]
    pub fn factor(&self, day_of_year: u16) -> Option<Decimal> {
        if !(1..=366).contains(&day_of_year) {
            return None;
        }
        let t = Decimal::from(day_of_year);
        // Horner, checked: a supplied polynomial can overflow where the
        // published one cannot, and refusing beats a fallback factor of 1.
        let f = self
            .coefficients
            .iter()
            .try_fold(Decimal::ZERO, |acc, c| acc.checked_mul(t)?.checked_add(*c))?;
        Some(f.round_dp_with_strategy(DYNAMIZATION_DP, DYNAMIZATION_STRATEGY))
    }

    /// Apply the factor to a profile value, cut to [`DYNAMIZED_VALUE_DP`].
    ///
    /// `None` for a `day_of_year` [`factor`](Self::factor) refuses.
    #[must_use]
    pub fn apply(&self, profile_value: Decimal, day_of_year: u16) -> Option<Decimal> {
        Some(
            profile_value
                .checked_mul(self.factor(day_of_year)?)?
                .round_dp_with_strategy(DYNAMIZED_VALUE_DP, DYNAMIZED_VALUE_STRATEGY),
        )
    }
}

// ── day types ─────────────────────────────────────────────────────────────────

/// Day types (Typtage) of the BDEW profiles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum SlpDayType {
    /// Werktag (Mon–Fri, not a public holiday).
    Werktag,
    /// Samstag (not a public holiday).
    Samstag,
    /// Sonn- und Feiertag (Bundesland-specific holiday calendar).
    SonnFeiertag,
}

impl SlpDayType {
    /// Every day type, in declaration order.
    pub const ALL: [Self; 3] = [Self::Werktag, Self::Samstag, Self::SonnFeiertag];

    /// Stable DB/wire label. Matches the `serde` tag and
    /// [`FromStr`](std::str::FromStr) input.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Werktag => "WERKTAG",
            Self::Samstag => "SAMSTAG",
            Self::SonnFeiertag => "SONN_FEIERTAG",
        }
    }
}

crate::ids::codes::string_codes! {
    SlpDayType;
    LoadProfile;
}

// ── the value table ───────────────────────────────────────────────────────────

/// Quarter-hours in one day table — the BDEW workbook's `00:00-00:15` …
/// `23:45-24:00` rows.
pub const QUARTERS_PER_DAY_TABLE: usize = 96;

/// Why a day table was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum SlpTableError {
    /// The month is not 1..=12.
    #[error("month {0} is not 1..=12")]
    Month(u8),
    /// The day table does not hold exactly 96 quarter-hour values.
    #[error("a day table holds 96 quarter-hour values, got {0}")]
    Length(usize),
}

/// A profile value table, keyed by `(month, day type)`.
///
/// BDEW *Hinweise zu den aktualisierten Standardlastprofilen Strom*
/// (17.03.2025), §1, states the shape verbatim:
///
/// > Alle neuen Profile arbeiten nun mit **zwölf Monaten (Saisons)**. […] Alle
/// > neuen Profile arbeiten mit **drei Typtagen**: Werktage (WT), Samstage (SA)
/// > sowie Sonn- und Feiertage (FT). […] Es gilt der **bundeslandspezifische
/// > Feiertagskalender** nach Definition des BDEW. […] Alle Profile sind auf
/// > **1 Mio. kWh** Jahresverbrauchsmenge normiert.
///
/// So the table is 12 × 3 × 96 values, with the day type resolved against the
/// delivery point's Bundesland ([`SlpCalendar`]). No value tables are
/// embedded: the operator loads the set it bills on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DynamicSlpProfile {
    profile: LoadProfile,
    values: BTreeMap<(u8, SlpDayType), Vec<Decimal>>,
    dynamization: Option<Dynamization>,
}

impl DynamicSlpProfile {
    /// An empty table for `profile`.
    ///
    /// A profile that [`requires_dynamization`](LoadProfile::requires_dynamization)
    /// answers nothing until a [`dynamization`](Self::dynamization) is set: an
    /// entdynamisiert value is not a load-profile value.
    #[must_use]
    pub const fn new(profile: LoadProfile) -> Self {
        Self {
            profile,
            values: BTreeMap::new(),
            dynamization: None,
        }
    }

    /// The Dynamisierungsfunktion that came with the table (builder style).
    /// Ignored by profiles that carry none (G25, L25, G0–G6, L0–L2).
    #[must_use]
    pub const fn dynamization(mut self, function: Dynamization) -> Self {
        self.dynamization = Some(function);
        self
    }

    /// Load the day table of `month` (1..=12) and `day_type`: the 96
    /// quarter-hour values in wall-clock order, `00:00-00:15` first.
    ///
    /// # Errors
    ///
    /// [`SlpTableError`] for a month outside 1..=12 or a table that is not 96
    /// values long.
    pub fn insert(
        &mut self,
        month: u8,
        day_type: SlpDayType,
        values: Vec<Decimal>,
    ) -> Result<(), SlpTableError> {
        if !(1..=12).contains(&month) {
            return Err(SlpTableError::Month(month));
        }
        if values.len() != QUARTERS_PER_DAY_TABLE {
            return Err(SlpTableError::Length(values.len()));
        }
        self.values.insert((month, day_type), values);
        Ok(())
    }

    /// The profile the table belongs to.
    #[must_use]
    pub const fn profile(&self) -> LoadProfile {
        self.profile
    }

    /// `true` when all 12 × 3 day tables are present.
    #[must_use]
    pub fn is_complete(&self) -> bool {
        (1u8..=12).all(|m| {
            SlpDayType::ALL
                .iter()
                .all(|dt| self.values.contains_key(&(m, *dt)))
        })
    }

    /// The profile value of the quarter-hour containing `instant`, with the
    /// Dynamisierungsfunktion applied where the profile requires it.
    ///
    /// The instant is read on the **Berlin wall clock** (the workbook rows are
    /// local time), which fixes the month, the day type under `calendar`, the
    /// day of the year and the quarter-hour row. The Anwendungshilfe states no
    /// DST rule, so on the 23-hour day rows `02:00`–`02:45` are never read and
    /// on the 25-hour day both passes through `02:xx` read the same rows.
    ///
    /// `None` when the day table is not loaded, when the profile needs a
    /// Dynamisierung and none was supplied, or outside the supported years.
    ///
    /// ```rust
    /// use metering::slp::strom::{DynamicSlpProfile, Dynamization, LoadProfile, SlpDayType};
    /// use metering::time::holiday::{Bundesland, SlpCalendar};
    /// use rust_decimal::dec;
    /// use time::macros::datetime;
    ///
    /// let mut h25 = DynamicSlpProfile::new(LoadProfile::H25).dynamization(Dynamization::BDEW);
    /// h25.insert(6, SlpDayType::SonnFeiertag, vec![dec!(100); 96])?;
    ///
    /// // Fronleichnam 2026 is a Thursday — but a Sonn-/Feiertag in Bavaria.
    /// let noon = datetime!(2026-06-04 10:00 UTC);
    /// assert!(h25.value_at(noon, &SlpCalendar::new(Bundesland::By)).is_some());
    /// // In Berlin the same date is a Werktag, and that table is not loaded.
    /// assert!(h25.value_at(noon, &SlpCalendar::new(Bundesland::Be)).is_none());
    /// # Ok::<(), metering::slp::strom::SlpTableError>(())
    /// ```
    #[must_use]
    pub fn value_at(&self, instant: OffsetDateTime, calendar: &SlpCalendar) -> Option<Decimal> {
        let date = crate::DayBoundary::Strom.day_of(instant)?;
        let local = crate::time::calendar::to_berlin(instant);
        let quarter = usize::from(local.hour()) * 4 + usize::from(local.minute()) / 15;
        let raw = self
            .values
            .get(&(u8::from(date.month()), calendar.day_type(date)))?
            .get(quarter)
            .copied()?;
        if self.profile.requires_dynamization() {
            self.dynamization?.apply(raw, date.ordinal())
        } else {
            Some(raw)
        }
    }
}

/// The `serde` form: a list of `{ month, day_type, values }` records (a tuple
/// key is not a JSON object key).
#[cfg(feature = "serde")]
#[derive(Serialize, Deserialize)]
struct RawProfile {
    profile: LoadProfile,
    dynamization: Option<Dynamization>,
    days: Vec<RawDay>,
}

#[cfg(feature = "serde")]
#[derive(Serialize, Deserialize)]
struct RawDay {
    month: u8,
    day_type: SlpDayType,
    #[serde(with = "crate::wire::decimal_vec")]
    values: Vec<Decimal>,
}

#[cfg(feature = "serde")]
impl Serialize for DynamicSlpProfile {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        RawProfile {
            profile: self.profile,
            dynamization: self.dynamization,
            days: self
                .values
                .iter()
                .map(|(&(month, day_type), values)| RawDay {
                    month,
                    day_type,
                    values: values.clone(),
                })
                .collect(),
        }
        .serialize(serializer)
    }
}

/// Reads through [`DynamicSlpProfile::insert`], so a short day table or a
/// thirteenth month is refused on the way in.
#[cfg(feature = "serde")]
impl<'de> Deserialize<'de> for DynamicSlpProfile {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = RawProfile::deserialize(deserializer)?;
        let mut table = Self::new(raw.profile);
        table.dynamization = raw.dynamization;
        for day in raw.days {
            table
                .insert(day.month, day.day_type, day.values)
                .map_err(serde::de::Error::custom)?;
        }
        Ok(table)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::time::holiday::Bundesland;
    use time::macros::datetime;

    fn flat(
        profile: LoadProfile,
        month: u8,
        day_type: SlpDayType,
        v: Decimal,
    ) -> DynamicSlpProfile {
        let mut t = DynamicSlpProfile::new(profile).dynamization(Dynamization::BDEW);
        t.insert(month, day_type, vec![v; 96]).unwrap();
        t
    }

    #[test]
    fn every_profile_round_trips_its_code() {
        for p in LoadProfile::ALL {
            assert_eq!(p.as_str().parse::<LoadProfile>().ok(), Some(p));
        }
        assert_eq!(" h0 ".parse::<LoadProfile>().ok(), Some(LoadProfile::H0));
    }

    /// The enum is electricity-only: a gas code is not an electricity profile.
    #[test]
    fn gas_codes_are_not_electricity_profiles() {
        for gas in ["HEF", "HMF", "GKO", "GHD", "EF", "CUSTOM"] {
            assert!(gas.parse::<LoadProfile>().is_err(), "{gas}");
        }
    }

    #[test]
    fn classification() {
        assert!(LoadProfile::H0.is_residential());
        assert!(LoadProfile::G25.is_commercial());
        assert!(LoadProfile::L25.is_agricultural());
        assert!(!LoadProfile::H0.is_commercial());
        for (p, dynamized) in [
            (LoadProfile::H25, true),
            (LoadProfile::G25, false),
            (LoadProfile::L25, false),
            (LoadProfile::P25, true),
            (LoadProfile::S25, true),
            (LoadProfile::H0, true),
            (LoadProfile::G0, false),
        ] {
            assert_eq!(p.requires_dynamization(), dynamized, "{p}");
        }
    }

    /// The VDEW step-by-step check values (M-05/2000, p. 19–20).
    #[test]
    fn dynamization_reproduces_the_published_check_values() {
        let d = Dynamization::BDEW;
        assert_eq!(d.factor(1), Some(dec!(1.2420)));
        assert_eq!(d.factor(2), Some(dec!(1.2439)));
        assert_eq!(d.factor(202), Some(dec!(0.7847)));
        assert_eq!(d.factor(365), Some(dec!(1.2572)));
        assert_eq!(d.factor(366), Some(dec!(1.2597)));
        // 01.01.2000, Wi-Sonntag 0:15: 87,5 W × 1,2420 = 108,675.
        assert_eq!(d.apply(dec!(87.5), 1), Some(dec!(108.675)));
        assert_eq!(d.factor(0), None);
        assert_eq!(d.factor(367), None);
    }

    #[test]
    fn a_day_table_is_validated() {
        let mut t = DynamicSlpProfile::new(LoadProfile::G25);
        assert_eq!(
            t.insert(13, SlpDayType::Werktag, vec![dec!(1); 96]),
            Err(SlpTableError::Month(13))
        );
        assert_eq!(
            t.insert(1, SlpDayType::Werktag, vec![dec!(1); 95]),
            Err(SlpTableError::Length(95))
        );
        assert!(!t.is_complete());
    }

    /// The instant is read on the Berlin wall clock: 23:00 UTC on a winter
    /// Monday is Tuesday 00:00 local — the first row of the next day.
    #[test]
    fn the_lookup_reads_the_berlin_wall_clock_quarter_hour() {
        let mut g25 = DynamicSlpProfile::new(LoadProfile::G25);
        let mut rows: Vec<Decimal> = (0..96).map(Decimal::from).collect();
        g25.insert(1, SlpDayType::Werktag, rows.clone()).unwrap();
        rows.reverse();
        g25.insert(7, SlpDayType::Werktag, rows).unwrap();
        let cal = SlpCalendar::new(Bundesland::Be);

        // Tue 2026-01-06 00:00 CET = Mon 23:00 UTC → row 0.
        assert_eq!(
            g25.value_at(datetime!(2026-01-05 23:00 UTC), &cal),
            Some(dec!(0))
        );
        // 12:14 CET → row 48.
        assert_eq!(
            g25.value_at(datetime!(2026-01-06 11:14 UTC), &cal),
            Some(dec!(48))
        );
        // Summer: 12:00 CEST = 10:00 UTC → row 48 of the reversed July table.
        assert_eq!(
            g25.value_at(datetime!(2026-07-07 10:00 UTC), &cal),
            Some(dec!(47))
        );
    }

    /// On the 25-hour day both passes through 02:00–02:59 read the same rows;
    /// on the 23-hour day the 02:xx rows are never read.
    #[test]
    fn dst_days_follow_the_wall_clock() {
        let mut g25 = DynamicSlpProfile::new(LoadProfile::G25);
        let rows: Vec<Decimal> = (0..96).map(Decimal::from).collect();
        g25.insert(10, SlpDayType::SonnFeiertag, rows.clone())
            .unwrap();
        g25.insert(3, SlpDayType::SonnFeiertag, rows).unwrap();
        let cal = SlpCalendar::new(Bundesland::Be);

        // 25.10.2026: 02:00 CEST = 00:00 UTC, 02:00 CET = 01:00 UTC.
        let first = g25.value_at(datetime!(2026-10-25 0:00 UTC), &cal);
        let second = g25.value_at(datetime!(2026-10-25 1:00 UTC), &cal);
        assert_eq!(first, Some(dec!(8)));
        assert_eq!(first, second);

        // 29.03.2026: 01:45 CET = 00:45 UTC is row 7, the next quarter-hour
        // (01:00 UTC) is 03:00 CEST — row 12. Rows 8–11 do not occur.
        assert_eq!(
            g25.value_at(datetime!(2026-03-29 0:45 UTC), &cal),
            Some(dec!(7))
        );
        assert_eq!(
            g25.value_at(datetime!(2026-03-29 1:00 UTC), &cal),
            Some(dec!(12))
        );
    }

    /// Fronleichnam is a Feiertag only in some Länder.
    #[test]
    fn the_lookup_uses_the_bundesland_calendar() {
        let h25 = flat(LoadProfile::H25, 6, SlpDayType::SonnFeiertag, dec!(100));
        let noon = datetime!(2026-06-04 10:00 UTC);
        assert!(
            h25.value_at(noon, &SlpCalendar::new(Bundesland::By))
                .is_some()
        );
        assert!(
            h25.value_at(noon, &SlpCalendar::new(Bundesland::Be))
                .is_none()
        );
    }

    /// An entdynamisiert value is not a load-profile value: without a
    /// function the lookup refuses.
    #[test]
    fn a_profile_needing_dynamization_refuses_without_a_function() {
        let mut h25 = DynamicSlpProfile::new(LoadProfile::H25);
        h25.insert(1, SlpDayType::Werktag, vec![dec!(100); 96])
            .unwrap();
        let cal = SlpCalendar::new(Bundesland::Be);
        let at = datetime!(2026-01-15 10:00 UTC);
        assert_eq!(h25.value_at(at, &cal), None);
        let h25 = h25.dynamization(Dynamization::BDEW);
        // 15 January: factor 1,2568 → 125,680.
        assert_eq!(h25.value_at(at, &cal), Some(dec!(125.680)));

        // G25 carries no Dynamisierung and answers raw.
        let g25 = flat(LoadProfile::G25, 1, SlpDayType::Werktag, dec!(100));
        assert_eq!(g25.value_at(at, &cal), Some(dec!(100)));
    }

    #[cfg(feature = "serde")]
    #[test]
    fn a_profile_table_round_trips_through_json() {
        let table = flat(LoadProfile::H25, 6, SlpDayType::Werktag, dec!(1.5));
        let json = serde_json::to_string(&table).unwrap();
        assert!(json.contains("\"day_type\":\"WERKTAG\""), "{json}");
        let back: DynamicSlpProfile = serde_json::from_str(&json).unwrap();
        assert_eq!(back, table);
        // A short day table is refused on the way in.
        let short = json.replacen("\"1.5\",", "", 1);
        assert!(serde_json::from_str::<DynamicSlpProfile>(&short).is_err());
    }
}
