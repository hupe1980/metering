//! German statutory holidays per Bundesland, for the SLP day type
//! ([`SlpCalendar`] → [`SlpDayType`]) and
//! [`Zaehlzeitdefinition`](crate::billing::zaehlzeit::Zaehlzeitdefinition)
//! tariff registers. Not a Fristenkalender: nothing here counts Werktage.
//!
//! Only the [`Holiday::NATIONWIDE`] holidays are common to every Land; the
//! rest are Landesrecht (Art. 70 GG), so every query takes a [`Bundesland`].
//!
//! ## Municipal scope is not modelled
//!
//! Fronleichnam in parts of Sachsen and Thüringen, and Mariä Himmelfahrt in
//! Catholic municipalities of Bayern, are statutory below Land level and are
//! reported as *not* holidays; an affected operator supplies its own day type.
//!
//! ## Rules and sources
//!
//! | Rule | Years | Source |
//! |---|---|---|
//! | Tag der Deutschen Einheit, all Länder | since 1990 | Einigungsvertrag Art. 2 Abs. 2 (BGBl. II 1990 S. 890) |
//! | Reformationstag BB, MV, SN, ST, TH | since 1990 | VO über die Einführung gesetzlicher Feiertage vom 16.5.1990 (GBl. DDR I S. 248), then the Länder laws |
//! | Reformationstag HB, HH, NI, SH | since 2018 | Brem.GBl. 2018 S. 302; HmbGVBl. 2018 S. 63; Nds. GVBl. 2018 S. 122; SH law of 21.3.2018 |
//! | Reformationstag, all Länder | 2017 only | 500th anniversary, by one-off law or ordinance in every Land (e.g. BayFTG Art. 1 Abs. 2a, GVBl. 2016 S. 50) |
//! | Frauentag BE | since 2019 | Drittes ÄndG SFG Berlin, GVBl. 2019 S. 22 |
//! | Frauentag MV | since 2023 | Viertes ÄndG FTG M-V vom 7.7.2022, GVOBl. M-V Nr. 31 |
//! | Weltkindertag TH | since 2019 | Drittes ÄndG ThürFGtG (2019, GVBl. S. 22) |
//! | Heilige Drei Könige ST | since 1993 | Feiertagsgesetz Sachsen-Anhalt (year per press sources) |
//! | Buß- und Bettag, all Länder | until 1994 | abolished from 1995 for the Pflegeversicherung (§ 58 Abs. 2 SGB XI); SN kept it. BY statewide only since 1981; the East since 1990 |
//! | Tag der Befreiung BE | 8.5.2020, 8.5.2025 | GVBl. 2019 S. 22; GVBl. 2024 S. 460 |
//! | 17. Juni BE | 17.6.2028 | GVBl. 2024 S. 460 (75th anniversary of 1953) |
//!
//! The West German 17 June holiday of 1954–1990 is not modelled. Movable
//! feasts are offsets from [`easter_sunday`].
//!
//! ```rust
//! use metering::time::holiday::{Bundesland, Holiday, SlpCalendar};
//! use metering::slp::strom::SlpDayType;
//! use time::macros::date;
//!
//! // Fronleichnam 2026 — a holiday in Bavaria, an ordinary Thursday in Berlin.
//! let fronleichnam = date!(2026 - 06 - 04);
//! assert_eq!(Bundesland::By.holiday(fronleichnam), Some(Holiday::Fronleichnam));
//! assert_eq!(Bundesland::Be.holiday(fronleichnam), None);
//!
//! assert_eq!(SlpCalendar::new(Bundesland::By).day_type(fronleichnam), SlpDayType::SonnFeiertag);
//! assert_eq!(SlpCalendar::new(Bundesland::Be).day_type(fronleichnam), SlpDayType::Werktag);
//! ```

use time::{Date, Month, Weekday};

use crate::slp::strom::SlpDayType;

/// A German federal state, identified by its ISO 3166-2:DE code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
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
    /// Every Land, in alphabetical order of name.
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

    /// The ISO 3166-2:DE code without the `DE-` prefix (`"BY"`) — the `serde`
    /// tag and [`FromStr`](std::str::FromStr) input, which also accepts `DE-BY`.
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

    /// The statutory holiday falling on `date` in this Land, if any; of two
    /// coinciding feasts, the earlier one in [`Holiday::ALL`].
    #[must_use]
    pub fn holiday(self, date: Date) -> Option<Holiday> {
        Holiday::on(date).find(|h| h.applies_in(self, date.year()))
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
            .filter(|h| h.applies_in(self, year))
            .filter_map(|h| h.date_in(year).map(|d| (d, *h)))
            .collect();
        out.sort_by_key(|(d, _)| *d);
        out
    }
}

crate::ids::codes::string_codes! {
    Bundesland, aliases = [
        ("DE-BW", Self::Bw), ("DE-BY", Self::By), ("DE-BE", Self::Be),
        ("DE-BB", Self::Bb), ("DE-HB", Self::Hb), ("DE-HH", Self::Hh),
        ("DE-HE", Self::He), ("DE-MV", Self::Mv), ("DE-NI", Self::Ni),
        ("DE-NW", Self::Nw), ("DE-RP", Self::Rp), ("DE-SL", Self::Sl),
        ("DE-SN", Self::Sn), ("DE-ST", Self::St), ("DE-SH", Self::Sh),
        ("DE-TH", Self::Th),
    ];
}

/// A German statutory holiday (gesetzlicher Feiertag).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum Holiday {
    /// Neujahr — 1 January. All Länder.
    Neujahr,
    /// Heilige Drei Könige — 6 January. BW, BY, ST.
    HeiligeDreiKoenige,
    /// Internationaler Frauentag — 8 March. BE since 2019, MV since 2023.
    Frauentag,
    /// Karfreitag — Easter − 2 days. All Länder.
    Karfreitag,
    /// Ostersonntag — Easter. Statutory in **BB only**.
    Ostersonntag,
    /// Ostermontag — Easter + 1 day. All Länder.
    Ostermontag,
    /// Tag der Arbeit — 1 May. All Länder.
    TagDerArbeit,
    /// Tag der Befreiung — 8 May. Berlin, one-off: 2020 and 2025.
    TagDerBefreiung,
    /// Christi Himmelfahrt — Easter + 39 days. All Länder.
    ChristiHimmelfahrt,
    /// Pfingstsonntag — Easter + 49 days. Statutory in **BB only**.
    Pfingstsonntag,
    /// Pfingstmontag — Easter + 50 days. All Länder.
    Pfingstmontag,
    /// Jahrestag des Volksaufstands vom 17. Juni 1953. Berlin, one-off: 2028.
    Volksaufstand,
    /// Fronleichnam — Easter + 60 days. BW, BY, HE, NW, RP, SL.
    Fronleichnam,
    /// Mariä Himmelfahrt — 15 August. SL.
    MariaeHimmelfahrt,
    /// Weltkindertag — 20 September. TH since 2019.
    Weltkindertag,
    /// Tag der Deutschen Einheit — 3 October. All Länder since 1990.
    TagDerDeutschenEinheit,
    /// Reformationstag — 31 October. BB, MV, SN, ST, TH since 1990; HB, HH,
    /// NI, SH since 2018; every Land in 2017.
    Reformationstag,
    /// Allerheiligen — 1 November. BW, BY, NW, RP, SL.
    Allerheiligen,
    /// Buß- und Bettag — the Wednesday before 23 November. SN; every Land
    /// until 1994.
    BussUndBettag,
    /// Erster Weihnachtstag — 25 December. All Länder.
    ErsterWeihnachtstag,
    /// Zweiter Weihnachtstag — 26 December. All Länder.
    ZweiterWeihnachtstag,
}

impl Holiday {
    /// Every modelled holiday, in the order it usually falls in the year.
    pub const ALL: [Self; 21] = [
        Self::Neujahr,
        Self::HeiligeDreiKoenige,
        Self::Frauentag,
        Self::Karfreitag,
        Self::Ostersonntag,
        Self::Ostermontag,
        Self::TagDerArbeit,
        Self::TagDerBefreiung,
        Self::ChristiHimmelfahrt,
        Self::Pfingstsonntag,
        Self::Pfingstmontag,
        Self::Volksaufstand,
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

    /// The holidays observed in every Bundesland today (Tag der Deutschen
    /// Einheit since 1990).
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

    /// Stable ASCII DB/wire label — the `serde` tag and
    /// [`FromStr`](std::str::FromStr) input; [`name`](Self::name) is for display.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Neujahr => "NEUJAHR",
            Self::HeiligeDreiKoenige => "HEILIGE_DREI_KOENIGE",
            Self::Frauentag => "FRAUENTAG",
            Self::Karfreitag => "KARFREITAG",
            Self::Ostersonntag => "OSTERSONNTAG",
            Self::Ostermontag => "OSTERMONTAG",
            Self::TagDerArbeit => "TAG_DER_ARBEIT",
            Self::TagDerBefreiung => "TAG_DER_BEFREIUNG",
            Self::Volksaufstand => "VOLKSAUFSTAND",
            Self::ChristiHimmelfahrt => "CHRISTI_HIMMELFAHRT",
            Self::Pfingstsonntag => "PFINGSTSONNTAG",
            Self::Pfingstmontag => "PFINGSTMONTAG",
            Self::Fronleichnam => "FRONLEICHNAM",
            Self::MariaeHimmelfahrt => "MARIAE_HIMMELFAHRT",
            Self::Weltkindertag => "WELTKINDERTAG",
            Self::TagDerDeutschenEinheit => "TAG_DER_DEUTSCHEN_EINHEIT",
            Self::Reformationstag => "REFORMATIONSTAG",
            Self::Allerheiligen => "ALLERHEILIGEN",
            Self::BussUndBettag => "BUSS_UND_BETTAG",
            Self::ErsterWeihnachtstag => "ERSTER_WEIHNACHTSTAG",
            Self::ZweiterWeihnachtstag => "ZWEITER_WEIHNACHTSTAG",
        }
    }

    /// The holiday's German name, for display.
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
            Self::TagDerBefreiung => "Tag der Befreiung",
            Self::Volksaufstand => "Jahrestag des Volksaufstandes vom 17. Juni 1953",
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

    /// The Länder this holiday is statutory in, in `year`.
    #[must_use]
    pub fn laender(self, year: i32) -> Vec<Bundesland> {
        Bundesland::ALL
            .into_iter()
            .filter(|land| self.applies_in(*land, year))
            .collect()
    }

    /// `true` when this holiday is statutory in every Bundesland in `year`.
    #[must_use]
    pub fn is_nationwide(self, year: i32) -> bool {
        Bundesland::ALL
            .into_iter()
            .all(|land| self.applies_in(land, year))
    }

    /// `true` when this holiday is statutory in `land` in `year` — by a
    /// standing rule valid that year, or by a one-off.
    #[must_use]
    pub fn applies_in(self, land: Bundesland, year: i32) -> bool {
        RULES.iter().any(|r| {
            r.holiday == self
                && r.laender.contains(&land)
                && r.since.is_none_or(|y| year >= y)
                && r.until.is_none_or(|y| year <= y)
        }) || ONE_OFFS
            .iter()
            .any(|o| o.holiday == self && o.year == year && o.laender.contains(&land))
    }

    /// The date this holiday falls on in `year`, regardless of Land (see
    /// [`applies_in`](Self::applies_in)).
    ///
    /// `None` for a one-off holiday in a year it was not declared, and when
    /// the date leaves [`Date`]'s range.
    #[must_use]
    pub fn date_in(self, year: i32) -> Option<Date> {
        let fixed = |m: Month, d: u8| Date::from_calendar_date(year, m, d).ok();
        let one_off = |m: Month, d: u8| {
            ONE_OFFS
                .iter()
                .any(|o| o.holiday == self && o.year == year)
                .then(|| fixed(m, d))
                .flatten()
        };
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
            Self::TagDerBefreiung => one_off(Month::May, 8),
            Self::Volksaufstand => one_off(Month::June, 17),
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

    /// Every holiday falling on `date` anywhere in Germany, in
    /// [`ALL`](Self::ALL) order.
    pub fn on(date: Date) -> impl Iterator<Item = Self> {
        let year = date.year();
        Self::ALL
            .into_iter()
            .filter(move |h| h.date_in(year) == Some(date))
    }
}

crate::ids::codes::string_codes! {
    Holiday;
}

/// A standing (holiday, Länder) rule with the years it is valid in.
struct Rule {
    holiday: Holiday,
    laender: &'static [Bundesland],
    since: Option<i32>,
    until: Option<i32>,
}

/// A holiday declared for one year only.
struct OneOff {
    holiday: Holiday,
    year: i32,
    laender: &'static [Bundesland],
}

const fn rule(
    holiday: Holiday,
    laender: &'static [Bundesland],
    since: Option<i32>,
    until: Option<i32>,
) -> Rule {
    Rule {
        holiday,
        laender,
        since,
        until,
    }
}

use Bundesland::{Bb, Be, Bw, By, Hb, He, Hh, Mv, Ni, Nw, Rp, Sh, Sl, Sn, St, Th};

/// Every Land.
const ALL_LAENDER: &[Bundesland] = &Bundesland::ALL;

/// The standing rules; sources in the [module docs](self#rules-and-sources).
const RULES: &[Rule] = &[
    rule(Holiday::Neujahr, ALL_LAENDER, None, None),
    rule(Holiday::HeiligeDreiKoenige, &[Bw, By], None, None),
    rule(Holiday::HeiligeDreiKoenige, &[St], Some(1993), None),
    rule(Holiday::Frauentag, &[Be], Some(2019), None),
    rule(Holiday::Frauentag, &[Mv], Some(2023), None),
    rule(Holiday::Karfreitag, ALL_LAENDER, None, None),
    rule(Holiday::Ostersonntag, &[Bb], None, None),
    rule(Holiday::Ostermontag, ALL_LAENDER, None, None),
    rule(Holiday::TagDerArbeit, ALL_LAENDER, None, None),
    rule(Holiday::ChristiHimmelfahrt, ALL_LAENDER, None, None),
    rule(Holiday::Pfingstsonntag, &[Bb], None, None),
    rule(Holiday::Pfingstmontag, ALL_LAENDER, None, None),
    rule(Holiday::Fronleichnam, &[Bw, By, He, Nw, Rp, Sl], None, None),
    rule(Holiday::MariaeHimmelfahrt, &[Sl], None, None),
    rule(Holiday::Weltkindertag, &[Th], Some(2019), None),
    rule(
        Holiday::TagDerDeutschenEinheit,
        ALL_LAENDER,
        Some(1990),
        None,
    ),
    rule(
        Holiday::Reformationstag,
        &[Bb, Mv, Sn, St, Th],
        Some(1990),
        None,
    ),
    rule(
        Holiday::Reformationstag,
        &[Hb, Hh, Ni, Sh],
        Some(2018),
        None,
    ),
    rule(Holiday::Allerheiligen, &[Bw, By, Nw, Rp, Sl], None, None),
    rule(Holiday::BussUndBettag, &[Sn], None, None),
    rule(
        Holiday::BussUndBettag,
        &[Bw, Be, Hb, Hh, He, Ni, Nw, Rp, Sl, Sh],
        None,
        Some(1994),
    ),
    rule(Holiday::BussUndBettag, &[By], Some(1981), Some(1994)),
    rule(
        Holiday::BussUndBettag,
        &[Bb, Mv, St, Th],
        Some(1990),
        Some(1994),
    ),
    rule(Holiday::ErsterWeihnachtstag, ALL_LAENDER, None, None),
    rule(Holiday::ZweiterWeihnachtstag, ALL_LAENDER, None, None),
];

/// The one-off holidays.
const ONE_OFFS: &[OneOff] = &[
    OneOff {
        holiday: Holiday::Reformationstag,
        year: 2017,
        laender: ALL_LAENDER,
    },
    OneOff {
        holiday: Holiday::TagDerBefreiung,
        year: 2020,
        laender: &[Be],
    },
    OneOff {
        holiday: Holiday::TagDerBefreiung,
        year: 2025,
        laender: &[Be],
    },
    OneOff {
        holiday: Holiday::Volksaufstand,
        year: 2028,
        laender: &[Be],
    },
];

/// The holiday calendar and day-typing policy of a standard load profile.
///
/// The calendar is the delivery point's Bundesland (*"Es gilt der
/// bundeslandspezifische Feiertagskalender nach Definition des BDEW"*, BDEW
/// Anwendungshilfe SLP 2025 p. 3). By default 24 and 31 December are typed
/// Samstag per VDEW M-32/99 p. 30: *"der 24. 12. und 31. 12. erhalten das
/// Samstagsprofil, sofern sie nicht auf einen Sonntag fallen"*; switch it off
/// with [`eves_as_saturday(false)`](Self::eves_as_saturday).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct SlpCalendar {
    land: Bundesland,
    eves_as_saturday: bool,
}

impl SlpCalendar {
    /// The calendar of `land`, with the VDEW 24/31 December rule.
    #[must_use]
    pub const fn new(land: Bundesland) -> Self {
        Self {
            land,
            eves_as_saturday: true,
        }
    }

    /// Whether 24 and 31 December (when not a Sunday) are typed Samstag.
    #[must_use]
    pub const fn eves_as_saturday(mut self, on: bool) -> Self {
        self.eves_as_saturday = on;
        self
    }

    /// The Land whose holidays apply.
    #[must_use]
    pub const fn land(&self) -> Bundesland {
        self.land
    }

    /// The SLP day type of `date`; a Sunday or statutory holiday is
    /// Sonn-/Feiertag even when it falls on a Saturday.
    #[must_use]
    pub fn day_type(&self, date: Date) -> SlpDayType {
        let eve = date.month() == Month::December && matches!(date.day(), 24 | 31);
        if date.weekday() == Weekday::Sunday || self.land.is_holiday(date) {
            SlpDayType::SonnFeiertag
        } else if date.weekday() == Weekday::Saturday || (eve && self.eves_as_saturday) {
            SlpDayType::Samstag
        } else {
            SlpDayType::Werktag
        }
    }
}

/// Easter Sunday in the Gregorian calendar — the Anonymous Gregorian algorithm
/// (Meeus/Jones/Butcher). `None` only when the result leaves [`Date`]'s range.
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

/// Buß- und Bettag — the Wednesday **before** 23 November (16–22 November).
fn buss_und_bettag(year: i32) -> Option<Date> {
    let reference = Date::from_calendar_date(year, Month::November, 23).ok()?;
    // A Wednesday 23rd steps back a full week, not zero.
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

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::date;

    /// Pinned against published dates, including both extremes of the range.
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

        let buss = date!(2026 - 11 - 18);
        assert!(Bundesland::Sn.is_holiday(buss));
        assert_eq!(
            Bundesland::ALL
                .iter()
                .filter(|l| l.is_holiday(buss))
                .count(),
            1
        );

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
            assert!(h.is_nationwide(2026), "{h}");
            let d = h.date_in(2026).expect("in range");
            for land in Bundesland::ALL {
                assert!(land.is_holiday(d), "{h} in {land}");
            }
        }
        for land in Bundesland::ALL {
            let n = land.holidays_in_year(2026).len();
            assert!(n >= 9, "{land} has only {n} holidays");
        }
        // + Heilige Drei Könige, Fronleichnam, Allerheiligen.
        assert_eq!(Bundesland::By.holidays_in_year(2026).len(), 12);
        // + Frauentag.
        assert_eq!(Bundesland::Be.holidays_in_year(2026).len(), 10);
    }

    #[test]
    fn holidays_in_year_are_sorted() {
        let list = Bundesland::By.holidays_in_year(2026);
        assert!(list.windows(2).all(|w| w[0].0 <= w[1].0));
        assert_eq!(list.first().unwrap().1, Holiday::Neujahr);
        assert_eq!(list.last().unwrap().1, Holiday::ZweiterWeihnachtstag);
    }

    #[test]
    fn day_type_ins_follow_the_land_calendar() {
        let day_type_in = |d, land| SlpCalendar::new(land).day_type(d);
        let fronleichnam = date!(2026 - 06 - 04); // Thursday
        assert_eq!(
            day_type_in(fronleichnam, Bundesland::By),
            SlpDayType::SonnFeiertag
        );
        assert_eq!(
            day_type_in(fronleichnam, Bundesland::Be),
            SlpDayType::Werktag,
            "a Berlin Fronleichnam is an ordinary Thursday"
        );
        assert_eq!(
            day_type_in(date!(2026 - 06 - 06), Bundesland::Be),
            SlpDayType::Samstag
        );
        assert_eq!(
            day_type_in(date!(2026 - 06 - 07), Bundesland::Be),
            SlpDayType::SonnFeiertag
        );
        // Mariä Himmelfahrt on a Saturday.
        assert_eq!(date!(2026 - 08 - 15).weekday(), Weekday::Saturday);
        assert_eq!(
            day_type_in(date!(2026 - 08 - 15), Bundesland::Sl),
            SlpDayType::SonnFeiertag
        );
        assert_eq!(
            day_type_in(date!(2026 - 08 - 15), Bundesland::Nw),
            SlpDayType::Samstag
        );
        assert_eq!(
            day_type_in(date!(2026 - 12 - 24), Bundesland::Be),
            SlpDayType::Samstag
        );
        assert_eq!(
            day_type_in(date!(2023 - 12 - 31), Bundesland::Be),
            SlpDayType::SonnFeiertag
        );
        assert_eq!(
            SlpCalendar::new(Bundesland::Be)
                .eves_as_saturday(false)
                .day_type(date!(2026 - 12 - 24)),
            SlpDayType::Werktag
        );
    }

    #[test]
    fn coinciding_holidays_are_both_reported() {
        let both: Vec<_> = Holiday::on(date!(2008 - 05 - 01)).collect();
        assert_eq!(
            both,
            vec![Holiday::TagDerArbeit, Holiday::ChristiHimmelfahrt]
        );
    }

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

    #[test]
    fn scope_accessors_agree() {
        for h in Holiday::ALL {
            assert!(!h.name().is_empty());
            for land in Bundesland::ALL {
                assert_eq!(
                    h.applies_in(land, 2026),
                    h.laender(2026).contains(&land),
                    "{h}/{land}"
                );
            }
            assert_eq!(
                h.is_nationwide(2026),
                Holiday::NATIONWIDE.contains(&h),
                "{h} nationwide flag"
            );
        }
    }

    #[test]
    fn rules_change_in_the_year_the_law_says() {
        use Bundesland as B;
        let check = |land: B, d: Date, expected: bool| {
            assert_eq!(land.is_holiday(d), expected, "{land} {d}");
        };
        // Tag der Deutschen Einheit since 1990.
        check(B::Nw, date!(1989 - 10 - 03), false);
        check(B::Nw, date!(1990 - 10 - 03), true);
        // Reformationstag: everywhere in 2017; the north since 2018.
        for land in B::ALL {
            check(land, date!(2017 - 10 - 31), true);
        }
        check(B::Nw, date!(2018 - 10 - 31), false);
        check(B::Ni, date!(2016 - 10 - 31), false);
        check(B::Ni, date!(2018 - 10 - 31), true);
        check(B::Hb, date!(2018 - 10 - 31), true);
        check(B::Sn, date!(2016 - 10 - 31), true);
        // Frauentag: BE since 2019, MV since 2023.
        check(B::Be, date!(2018 - 03 - 08), false);
        check(B::Be, date!(2019 - 03 - 08), true);
        check(B::Mv, date!(2022 - 03 - 08), false);
        check(B::Mv, date!(2023 - 03 - 08), true);
        // Weltkindertag TH since 2019.
        check(B::Th, date!(2018 - 09 - 20), false);
        check(B::Th, date!(2019 - 09 - 20), true);
        // Berlin one-offs.
        check(B::Be, date!(2020 - 05 - 08), true);
        check(B::Be, date!(2025 - 05 - 08), true);
        check(B::Be, date!(2026 - 05 - 08), false);
        check(B::Bb, date!(2025 - 05 - 08), false);
        check(B::Be, date!(2028 - 06 - 17), true);
        // Buß- und Bettag nationwide until 1994; Saxony since.
        check(B::Nw, date!(1994 - 11 - 16), true);
        check(B::Nw, date!(1995 - 11 - 22), false);
        check(B::Sn, date!(1995 - 11 - 22), true);
        assert_eq!(Holiday::TagDerBefreiung.date_in(2024), None);
    }
}
