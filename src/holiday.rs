//! German statutory holidays, for day-type classification.
//!
//! ## Scope — and what is deliberately not here
//!
//! This module exists for exactly one reason: **two calculations in this crate
//! cannot be completed without a holiday calendar.**
//!
//! - [`crate::load_profile::SlpDayType`] keys every BDEW standard load profile
//!   table on Werktag / Samstag / Sonn-Feiertag. Without a holiday calendar the
//!   crate defines the key but cannot produce it.
//! - [`crate::zaehlzeit::Zaehlzeitdefinition`] resolves a timestamp to a tariff
//!   register by weekday and time band, and German definitions put statutory
//!   holidays on the off-peak register.
//!
//! It is **not** a Fristenkalender. Counting Werktage for a GPKE deadline, and
//! the EDI@Energy rule that a holiday in one Bundesland counts nationwide, are
//! market-*communication* concerns that belong in a process engine, not in a
//! library that computes kWh. Nothing here counts business days.
//!
//! ## Holidays are Land law
//!
//! Only nine holidays are common to all sixteen Länder ([`Holiday::NATIONWIDE`]);
//! the rest are set by Landesrecht under Art. 70 GG. So the calendar of a
//! delivery point is the calendar of the Land it sits in, and
//! [`slp_day_type`] takes that Land as an argument rather than assuming one.
//!
//! ### Municipal scope is not modelled
//!
//! Fronleichnam in parts of Sachsen and Thüringen, and Mariä Himmelfahrt in the
//! predominantly Catholic municipalities of Bayern, are statutory below Land
//! level. [`Bundesland`] has no finer resolution, so those are reported as
//! *not* holidays. An operator billing in an affected municipality supplies its
//! own day type.
//!
//! ## No table, no I/O
//!
//! Every date is computed: Easter by the Anonymous Gregorian algorithm
//! (Meeus/Jones/Butcher), the movable feasts as fixed offsets from it, and
//! Buß- und Bettag from the weekday of 23 November. There is no embedded year
//! table to run out.
//!
//! ## Example
//!
//! ```rust
//! use metering::holiday::{Bundesland, Holiday, slp_day_type};
//! use metering::load_profile::SlpDayType;
//! use time::macros::date;
//!
//! // Fronleichnam 2026 — a holiday in Bavaria, an ordinary Thursday in Berlin.
//! let fronleichnam = date!(2026 - 06 - 04);
//! assert_eq!(Bundesland::By.holiday(fronleichnam), Some(Holiday::Fronleichnam));
//! assert_eq!(Bundesland::Be.holiday(fronleichnam), None);
//!
//! // ...so the two Länder read different rows out of the same profile table.
//! assert_eq!(slp_day_type(fronleichnam, Bundesland::By), SlpDayType::SonnFeiertag);
//! assert_eq!(slp_day_type(fronleichnam, Bundesland::Be), SlpDayType::Werktag);
//! ```

use std::fmt;
use std::str::FromStr;

use time::{Date, Month, Weekday};

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use crate::error::ParseError;
use crate::load_profile::SlpDayType;

// ── Bundesland ────────────────────────────────────────────────────────────────

/// A German federal state, identified by its ISO 3166-2:DE code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "UPPERCASE"))]
pub enum Bundesland {
    /// Baden-Württemberg.
    Bw,
    /// Bayern.
    By,
    /// Berlin.
    Be,
    /// Brandenburg.
    Bb,
    /// Bremen.
    Hb,
    /// Hamburg.
    Hh,
    /// Hessen.
    He,
    /// Mecklenburg-Vorpommern.
    Mv,
    /// Niedersachsen.
    Ni,
    /// Nordrhein-Westfalen.
    Nw,
    /// Rheinland-Pfalz.
    Rp,
    /// Saarland.
    Sl,
    /// Sachsen.
    Sn,
    /// Sachsen-Anhalt.
    St,
    /// Schleswig-Holstein.
    Sh,
    /// Thüringen.
    Th,
}

impl Bundesland {
    /// Every Land, in ISO-code order.
    pub const ALL: [Self; 16] = [
        Self::Bw,
        Self::By,
        Self::Be,
        Self::Bb,
        Self::Hb,
        Self::Hh,
        Self::He,
        Self::Mv,
        Self::Ni,
        Self::Nw,
        Self::Rp,
        Self::Sl,
        Self::Sn,
        Self::St,
        Self::Sh,
        Self::Th,
    ];

    /// The ISO 3166-2:DE subdivision code without the `DE-` prefix — `"BY"`.
    ///
    /// This is the `serde` tag and the [`FromStr`] input.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Bw => "BW",
            Self::By => "BY",
            Self::Be => "BE",
            Self::Bb => "BB",
            Self::Hb => "HB",
            Self::Hh => "HH",
            Self::He => "HE",
            Self::Mv => "MV",
            Self::Ni => "NI",
            Self::Nw => "NW",
            Self::Rp => "RP",
            Self::Sl => "SL",
            Self::Sn => "SN",
            Self::St => "ST",
            Self::Sh => "SH",
            Self::Th => "TH",
        }
    }

    /// The accepted [`FromStr`] codes, in the same order as [`ALL`](Self::ALL).
    pub const CODES: &'static [&'static str] = &[
        "BW", "BY", "BE", "BB", "HB", "HH", "HE", "MV", "NI", "NW", "RP", "SL", "SN", "ST", "SH",
        "TH",
    ];

    /// The Land's full German name.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Bw => "Baden-Württemberg",
            Self::By => "Bayern",
            Self::Be => "Berlin",
            Self::Bb => "Brandenburg",
            Self::Hb => "Bremen",
            Self::Hh => "Hamburg",
            Self::He => "Hessen",
            Self::Mv => "Mecklenburg-Vorpommern",
            Self::Ni => "Niedersachsen",
            Self::Nw => "Nordrhein-Westfalen",
            Self::Rp => "Rheinland-Pfalz",
            Self::Sl => "Saarland",
            Self::Sn => "Sachsen",
            Self::St => "Sachsen-Anhalt",
            Self::Sh => "Schleswig-Holstein",
            Self::Th => "Thüringen",
        }
    }

    /// The statutory holiday falling on `date` in this Land, if any.
    ///
    /// When two feasts coincide — 1 May 2008 was both Tag der Arbeit and
    /// Christi Himmelfahrt — the earlier one in [`Holiday::ALL`] is returned.
    #[must_use]
    pub fn holiday(self, date: Date) -> Option<Holiday> {
        Holiday::on(date).find(|h| h.applies_in(self))
    }

    /// `true` when `date` is a statutory holiday in this Land.
    #[must_use]
    pub fn is_holiday(self, date: Date) -> bool {
        self.holiday(date).is_some()
    }

    /// Every statutory holiday in this Land in `year`, in date order.
    #[must_use]
    pub fn holidays_in_year(self, year: i32) -> Vec<(Date, Holiday)> {
        let mut out: Vec<(Date, Holiday)> = Holiday::ALL
            .iter()
            .filter(|h| h.applies_in(self))
            .filter_map(|h| h.date_in(year).map(|d| (d, *h)))
            .collect();
        out.sort_by_key(|(d, _)| *d);
        out
    }
}

impl fmt::Display for Bundesland {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Bundesland {
    type Err = ParseError;

    /// Parses the ISO code with or without the `DE-` prefix, case-insensitively.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let t = s.trim().to_uppercase();
        let code = t.strip_prefix("DE-").unwrap_or(&t);
        Self::ALL
            .iter()
            .copied()
            .find(|b| b.as_str() == code)
            .ok_or_else(|| ParseError::one_of("Bundesland", s, Self::CODES))
    }
}

// ── Holiday ───────────────────────────────────────────────────────────────────

/// A German statutory holiday (gesetzlicher Feiertag).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "SCREAMING_SNAKE_CASE"))]
pub enum Holiday {
    /// Neujahr — 1 January. All Länder.
    Neujahr,
    /// Heilige Drei Könige — 6 January. BW, BY, ST.
    HeiligeDreiKoenige,
    /// Internationaler Frauentag — 8 March. BE, MV.
    Frauentag,
    /// Karfreitag — Easter − 2 days. All Länder.
    Karfreitag,
    /// Ostersonntag — Easter. Statutory in **BB only**; elsewhere it is simply
    /// a Sunday, which the day-type classification already treats as such.
    Ostersonntag,
    /// Ostermontag — Easter + 1 day. All Länder.
    Ostermontag,
    /// Tag der Arbeit — 1 May. All Länder.
    TagDerArbeit,
    /// Christi Himmelfahrt — Easter + 39 days. All Länder.
    ChristiHimmelfahrt,
    /// Pfingstsonntag — Easter + 49 days. Statutory in **BB only**.
    Pfingstsonntag,
    /// Pfingstmontag — Easter + 50 days. All Länder.
    Pfingstmontag,
    /// Fronleichnam — Easter + 60 days. BW, BY, HE, NW, RP, SL.
    ///
    /// Also municipal in parts of SN and TH — not modelled, see the
    /// [module docs](self#municipal-scope-is-not-modelled).
    Fronleichnam,
    /// Mariä Himmelfahrt — 15 August. SL.
    ///
    /// Also municipal in the predominantly Catholic parts of BY — not modelled.
    MariaeHimmelfahrt,
    /// Weltkindertag — 20 September. TH.
    Weltkindertag,
    /// Tag der Deutschen Einheit — 3 October. All Länder.
    TagDerDeutschenEinheit,
    /// Reformationstag — 31 October. BB, HB, HH, MV, NI, SH, SN, ST, TH.
    Reformationstag,
    /// Allerheiligen — 1 November. BW, BY, NW, RP, SL.
    Allerheiligen,
    /// Buß- und Bettag — the Wednesday before 23 November. SN.
    BussUndBettag,
    /// Erster Weihnachtstag — 25 December. All Länder.
    ErsterWeihnachtstag,
    /// Zweiter Weihnachtstag — 26 December. All Länder.
    ZweiterWeihnachtstag,
}

impl Holiday {
    /// Every modelled holiday, in the order it falls in the year.
    pub const ALL: [Self; 19] = [
        Self::Neujahr,
        Self::HeiligeDreiKoenige,
        Self::Frauentag,
        Self::Karfreitag,
        Self::Ostersonntag,
        Self::Ostermontag,
        Self::TagDerArbeit,
        Self::ChristiHimmelfahrt,
        Self::Pfingstsonntag,
        Self::Pfingstmontag,
        Self::Fronleichnam,
        Self::MariaeHimmelfahrt,
        Self::Weltkindertag,
        Self::TagDerDeutschenEinheit,
        Self::Reformationstag,
        Self::Allerheiligen,
        Self::BussUndBettag,
        Self::ErsterWeihnachtstag,
        Self::ZweiterWeihnachtstag,
    ];

    /// The nine holidays observed in every Bundesland.
    pub const NATIONWIDE: [Self; 9] = [
        Self::Neujahr,
        Self::Karfreitag,
        Self::Ostermontag,
        Self::TagDerArbeit,
        Self::ChristiHimmelfahrt,
        Self::Pfingstmontag,
        Self::TagDerDeutschenEinheit,
        Self::ErsterWeihnachtstag,
        Self::ZweiterWeihnachtstag,
    ];

    /// The holiday's German name.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Neujahr => "Neujahr",
            Self::HeiligeDreiKoenige => "Heilige Drei Könige",
            Self::Frauentag => "Internationaler Frauentag",
            Self::Karfreitag => "Karfreitag",
            Self::Ostersonntag => "Ostersonntag",
            Self::Ostermontag => "Ostermontag",
            Self::TagDerArbeit => "Tag der Arbeit",
            Self::ChristiHimmelfahrt => "Christi Himmelfahrt",
            Self::Pfingstsonntag => "Pfingstsonntag",
            Self::Pfingstmontag => "Pfingstmontag",
            Self::Fronleichnam => "Fronleichnam",
            Self::MariaeHimmelfahrt => "Mariä Himmelfahrt",
            Self::Weltkindertag => "Weltkindertag",
            Self::TagDerDeutschenEinheit => "Tag der Deutschen Einheit",
            Self::Reformationstag => "Reformationstag",
            Self::Allerheiligen => "Allerheiligen",
            Self::BussUndBettag => "Buß- und Bettag",
            Self::ErsterWeihnachtstag => "Erster Weihnachtstag",
            Self::ZweiterWeihnachtstag => "Zweiter Weihnachtstag",
        }
    }

    /// The Länder this holiday is statutory in.
    #[must_use]
    pub const fn laender(self) -> &'static [Bundesland] {
        use Bundesland as B;
        match self {
            Self::Neujahr
            | Self::Karfreitag
            | Self::Ostermontag
            | Self::TagDerArbeit
            | Self::ChristiHimmelfahrt
            | Self::Pfingstmontag
            | Self::TagDerDeutschenEinheit
            | Self::ErsterWeihnachtstag
            | Self::ZweiterWeihnachtstag => &B::ALL,
            Self::HeiligeDreiKoenige => &[B::Bw, B::By, B::St],
            Self::Frauentag => &[B::Be, B::Mv],
            Self::Ostersonntag | Self::Pfingstsonntag => &[B::Bb],
            Self::Fronleichnam => &[B::Bw, B::By, B::He, B::Nw, B::Rp, B::Sl],
            Self::MariaeHimmelfahrt => &[B::Sl],
            Self::Weltkindertag => &[B::Th],
            Self::Reformationstag => &[
                B::Bb,
                B::Hb,
                B::Hh,
                B::Mv,
                B::Ni,
                B::Sh,
                B::Sn,
                B::St,
                B::Th,
            ],
            Self::Allerheiligen => &[B::Bw, B::By, B::Nw, B::Rp, B::Sl],
            Self::BussUndBettag => &[B::Sn],
        }
    }

    /// `true` when this holiday is statutory in every Bundesland.
    #[must_use]
    pub fn is_nationwide(self) -> bool {
        self.laender().len() == Bundesland::ALL.len()
    }

    /// `true` when this holiday is statutory in `land`.
    #[must_use]
    pub fn applies_in(self, land: Bundesland) -> bool {
        self.laender().contains(&land)
    }

    /// The date this holiday falls on in `year`.
    ///
    /// `None` only for a year whose arithmetic leaves [`Date`]'s range.
    #[must_use]
    pub fn date_in(self, year: i32) -> Option<Date> {
        let fixed = |m: Month, d: u8| Date::from_calendar_date(year, m, d).ok();
        let from_easter =
            |offset: i64| easter_sunday(year)?.checked_add(time::Duration::days(offset));
        match self {
            Self::Neujahr => fixed(Month::January, 1),
            Self::HeiligeDreiKoenige => fixed(Month::January, 6),
            Self::Frauentag => fixed(Month::March, 8),
            Self::Karfreitag => from_easter(-2),
            Self::Ostersonntag => easter_sunday(year),
            Self::Ostermontag => from_easter(1),
            Self::TagDerArbeit => fixed(Month::May, 1),
            Self::ChristiHimmelfahrt => from_easter(39),
            Self::Pfingstsonntag => from_easter(49),
            Self::Pfingstmontag => from_easter(50),
            Self::Fronleichnam => from_easter(60),
            Self::MariaeHimmelfahrt => fixed(Month::August, 15),
            Self::Weltkindertag => fixed(Month::September, 20),
            Self::TagDerDeutschenEinheit => fixed(Month::October, 3),
            Self::Reformationstag => fixed(Month::October, 31),
            Self::Allerheiligen => fixed(Month::November, 1),
            Self::BussUndBettag => buss_und_bettag(year),
            Self::ErsterWeihnachtstag => fixed(Month::December, 25),
            Self::ZweiterWeihnachtstag => fixed(Month::December, 26),
        }
    }

    /// Every holiday falling on `date` anywhere in Germany, in [`ALL`] order.
    ///
    /// More than one can coincide: 1 May 2008 was both Tag der Arbeit and
    /// Christi Himmelfahrt.
    ///
    /// [`ALL`]: Self::ALL
    pub fn on(date: Date) -> impl Iterator<Item = Self> {
        let year = date.year();
        Self::ALL
            .into_iter()
            .filter(move |h| h.date_in(year) == Some(date))
    }
}

impl fmt::Display for Holiday {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

// ── SLP day type ──────────────────────────────────────────────────────────────

/// The BDEW Standardlastprofil day type for `date` in `land`.
///
/// The 1999 VDEW profiles and the 2025 revision both key their tables on these
/// three day types, and the holiday calendar that decides between them is the
/// **delivery point's Bundesland**.
///
/// A statutory holiday outranks the weekday, so a Fronleichnam Thursday in
/// Bavaria reads the Sonn-/Feiertag row while the same Thursday in Berlin reads
/// the Werktag row.
#[must_use]
pub fn slp_day_type(date: Date, land: Bundesland) -> SlpDayType {
    if date.weekday() == Weekday::Sunday || land.is_holiday(date) {
        SlpDayType::SonnFeiertag
    } else if date.weekday() == Weekday::Saturday {
        SlpDayType::Samstag
    } else {
        SlpDayType::Werktag
    }
}

// ── date arithmetic ───────────────────────────────────────────────────────────

/// Easter Sunday in the Gregorian calendar — the Anonymous Gregorian algorithm
/// (Meeus/Jones/Butcher).
///
/// Ten of the nineteen holidays are offsets from this date. `None` only when
/// the result leaves [`Date`]'s range.
#[must_use]
pub fn easter_sunday(year: i32) -> Option<Date> {
    let a = year.rem_euclid(19);
    let b = year.div_euclid(100);
    let c = year.rem_euclid(100);
    let d = b.div_euclid(4);
    let e = b.rem_euclid(4);
    let f = (b + 8).div_euclid(25);
    let g = (b - f + 1).div_euclid(3);
    let h = (19 * a + b - d - g + 15).rem_euclid(30);
    let i = c.div_euclid(4);
    let k = c.rem_euclid(4);
    let l = (32 + 2 * e + 2 * i - h - k).rem_euclid(7);
    let m = (a + 11 * h + 22 * l).div_euclid(451);
    let month = (h + l - 7 * m + 114).div_euclid(31); // 3 = March, 4 = April
    let day = (h + l - 7 * m + 114).rem_euclid(31) + 1;
    let month = Month::try_from(u8::try_from(month).ok()?).ok()?;
    Date::from_calendar_date(year, month, u8::try_from(day).ok()?).ok()
}

/// Buß- und Bettag — the Wednesday **before** 23 November.
///
/// Equivalently the Wednesday falling in 16–22 November. Never 23 November
/// itself, even in a year when the 23rd is a Wednesday.
fn buss_und_bettag(year: i32) -> Option<Date> {
    let reference = Date::from_calendar_date(year, Month::November, 23).ok()?;
    // Days to step back to the *preceding* Wednesday. When the 23rd is itself a
    // Wednesday the answer is a full week, not zero.
    let back = match reference.weekday() {
        Weekday::Wednesday => 7,
        Weekday::Thursday => 1,
        Weekday::Friday => 2,
        Weekday::Saturday => 3,
        Weekday::Sunday => 4,
        Weekday::Monday => 5,
        Weekday::Tuesday => 6,
    };
    reference.checked_sub(time::Duration::days(back))
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::date;

    /// Easter anchors ten of the nineteen holidays, so it is pinned against
    /// published dates including both extremes of its range.
    #[test]
    fn easter_matches_the_published_dates() {
        let cases = [
            (2020, date!(2020 - 04 - 12)),
            (2021, date!(2021 - 04 - 04)),
            (2022, date!(2022 - 04 - 17)),
            (2023, date!(2023 - 04 - 09)),
            (2024, date!(2024 - 03 - 31)),
            (2025, date!(2025 - 04 - 20)),
            (2026, date!(2026 - 04 - 05)),
            (2027, date!(2027 - 03 - 28)),
            (2028, date!(2028 - 04 - 16)),
            (2030, date!(2030 - 04 - 21)),
            (2038, date!(2038 - 04 - 25)), // the latest date Easter can take
            (2285, date!(2285 - 03 - 22)), // the earliest
        ];
        for (year, expected) in cases {
            assert_eq!(easter_sunday(year), Some(expected), "Easter {year}");
        }
    }

    /// Easter is always a Sunday in 22 March – 25 April; a wrong branch in the
    /// algorithm shows up here rather than in one sampled year.
    #[test]
    fn easter_is_always_a_sunday_in_range() {
        for year in 1900..2200 {
            let e = easter_sunday(year).expect("in range");
            assert_eq!(e.weekday(), Weekday::Sunday, "{year}");
            assert!(
                e >= Date::from_calendar_date(year, Month::March, 22).unwrap()
                    && e <= Date::from_calendar_date(year, Month::April, 25).unwrap(),
                "{year}: {e}"
            );
        }
    }

    #[test]
    fn movable_feasts_2026() {
        assert_eq!(
            Holiday::Karfreitag.date_in(2026),
            Some(date!(2026 - 04 - 03))
        );
        assert_eq!(
            Holiday::Ostermontag.date_in(2026),
            Some(date!(2026 - 04 - 06))
        );
        assert_eq!(
            Holiday::ChristiHimmelfahrt.date_in(2026),
            Some(date!(2026 - 05 - 14))
        );
        assert_eq!(
            Holiday::Pfingstmontag.date_in(2026),
            Some(date!(2026 - 05 - 25))
        );
        assert_eq!(
            Holiday::Fronleichnam.date_in(2026),
            Some(date!(2026 - 06 - 04))
        );
    }

    /// Buß- und Bettag is the Wednesday *before* the 23rd — when the 23rd is
    /// itself a Wednesday the holiday is the 16th, not the 23rd.
    #[test]
    fn buss_und_bettag_is_the_wednesday_before_the_23rd() {
        let cases = [
            (2022, date!(2022 - 11 - 16)), // 23 Nov 2022 was a Wednesday
            (2023, date!(2023 - 11 - 22)),
            (2024, date!(2024 - 11 - 20)),
            (2025, date!(2025 - 11 - 19)),
            (2026, date!(2026 - 11 - 18)),
            (2027, date!(2027 - 11 - 17)),
            (2028, date!(2028 - 11 - 22)),
        ];
        for (year, expected) in cases {
            let d = Holiday::BussUndBettag.date_in(year).expect("in range");
            assert_eq!(d, expected, "Buß- und Bettag {year}");
            assert_eq!(d.weekday(), Weekday::Wednesday);
            assert!((16..=22).contains(&d.day()), "{d} must fall in 16–22 Nov");
        }
    }

    /// The reason this module takes a Land: the same Thursday is a holiday in
    /// one Land and a working day in the next.
    #[test]
    fn regional_holidays_are_regional() {
        let fronleichnam = date!(2026 - 06 - 04);
        for land in [
            Bundesland::Bw,
            Bundesland::By,
            Bundesland::He,
            Bundesland::Nw,
            Bundesland::Rp,
            Bundesland::Sl,
        ] {
            assert!(
                land.is_holiday(fronleichnam),
                "{land} observes Fronleichnam"
            );
        }
        for land in [Bundesland::Be, Bundesland::Hh, Bundesland::Ni] {
            assert!(!land.is_holiday(fronleichnam), "{land} does not");
        }

        // Buß- und Bettag is Saxony alone.
        let buss = date!(2026 - 11 - 18);
        assert!(Bundesland::Sn.is_holiday(buss));
        assert_eq!(
            Bundesland::ALL
                .iter()
                .filter(|l| l.is_holiday(buss))
                .count(),
            1
        );

        // Reformationstag is nine Länder — neither one nor sixteen.
        let reformation = date!(2026 - 10 - 31);
        assert_eq!(
            Bundesland::ALL
                .iter()
                .filter(|l| l.is_holiday(reformation))
                .count(),
            9
        );
    }

    #[test]
    fn the_nationwide_nine_hold_everywhere() {
        for h in Holiday::NATIONWIDE {
            assert!(h.is_nationwide(), "{h}");
            let d = h.date_in(2026).expect("in range");
            for land in Bundesland::ALL {
                assert!(land.is_holiday(d), "{h} in {land}");
            }
        }
        for land in Bundesland::ALL {
            let n = land.holidays_in_year(2026).len();
            assert!(n >= 9, "{land} has only {n} holidays");
        }
        // Bavaria: the nine plus Heilige Drei Könige, Fronleichnam and
        // Allerheiligen.
        assert_eq!(Bundesland::By.holidays_in_year(2026).len(), 12);
        // Berlin: the nine plus Frauentag.
        assert_eq!(Bundesland::Be.holidays_in_year(2026).len(), 10);
    }

    /// A Land's holiday list must come back in date order — callers render it.
    #[test]
    fn holidays_in_year_are_sorted() {
        let list = Bundesland::By.holidays_in_year(2026);
        assert!(list.windows(2).all(|w| w[0].0 <= w[1].0));
        assert_eq!(list.first().unwrap().1, Holiday::Neujahr);
        assert_eq!(list.last().unwrap().1, Holiday::ZweiterWeihnachtstag);
    }

    /// Day typing is what this module exists for.
    #[test]
    fn slp_day_types_follow_the_land_calendar() {
        let fronleichnam = date!(2026 - 06 - 04); // Thursday
        assert_eq!(
            slp_day_type(fronleichnam, Bundesland::By),
            SlpDayType::SonnFeiertag
        );
        assert_eq!(
            slp_day_type(fronleichnam, Bundesland::Be),
            SlpDayType::Werktag,
            "a Berlin Fronleichnam is an ordinary Thursday"
        );
        assert_eq!(
            slp_day_type(date!(2026 - 06 - 06), Bundesland::Be),
            SlpDayType::Samstag
        );
        assert_eq!(
            slp_day_type(date!(2026 - 06 - 07), Bundesland::Be),
            SlpDayType::SonnFeiertag
        );
        // A holiday landing on a Saturday is still Sonn-/Feiertag, not Samstag:
        // 2026-08-15 (Mariä Himmelfahrt) is a Saturday.
        assert_eq!(date!(2026 - 08 - 15).weekday(), Weekday::Saturday);
        assert_eq!(
            slp_day_type(date!(2026 - 08 - 15), Bundesland::Sl),
            SlpDayType::SonnFeiertag
        );
        assert_eq!(
            slp_day_type(date!(2026 - 08 - 15), Bundesland::Nw),
            SlpDayType::Samstag
        );
        // 24 December is not a statutory holiday in any Land — the market
        // communication rule that treats it as one is a Fristen concern and
        // deliberately absent here.
        assert_eq!(
            slp_day_type(date!(2026 - 12 - 24), Bundesland::Be),
            SlpDayType::Werktag
        );
    }

    #[test]
    fn coinciding_holidays_are_both_reported() {
        // 1 May 2008: Tag der Arbeit and Christi Himmelfahrt.
        let both: Vec<_> = Holiday::on(date!(2008 - 05 - 01)).collect();
        assert_eq!(
            both,
            vec![Holiday::TagDerArbeit, Holiday::ChristiHimmelfahrt]
        );
    }

    /// Codes round-trip and are unique — the same contract every other string
    /// form in this crate holds to.
    #[test]
    fn bundesland_codes_round_trip() {
        assert_eq!(Bundesland::ALL.len(), Bundesland::CODES.len());
        for (land, code) in Bundesland::ALL.iter().zip(Bundesland::CODES) {
            assert_eq!(land.as_str(), *code);
            assert_eq!(&land.to_string().parse::<Bundesland>().unwrap(), land);
            assert!(!land.name().is_empty());
        }
        let unique: std::collections::BTreeSet<_> = Bundesland::CODES.iter().collect();
        assert_eq!(unique.len(), Bundesland::CODES.len());

        assert_eq!("de-by".parse::<Bundesland>().unwrap(), Bundesland::By);
        assert_eq!("  ni ".parse::<Bundesland>().unwrap(), Bundesland::Ni);
        assert!("XX".parse::<Bundesland>().is_err());
    }

    /// `laender()`, `applies_in()` and `is_nationwide()` must not drift apart.
    #[test]
    fn scope_accessors_agree() {
        for h in Holiday::ALL {
            assert!(!h.name().is_empty());
            assert!(h.date_in(2026).is_some(), "{h}");
            for land in Bundesland::ALL {
                assert_eq!(
                    h.applies_in(land),
                    h.laender().contains(&land),
                    "{h}/{land}"
                );
            }
            assert_eq!(
                h.is_nationwide(),
                Holiday::NATIONWIDE.contains(&h),
                "{h} nationwide flag"
            );
        }
    }
}
