//! Zählzeitdefinition — resolving a timestamp to a tariff register.
//!
//! ## One mechanism, not two
//!
//! Every time-of-use split in the German market is the same shape: ordered
//! windows over (months × day group × time band), each naming a register, with
//! a fallback for the times no window covers. That covers
//!
//! - the classic **Zweitarif** — HT by day, NT for the rest
//!   ([`Zaehlzeitdefinition::ht_nt`]),
//! - **§ 14a EnWG Modul 3** time-variable Netzentgelte — HT, NT and a standard
//!   band for everything else ([`Zaehlzeitdefinition::modul_3`]),
//! - and any Zählzeitdefinition a Netzbetreiber transmits over UTILTS, with as
//!   many registers as it likes.
//!
//! One mechanism rather than a dedicated two-register type: Modul 3 has
//! **three** tariff levels and every Netzbetreiber has had to offer it since
//! 1 April 2025, and Netzbetreiber bands routinely start on the half hour.
//!
//! ## Resolution is DST-correct and Feiertag-aware
//!
//! Timestamps arrive as UTC and are converted to Europe/Berlin before matching,
//! so a band boundary is a *local* clock time across both transitions.
//!
//! German tariff definitions routinely treat a gesetzlicher Feiertag as a
//! Sunday rather than as the weekday it falls on. Holidays are Land law, so
//! that needs the Bundesland of the delivery point —
//! [`Zaehlzeitdefinition::holiday_land`]. Leaving it unset classifies by weekday
//! alone, which books Fronleichnam in Bavaria into the working-day register.
//!
//! ## Example — a §14a Modul 3 definition
//!
//! ```rust
//! use metering::zaehlzeit::{HT, NT, ST, Zaehlzeitdefinition};
//! use metering::Bundesland;
//! use time::macros::{date, datetime};
//!
//! // High tariff 17:00–20:00, low tariff 00:00–06:00, standard for the rest.
//! let zzd = Zaehlzeitdefinition::modul_3(
//!     "NB-14A-3",
//!     date!(2026 - 01 - 01),
//!     (17 * 60, 20 * 60),
//!     (0, 6 * 60),
//! )
//! .in_land(Bundesland::By);
//!
//! // Monday 18:00 Berlin (17:00 UTC in winter) → Hochtarif.
//! assert_eq!(zzd.register_for(datetime!(2026-01-05 17:00 UTC)), Some(HT));
//! // Monday 03:00 Berlin → Niedertarif.
//! assert_eq!(zzd.register_for(datetime!(2026-01-05 2:00 UTC)), Some(NT));
//! // Monday 10:00 Berlin → the standard band.
//! assert_eq!(zzd.register_for(datetime!(2026-01-05 9:00 UTC)), Some(ST));
//! ```

use std::collections::BTreeMap;

use rust_decimal::Decimal;
use time::{Date, OffsetDateTime, Weekday};
use time_tz::{OffsetDateTimeExt as _, timezones};

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use crate::holiday::Bundesland;
use crate::ids::BdewCode;
use crate::interval::MeterInterval;

// ── register ids ──────────────────────────────────────────────────────────────

/// Hochtarif — the conventional register id for the peak band.
pub const HT: &str = "HT";
/// Niedertarif — the conventional register id for the off-peak band.
pub const NT: &str = "NT";
/// Standardtarif — the § 14a Modul 3 band for everything that is neither.
pub const ST: &str = "ST";

/// All twelve months active in a [`ZaehlzeitFenster::months_mask`].
pub const ALL_MONTHS: u16 = 0x0FFF;

/// Minutes in a day, the exclusive upper bound of a window.
const MINUTES_PER_DAY: u16 = 24 * 60;

// ── DayGroup ──────────────────────────────────────────────────────────────────

/// Which days of the week a window applies to.
///
/// A statutory holiday is handled by [`Zaehlzeitdefinition::holiday_land`], not
/// here: it is a property of the date, not of the window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "SCREAMING_SNAKE_CASE"))]
pub enum DayGroup {
    /// Monday to Friday.
    Weekdays,
    /// Monday to Saturday.
    WeekdaysAndSaturday,
    /// All seven days — the window makes no distinction by day.
    AllDays,
}

impl DayGroup {
    /// Every variant, in declaration order.
    pub const ALL: [Self; 3] = [Self::Weekdays, Self::WeekdaysAndSaturday, Self::AllDays];

    /// Stable DB/wire label. Matches the `serde` tag and
    /// [`FromStr`](std::str::FromStr) input.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Weekdays => "WEEKDAYS",
            Self::WeekdaysAndSaturday => "WEEKDAYS_AND_SATURDAY",
            Self::AllDays => "ALL_DAYS",
        }
    }

    /// `true` when `weekday` is inside this group, ignoring holidays.
    #[must_use]
    pub const fn contains(self, weekday: Weekday) -> bool {
        match self {
            Self::Weekdays => !matches!(weekday, Weekday::Saturday | Weekday::Sunday),
            Self::WeekdaysAndSaturday => !matches!(weekday, Weekday::Sunday),
            Self::AllDays => true,
        }
    }
}

crate::codes::string_codes! {
    DayGroup;
}

// ── ZaehlzeitFenster ──────────────────────────────────────────────────────────

/// One resolution window of a Zählzeitdefinition.
///
/// Bounds are **minutes since local midnight**, not hours: § 14a bands and
/// several Netzbetreiber Preisblätter start on the half hour.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct ZaehlzeitFenster {
    /// Register this window books into — [`HT`], [`NT`], [`ST`] or an
    /// NB-assigned id.
    pub register_id: String,
    /// Months the window is active in, as a bitmask (bit 0 = January).
    /// [`ALL_MONTHS`] for a season-independent window.
    pub months_mask: u16,
    /// Day group the window applies to.
    pub days: DayGroup,
    /// Window start in Berlin local time, minutes since midnight (inclusive).
    pub from_minute: u16,
    /// Window end in Berlin local time, minutes since midnight (exclusive).
    ///
    /// A band crossing midnight is two windows; [`spanning`](Self::spanning)
    /// builds them.
    pub to_minute: u16,
}

impl ZaehlzeitFenster {
    /// An all-year, all-week window over `[from_minute, to_minute)`.
    #[must_use]
    pub fn new(register_id: impl Into<String>, from_minute: u16, to_minute: u16) -> Self {
        Self {
            register_id: register_id.into(),
            months_mask: ALL_MONTHS,
            days: DayGroup::AllDays,
            from_minute,
            to_minute,
        }
    }

    /// The one or two windows covering `[from_minute, to_minute)`, splitting at
    /// midnight when the band wraps.
    ///
    /// A Niedertarif band is usually written `22:00–06:00`, which is not a
    /// half-open range in minutes-since-midnight at all. Writing it as one
    /// window with `from > to` matches nothing; forgetting to split it is the
    /// obvious mistake, so this makes the split the easy path.
    ///
    /// ```rust
    /// use metering::zaehlzeit::{NT, ZaehlzeitFenster};
    ///
    /// let wrapping = ZaehlzeitFenster::spanning(NT, 22 * 60, 6 * 60);
    /// assert_eq!(wrapping.len(), 2, "22:00–24:00 and 00:00–06:00");
    ///
    /// let ordinary = ZaehlzeitFenster::spanning(NT, 6 * 60, 22 * 60);
    /// assert_eq!(ordinary.len(), 1);
    /// ```
    #[must_use]
    pub fn spanning(register_id: impl Into<String>, from_minute: u16, to_minute: u16) -> Vec<Self> {
        let id = register_id.into();
        if from_minute < to_minute {
            return vec![Self::new(id, from_minute, to_minute)];
        }
        if from_minute == to_minute {
            // A zero-width band is a whole day: 00:00–00:00 means "always".
            return vec![Self::new(id, 0, MINUTES_PER_DAY)];
        }
        vec![
            Self::new(id.clone(), from_minute, MINUTES_PER_DAY),
            Self::new(id, 0, to_minute),
        ]
    }

    /// Restrict the window to a day group (builder style).
    #[must_use]
    pub const fn on_days(mut self, days: DayGroup) -> Self {
        self.days = days;
        self
    }

    /// Restrict the window to the months in `mask` (builder style).
    #[must_use]
    pub const fn in_months(mut self, mask: u16) -> Self {
        self.months_mask = mask;
        self
    }

    /// `true` when the Berlin-local (month, day group, minute) falls inside.
    ///
    /// `is_holiday` short-circuits the day group: a Feiertag is never a working
    /// day, whatever weekday it lands on. A [`DayGroup::AllDays`] window keeps
    /// it, because there is no other band for it to fall into.
    fn matches(&self, month0: u8, weekday: Weekday, minute: u16, is_holiday: bool) -> bool {
        let day_matches = match self.days {
            DayGroup::AllDays => true,
            _ if is_holiday => false,
            days => days.contains(weekday),
        };
        self.months_mask & (1 << month0) != 0
            && day_matches
            && minute >= self.from_minute
            && minute < self.to_minute
    }
}

// ── Zaehlzeitdefinition ───────────────────────────────────────────────────────

/// A named Zählzeitdefinition with validity and ordered windows.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct Zaehlzeitdefinition {
    /// NB-assigned identifier (UTILTS Zählzeitdefinitions-ID).
    pub id: String,
    /// First day the definition applies (inclusive, German calendar day).
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::iso_date"))]
    pub valid_from: Date,
    /// Last day (inclusive); `None` = open-ended.
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::iso_date_option"))]
    pub valid_to: Option<Date>,
    /// Ordered windows — the **first match wins**, so put the narrower bands
    /// first.
    pub windows: Vec<ZaehlzeitFenster>,
    /// Register for times no window covers.
    ///
    /// This is what makes a two-band definition a one-window definition: HT is
    /// a window, NT is "everything else".
    pub fallback_register: Option<String>,
    /// Bundesland whose statutory holidays are treated as non-working days.
    ///
    /// `None` classifies by weekday alone. See the
    /// [module docs](self#resolution-is-dst-correct-and-feiertag-aware).
    pub holiday_land: Option<Bundesland>,
    /// Marktpartner-ID of the Netzbetreiber that published this definition.
    ///
    /// [`id`](Self::id) is *NB-assigned*, so it identifies the definition only
    /// within one operator's price sheet: `HT/NT-1` from two Netzbetreiber are
    /// two different calendars under one name. A portfolio holding hundreds of
    /// DSO calendars needs the pair.
    ///
    /// `None` for a definition built in place, where the caller already knows
    /// whose it is. The **year** is deliberately not a field: it is
    /// [`valid_from`](Self::valid_from) and [`valid_to`](Self::valid_to), and a
    /// second copy of one fact is a second thing to keep in step.
    pub netzbetreiber: Option<BdewCode>,
}

impl Zaehlzeitdefinition {
    /// The classic Zweitarif: [`HT`] inside the window on weekdays, [`NT`]
    /// everywhere else.
    ///
    /// `[from_minute, to_minute)` is Berlin local time. The BDEW
    /// Musterleistungsbeschreibung window is `(6 * 60, 22 * 60)`, but there is
    /// no national standard — each Netzbetreiber sets its own and publishes it
    /// in the Preisblatt.
    ///
    /// ```rust
    /// use metering::zaehlzeit::{HT, NT, Zaehlzeitdefinition};
    /// use time::macros::{date, datetime};
    ///
    /// let zzd = Zaehlzeitdefinition::ht_nt("NB-1", date!(2026 - 01 - 01), 6 * 60, 22 * 60);
    ///
    /// // Monday 09:00 CET = 08:00 UTC → HT.
    /// assert_eq!(zzd.register_for(datetime!(2026-01-05 8:00 UTC)), Some(HT));
    /// // Monday 22:00 CET = 21:00 UTC → NT, the end bound being exclusive.
    /// assert_eq!(zzd.register_for(datetime!(2026-01-05 21:00 UTC)), Some(NT));
    /// // Sunday midday → NT, whatever the hour.
    /// assert_eq!(zzd.register_for(datetime!(2026-01-04 11:00 UTC)), Some(NT));
    /// ```
    #[must_use]
    pub fn ht_nt(
        id: impl Into<String>,
        valid_from: Date,
        from_minute: u16,
        to_minute: u16,
    ) -> Self {
        Self {
            id: id.into(),
            valid_from,
            valid_to: None,
            windows: ZaehlzeitFenster::spanning(HT, from_minute, to_minute)
                .into_iter()
                .map(|w| w.on_days(DayGroup::Weekdays))
                .collect(),
            fallback_register: Some(NT.to_owned()),
            holiday_land: None,
            netzbetreiber: None,
        }
    }

    /// A § 14a EnWG Modul 3 definition: [`HT`], [`NT`] and [`ST`] for the rest.
    ///
    /// Since **1 April 2025** every Netzbetreiber must offer Modul 3. The
    /// bands themselves are the Netzbetreiber's to set; both are given as
    /// `(from_minute, to_minute)` in Berlin local time and may cross midnight.
    ///
    /// The HT band is listed first, so an overlap between the two resolves to
    /// HT. Both apply on **all days** — Modul 3 windows are about network load,
    /// which does not stop at the weekend — and can be narrowed afterwards
    /// through [`windows`](Self::windows) if a Netzbetreiber says otherwise.
    #[must_use]
    pub fn modul_3(
        id: impl Into<String>,
        valid_from: Date,
        hochtarif: (u16, u16),
        niedertarif: (u16, u16),
    ) -> Self {
        let mut windows = ZaehlzeitFenster::spanning(HT, hochtarif.0, hochtarif.1);
        windows.extend(ZaehlzeitFenster::spanning(NT, niedertarif.0, niedertarif.1));
        Self {
            id: id.into(),
            valid_from,
            valid_to: None,
            windows,
            fallback_register: Some(ST.to_owned()),
            holiday_land: None,
            netzbetreiber: None,
        }
    }

    /// Treat `land`'s statutory holidays as non-working days (builder style).
    #[must_use]
    pub fn in_land(mut self, land: Bundesland) -> Self {
        self.holiday_land = Some(land);
        self
    }

    /// Record which Netzbetreiber published this definition (builder style).
    #[must_use]
    pub const fn published_by(mut self, netzbetreiber: BdewCode) -> Self {
        self.netzbetreiber = Some(netzbetreiber);
        self
    }

    /// Close the validity period (builder style).
    #[must_use]
    pub const fn until(mut self, valid_to: Date) -> Self {
        self.valid_to = Some(valid_to);
        self
    }

    /// `true` when the definition is valid on the given German calendar day.
    #[must_use]
    pub fn is_valid_on(&self, date: Date) -> bool {
        date >= self.valid_from && self.valid_to.is_none_or(|end| date <= end)
    }

    /// Every register this definition can book into, sorted and deduplicated.
    ///
    /// A billing layer needs the full set up front — including the fallback,
    /// which appears in no window.
    #[must_use]
    pub fn registers(&self) -> Vec<&str> {
        let mut all: Vec<&str> = self
            .windows
            .iter()
            .map(|w| w.register_id.as_str())
            .chain(self.fallback_register.as_deref())
            .collect();
        all.sort_unstable();
        all.dedup();
        all
    }

    /// Resolve a UTC timestamp to the register it books into.
    ///
    /// Conversion to Europe/Berlin happens here, so the caller passes plain UTC
    /// interval starts and DST transitions resolve correctly. `None` when the
    /// definition is not valid on that day, or no window and no fallback covers
    /// the time.
    #[must_use]
    pub fn register_for(&self, ts_utc: OffsetDateTime) -> Option<&str> {
        let berlin = ts_utc.to_timezone(timezones::db::europe::BERLIN);
        if !self.is_valid_on(berlin.date()) {
            return None;
        }
        let month0 = u8::from(berlin.month()) - 1;
        let minute = u16::from(berlin.hour()) * 60 + u16::from(berlin.minute());
        let is_holiday = self
            .holiday_land
            .is_some_and(|land| land.is_holiday(berlin.date()));
        self.windows
            .iter()
            .find(|w| w.matches(month0, berlin.weekday(), minute, is_holiday))
            .map(|w| w.register_id.as_str())
            .or(self.fallback_register.as_deref())
    }

    /// Split a set of intervals into per-register sums.
    ///
    /// Non-billable intervals are excluded, so the totals match
    /// [`BillingPeriod::arbeitsmenge`](crate::BillingPeriod::arbeitsmenge) over
    /// the same input. Intervals outside the definition's validity or coverage
    /// land in the `None` bucket, so unassigned energy is visible rather than
    /// lost.
    ///
    /// Each interval is assigned by its **start**. An interval straddling a band
    /// boundary is booked whole into the band it begins in; at quarter-hour
    /// resolution against bands on the hour or half hour, that never arises.
    ///
    /// The keys **borrow** from this definition, so they are exactly the
    /// strings [`registers`](Self::registers) lists, and looking one up
    /// allocates nothing.
    ///
    /// ```rust
    /// use metering::zaehlzeit::{HT, NT, Zaehlzeitdefinition};
    /// use metering::{MeterInterval, QualityFlag};
    /// use rust_decimal::dec;
    /// use time::macros::{date, datetime};
    ///
    /// let zzd = Zaehlzeitdefinition::ht_nt("NB-1", date!(2026 - 01 - 01), 6 * 60, 22 * 60);
    /// let midday = MeterInterval {
    ///     from: datetime!(2026-01-05 9:00 UTC), // Monday 10:00 local
    ///     to:   datetime!(2026-01-05 9:15 UTC),
    ///     value: dec!(3),
    ///     quality: QualityFlag::Measured,
    ///     obis_code: None,
    /// };
    ///
    /// let split = zzd.split_energy(std::slice::from_ref(&midday));
    /// assert_eq!(split[&Some(HT)], dec!(3));
    /// assert!(!split.contains_key(&Some(NT)));
    /// ```
    #[must_use]
    pub fn split_energy<'a>(
        &'a self,
        intervals: &[MeterInterval],
    ) -> BTreeMap<Option<&'a str>, Decimal> {
        let mut sums: BTreeMap<Option<&'a str>, Decimal> = BTreeMap::new();
        for iv in intervals.iter().filter(|iv| iv.quality.is_billable()) {
            *sums
                .entry(self.register_for(iv.from))
                .or_insert(Decimal::ZERO) += iv.value;
        }
        sums
    }
}

// ── Modul 3 conformance ──────────────────────────────────────────────────────

/// A calendar quarter, January to March being the first.
///
/// BDEW *Anwendungshilfe für die Umsetzung von Modul 3* v1.1 (07.02.2025),
/// §2: *"Dabei wird das Jahr in kalenderjährliche Quartale beginnend mit
/// Januar unterteilt."*
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum Quarter {
    /// January, February, March.
    Q1,
    /// April, May, June.
    Q2,
    /// July, August, September.
    Q3,
    /// October, November, December.
    Q4,
}

impl Quarter {
    /// Every quarter, in calendar order.
    pub const ALL: [Self; 4] = [Self::Q1, Self::Q2, Self::Q3, Self::Q4];

    /// Stable DB/wire label. Matches the `serde` tag and
    /// [`FromStr`](std::str::FromStr) input.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Q1 => "Q1",
            Self::Q2 => "Q2",
            Self::Q3 => "Q3",
            Self::Q4 => "Q4",
        }
    }

    /// The quarter a 1-based calendar month falls in, or `None` outside 1–12.
    #[must_use]
    pub const fn of_month(month: u8) -> Option<Self> {
        match month {
            1..=3 => Some(Self::Q1),
            4..=6 => Some(Self::Q2),
            7..=9 => Some(Self::Q3),
            10..=12 => Some(Self::Q4),
            _ => None,
        }
    }

    /// The three 1-based calendar months in this quarter.
    #[must_use]
    pub const fn months(self) -> [u8; 3] {
        match self {
            Self::Q1 => [1, 2, 3],
            Self::Q2 => [4, 5, 6],
            Self::Q3 => [7, 8, 9],
            Self::Q4 => [10, 11, 12],
        }
    }
}

/// Shortest Hochtarif window the Modul 3 rules admit, in minutes.
///
/// BDEW AWH Modul 3 v1.1, §2: *"Hochlasttarif (HT): min. an 2 Stunden pro
/// Tag"*.
pub const MODUL_3_MIN_HOCHTARIF_MINUTES: u16 = 120;

/// Fewest calendar quarters the three tariffs must be billed in.
///
/// BDEW AWH Modul 3 v1.1, §2: *"Die Zeitfenster und insofern die drei
/// Netzentgelttarife müssen in mindestens zwei Quartalen eines Jahres
/// abgerechnet werden. Diese zwei Quartale müssen nicht zusammenhängend
/// sein."*
pub const MODUL_3_MIN_BILLED_QUARTERS: usize = 2;

/// Why a Zählzeitdefinition does not meet the Modul 3 rules.
///
/// A closed vocabulary, like [`crate::sharing::Finding`]: a curation pipeline
/// routes on the reason, and `contains("Hochtarif")` is not a interface.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "SCREAMING_SNAKE_CASE"))]
pub enum Modul3Finding {
    /// The definition does not book into exactly [`HT`], [`NT`] and [`ST`].
    RegistersAreNotHtNtSt,
    /// A register is declared but no instant of any day ever resolves to it.
    ///
    /// The likeliest cause is a band written as one wrapping window —
    /// `22:00–06:00` as a single `from > to` pair, which
    /// a window's bounds can never satisfy. The register then
    /// appears in [`registers`](Zaehlzeitdefinition::registers), the day is
    /// still fully covered by the fallback, and every other rule passes: the
    /// calendar reads as a conforming three-level tariff while one of the three
    /// levels is unreachable. [`ZaehlzeitFenster::spanning`] is what builds the
    /// two windows such a band actually needs.
    RegisterNeverReached,
    /// Some part of the day falls into no register at all — there is no
    /// fallback, or a window leaves a hole.
    TimeNotFullyCovered,
    /// The Hochtarif window is shorter than
    /// [`MODUL_3_MIN_HOCHTARIF_MINUTES`] on at least one day the definition
    /// distinguishes.
    HochtarifBelowTwoHours,
    /// The windows are not the same in every month, so they vary between
    /// quarters.
    WindowsVaryAcrossTheYear,
    /// Fewer than [`MODUL_3_MIN_BILLED_QUARTERS`] quarters were named.
    FewerThanTwoBilledQuarters,
    /// No billed quarters were supplied, so that rule could not be checked.
    BilledQuartersUnknown,
    /// Validity does not describe one calendar year.
    ValidityIsNotOneCalendarYear,
    /// The delivery point has not selected Modul 1.
    ///
    /// The `serde` tag is spelled out because `SCREAMING_SNAKE_CASE` renders
    /// the Rust name as `MODUL1_NOT_SELECTED`, which is not what `as_str`
    /// writes — and the contract is that the two are the same string.
    #[cfg_attr(feature = "serde", serde(rename = "MODUL_1_NOT_SELECTED"))]
    Modul1NotSelected,
    /// The Marktlokation is metered by registrierende Leistungsmessung.
    RegistrierendeLeistungsmessung,
    /// No intelligentes Messsystem is installed.
    NoIntelligentesMesssystem,
    /// A precondition on the delivery point was not stated.
    DeliveryPointDataMissing,
}

impl Modul3Finding {
    /// Every finding, in declaration order.
    pub const ALL: [Self; 12] = [
        Self::RegistersAreNotHtNtSt,
        Self::RegisterNeverReached,
        Self::TimeNotFullyCovered,
        Self::HochtarifBelowTwoHours,
        Self::WindowsVaryAcrossTheYear,
        Self::FewerThanTwoBilledQuarters,
        Self::BilledQuartersUnknown,
        Self::ValidityIsNotOneCalendarYear,
        Self::Modul1NotSelected,
        Self::RegistrierendeLeistungsmessung,
        Self::NoIntelligentesMesssystem,
        Self::DeliveryPointDataMissing,
    ];

    /// Stable DB/wire label. Matches the `serde` tag and
    /// [`FromStr`](std::str::FromStr) input.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::RegistersAreNotHtNtSt => "REGISTERS_ARE_NOT_HT_NT_ST",
            Self::RegisterNeverReached => "REGISTER_NEVER_REACHED",
            Self::TimeNotFullyCovered => "TIME_NOT_FULLY_COVERED",
            Self::HochtarifBelowTwoHours => "HOCHTARIF_BELOW_TWO_HOURS",
            Self::WindowsVaryAcrossTheYear => "WINDOWS_VARY_ACROSS_THE_YEAR",
            Self::FewerThanTwoBilledQuarters => "FEWER_THAN_TWO_BILLED_QUARTERS",
            Self::BilledQuartersUnknown => "BILLED_QUARTERS_UNKNOWN",
            Self::ValidityIsNotOneCalendarYear => "VALIDITY_IS_NOT_ONE_CALENDAR_YEAR",
            Self::Modul1NotSelected => "MODUL_1_NOT_SELECTED",
            Self::RegistrierendeLeistungsmessung => "REGISTRIERENDE_LEISTUNGSMESSUNG",
            Self::NoIntelligentesMesssystem => "NO_INTELLIGENTES_MESSSYSTEM",
            Self::DeliveryPointDataMissing => "DELIVERY_POINT_DATA_MISSING",
        }
    }

    /// The provision this finding rests on.
    #[must_use]
    pub const fn legal_basis(self) -> &'static str {
        match self {
            Self::BilledQuartersUnknown | Self::DeliveryPointDataMissing => {
                "(not a rule — the input did not say)"
            }
            _ => "BDEW AWH Modul 3 v1.1 (07.02.2025) §2",
        }
    }

    /// `true` when the finding is about missing input rather than a breach.
    #[must_use]
    pub const fn is_unknown(self) -> bool {
        matches!(
            self,
            Self::BilledQuartersUnknown | Self::DeliveryPointDataMissing
        )
    }
}

/// Whether a Zählzeitdefinition meets the Modul 3 rules.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "SCREAMING_SNAKE_CASE"))]
pub enum Modul3Conformance {
    /// Every rule this crate can check was met.
    Conforms,
    /// At least one rule was broken.
    Violates,
    /// Nothing was broken, but something could not be checked.
    Unknown,
}

impl Modul3Conformance {
    /// Every verdict, in declaration order.
    pub const ALL: [Self; 3] = [Self::Conforms, Self::Violates, Self::Unknown];

    /// Stable DB/wire label. Matches the `serde` tag and
    /// [`FromStr`](std::str::FromStr) input.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Conforms => "CONFORMS",
            Self::Violates => "VIOLATES",
            Self::Unknown => "UNKNOWN",
        }
    }
}

crate::codes::string_codes! {
    Quarter;
    Modul3Finding;
    Modul3Conformance;
}

/// What [`assess_modul_3`] needs that the calendar itself does not carry.
///
/// Every field is optional because a curated portfolio is routinely
/// incomplete, and the assessment reports *what it could not check* rather
/// than guessing — the same shape as
/// [`MeteringCapabilityInput`](crate::MeteringCapabilityInput).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Modul3Context {
    /// The calendar quarters the Netzbetreiber bills the three tariffs in.
    ///
    /// The operator's Wahlrecht, published on the price sheet — *"Der
    /// Netzbetreiber hat das Wahlrecht, den Gültigkeitszeitraum auf einzelne
    /// Quartale zu beschränken"* — so it is not a property of the windows.
    /// Duplicates are ignored.
    pub billed_quarters: Option<Vec<Quarter>>,
    /// Whether the delivery point has selected Modul 1.
    pub modul_1_selected: Option<bool>,
    /// Whether the Marktlokation is metered by registrierende
    /// Leistungsmessung.
    pub registrierende_leistungsmessung: Option<bool>,
    /// Whether an intelligentes Messsystem is installed.
    pub intelligentes_messsystem: Option<bool>,
}

impl Modul3Context {
    /// The quarters the three tariffs are billed in (builder style).
    #[must_use]
    pub fn billed_in(mut self, quarters: impl IntoIterator<Item = Quarter>) -> Self {
        self.billed_quarters = Some(quarters.into_iter().collect());
        self
    }

    /// Whether the delivery point has Modul 1 selected (builder style).
    ///
    /// The three preconditions are named rather than positional: as bare
    /// booleans they read as interchangeable, and the middle one is inverted —
    /// Modul 3 requires *no* RLM.
    #[must_use]
    pub const fn with_modul_1(mut self, selected: bool) -> Self {
        self.modul_1_selected = Some(selected);
        self
    }

    /// Whether the Marktlokation is metered by registrierende
    /// Leistungsmessung (builder style). Modul 3 requires that it is **not**.
    #[must_use]
    pub const fn with_registrierende_leistungsmessung(mut self, rlm: bool) -> Self {
        self.registrierende_leistungsmessung = Some(rlm);
        self
    }

    /// Whether an intelligentes Messsystem is installed (builder style).
    #[must_use]
    pub const fn with_intelligentes_messsystem(mut self, imsys: bool) -> Self {
        self.intelligentes_messsystem = Some(imsys);
        self
    }

    /// The three delivery-point preconditions a conforming point satisfies:
    /// Modul 1 selected, no RLM, iMSys installed.
    #[must_use]
    pub const fn at_a_conforming_delivery_point(self) -> Self {
        self.with_modul_1(true)
            .with_registrierende_leistungsmessung(false)
            .with_intelligentes_messsystem(true)
    }
}

/// Assess a Zählzeitdefinition against the § 14a **Modul 3** rules.
///
/// Modul 3 is the *Anreizmodul* of BK8-22/010-A: three time-variable
/// Netzentgelt levels over windows the Netzbetreiber sets for its whole
/// Netzgebiet, mandatory to offer since 1 April 2025. A portfolio of hundreds
/// of DSO calendars is worth refusing at the door rather than at the optimiser.
///
/// | Rule (BDEW AWH Modul 3 v1.1 §2) | Finding |
/// |---|---|
/// | three Netzentgelttarife HT/NT/ST | [`RegistersAreNotHtNtSt`] |
/// | each of them reachable | [`RegisterNeverReached`] |
/// | every instant books into one of them | [`TimeNotFullyCovered`] |
/// | HT at least two hours per day | [`HochtarifBelowTwoHours`] |
/// | windows *ganzjährig identisch* | [`WindowsVaryAcrossTheYear`] |
/// | billed in at least two quarters | [`FewerThanTwoBilledQuarters`] |
/// | set per calendar year | [`ValidityIsNotOneCalendarYear`] |
/// | only with Modul 1, iMSys, no RLM | from [`Modul3Context`] |
///
/// The price corridors (HT ≤ 100 % Aufschlag, NT 10–40 % of ST, the
/// Netzentgeltgleichheit condition) and the publication deadline are **not**
/// checked. The first are money, which this crate does not model; the second is
/// a Fristen question, and the AWH states its 15.10. date for the first year
/// rather than as a standing rule. Check both where the price sheet is parsed.
///
/// *"min. an 2 Stunden pro Tag"* is read per day **class** — an ordinary
/// weekday, a Saturday, a Sunday, and a statutory holiday where
/// [`holiday_land`](Zaehlzeitdefinition::holiday_land) is set — so a
/// weekday-only Hochtarif is reported: on a Sunday it offers none at all.
/// Minutes are wall-clock minutes, the vocabulary the windows are written in.
///
/// [`RegistersAreNotHtNtSt`]: Modul3Finding::RegistersAreNotHtNtSt
/// [`RegisterNeverReached`]: Modul3Finding::RegisterNeverReached
/// [`TimeNotFullyCovered`]: Modul3Finding::TimeNotFullyCovered
/// [`HochtarifBelowTwoHours`]: Modul3Finding::HochtarifBelowTwoHours
/// [`WindowsVaryAcrossTheYear`]: Modul3Finding::WindowsVaryAcrossTheYear
/// [`FewerThanTwoBilledQuarters`]: Modul3Finding::FewerThanTwoBilledQuarters
/// [`ValidityIsNotOneCalendarYear`]: Modul3Finding::ValidityIsNotOneCalendarYear
///
/// ```rust
/// use metering::zaehlzeit::{
///     Modul3Conformance, Modul3Context, Quarter, Zaehlzeitdefinition, assess_modul_3,
/// };
/// use time::macros::date;
///
/// let zzd = Zaehlzeitdefinition::modul_3(
///     "NB-14A-3",
///     date!(2026 - 01 - 01),
///     (17 * 60, 20 * 60), // Hochtarif 17:00–20:00 — three hours
///     (22 * 60, 6 * 60),  // Niedertarif 22:00–06:00, wrapping
/// )
/// .until(date!(2026 - 12 - 31));
///
/// let ctx = Modul3Context::default()
///     .billed_in([Quarter::Q1, Quarter::Q4])
///     .at_a_conforming_delivery_point();
///
/// let (verdict, findings) = assess_modul_3(&zzd, &ctx);
/// assert_eq!(verdict, Modul3Conformance::Conforms, "{findings:?}");
/// ```
#[must_use]
pub fn assess_modul_3(
    zzd: &Zaehlzeitdefinition,
    ctx: &Modul3Context,
) -> (Modul3Conformance, Vec<Modul3Finding>) {
    let mut findings = Vec::new();

    // ── the three tariffs ────────────────────────────────────────────────
    if zzd.registers() != vec![HT, NT, ST] {
        findings.push(Modul3Finding::RegistersAreNotHtNtSt);
    }

    // ── the day profiles, once per (month, day class) ────────────────────
    let classes = day_classes(zzd);
    let mut months = (1u8..=12).map(|month| {
        classes
            .iter()
            .map(|&class| day_profile(zzd, month - 1, class))
            .collect::<Vec<_>>()
    });
    let first = months.next().unwrap_or_default();
    if !months.all(|other| other == first) {
        findings.push(Modul3Finding::WindowsVaryAcrossTheYear);
    }
    let minutes = |profile: &DayProfile<'_>, register: &str| -> u16 {
        profile.get(&Some(register)).copied().unwrap_or(0)
    };
    if first.iter().any(|p| p.contains_key(&None)) {
        findings.push(Modul3Finding::TimeNotFullyCovered);
    }
    if first
        .iter()
        .any(|p| minutes(p, HT) < MODUL_3_MIN_HOCHTARIF_MINUTES)
    {
        findings.push(Modul3Finding::HochtarifBelowTwoHours);
    }
    // A register nobody can ever be charged is not one of the three levels,
    // whatever the window list says it is.
    let reachable: std::collections::BTreeSet<&str> = (1u8..=12)
        .flat_map(|month| {
            classes
                .iter()
                .flat_map(move |&class| day_profile(zzd, month - 1, class).into_keys())
        })
        .flatten()
        .collect();
    if zzd.registers().iter().any(|r| !reachable.contains(r)) {
        findings.push(Modul3Finding::RegisterNeverReached);
    }

    // ── billed quarters ──────────────────────────────────────────────────
    match &ctx.billed_quarters {
        None => findings.push(Modul3Finding::BilledQuartersUnknown),
        Some(quarters) => {
            let distinct: std::collections::BTreeSet<Quarter> = quarters.iter().copied().collect();
            if distinct.len() < MODUL_3_MIN_BILLED_QUARTERS {
                findings.push(Modul3Finding::FewerThanTwoBilledQuarters);
            }
        }
    }

    // ── validity ─────────────────────────────────────────────────────────
    // An open end is how operators express "until further notice" and is not
    // itself a breach; a start that is not a 1 January, or an end that is not
    // the 31 December of that same year, demonstrably is not one calendar year.
    let starts_a_year = zzd.valid_from.month() == time::Month::January && zzd.valid_from.day() == 1;
    let ends_that_year = zzd.valid_to.is_none_or(|end| {
        end.year() == zzd.valid_from.year()
            && end.month() == time::Month::December
            && end.day() == 31
    });
    if !starts_a_year || !ends_that_year {
        findings.push(Modul3Finding::ValidityIsNotOneCalendarYear);
    }

    // ── delivery-point preconditions ─────────────────────────────────────
    let mut missing = false;
    let mut precondition = |value: Option<bool>, required: bool, breach: Modul3Finding| match value
    {
        Some(v) if v == required => {}
        Some(_) => findings.push(breach),
        None => missing = true,
    };
    precondition(ctx.modul_1_selected, true, Modul3Finding::Modul1NotSelected);
    precondition(
        ctx.registrierende_leistungsmessung,
        false,
        Modul3Finding::RegistrierendeLeistungsmessung,
    );
    precondition(
        ctx.intelligentes_messsystem,
        true,
        Modul3Finding::NoIntelligentesMesssystem,
    );
    if missing {
        findings.push(Modul3Finding::DeliveryPointDataMissing);
    }

    let verdict = if findings.iter().any(|f| !f.is_unknown()) {
        Modul3Conformance::Violates
    } else if findings.is_empty() {
        Modul3Conformance::Conforms
    } else {
        Modul3Conformance::Unknown
    };
    (verdict, findings)
}

/// How one kind of day resolves: wall-clock minutes per register, with `None`
/// for the minutes no register covers.
///
/// The **whole** map, not a summary of it. Comparing only the Hochtarif and the
/// uncovered minutes across the twelve months missed a definition whose NT and
/// ST swap between summer and winter while HT stays put — which is exactly the
/// *"Preisstufen und Zeitfenster müssen ganzjährig identisch sein"* rule the
/// comparison exists to enforce.
type DayProfile<'a> = BTreeMap<Option<&'a str>, u16>;

/// The kinds of day a definition can tell apart.
///
/// [`ZaehlzeitFenster::matches`] reads the weekday only through
/// [`DayGroup::contains`] and the holiday flag, so four representatives cover
/// every distinction the type can express — and the holiday one only arises
/// where a [`Bundesland`] was named.
fn day_classes(zzd: &Zaehlzeitdefinition) -> Vec<(Weekday, bool)> {
    let mut classes = vec![
        (Weekday::Monday, false),
        (Weekday::Saturday, false),
        (Weekday::Sunday, false),
    ];
    if zzd.holiday_land.is_some() {
        classes.push((Weekday::Monday, true));
    }
    classes
}

/// Resolve one (month, day class) into its register profile.
///
/// [`ZaehlzeitFenster::matches`] is piecewise constant between window bounds,
/// so evaluating once per segment is exact — and far cheaper than walking
/// 1 440 minutes twelve times over.
fn day_profile<'a>(
    zzd: &'a Zaehlzeitdefinition,
    month0: u8,
    class: (Weekday, bool),
) -> DayProfile<'a> {
    let (weekday, is_holiday) = class;

    let mut breakpoints: Vec<u16> = vec![0, MINUTES_PER_DAY];
    for w in &zzd.windows {
        breakpoints.push(w.from_minute.min(MINUTES_PER_DAY));
        breakpoints.push(w.to_minute.min(MINUTES_PER_DAY));
    }
    breakpoints.sort_unstable();
    breakpoints.dedup();

    let mut profile = DayProfile::new();
    for pair in breakpoints.windows(2) {
        let (from, to) = (pair[0], pair[1]);
        let register = zzd
            .windows
            .iter()
            .find(|w| w.matches(month0, weekday, from, is_holiday))
            .map(|w| w.register_id.as_str())
            .or(zzd.fallback_register.as_deref());
        *profile.entry(register).or_insert(0) += to - from;
    }
    profile
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interval::QualityFlag;
    use rust_decimal::dec;
    use time::macros::{date, datetime};

    fn iv(from: OffsetDateTime, kwh: Decimal) -> MeterInterval {
        MeterInterval {
            from,
            to: from + time::Duration::minutes(15),
            value: kwh,
            quality: QualityFlag::Measured,
            obis_code: None,
        }
    }

    // ── the classic Zweitarif ────────────────────────────────────────────────

    #[test]
    fn ht_nt_resolves_in_berlin_local_time() {
        let zzd = Zaehlzeitdefinition::ht_nt("NB-1", date!(2026 - 01 - 01), 6 * 60, 22 * 60);

        // Winter, CET = UTC+1.
        assert_eq!(zzd.register_for(datetime!(2026-01-05 8:00 UTC)), Some(HT));
        assert_eq!(
            zzd.register_for(datetime!(2026-01-05 21:00 UTC)),
            Some(NT),
            "22:00 local is the exclusive end bound"
        );
        assert_eq!(zzd.register_for(datetime!(2026-01-05 20:59 UTC)), Some(HT));

        // Summer, CEST = UTC+2 — the same local bounds, two hours earlier in UTC.
        assert_eq!(zzd.register_for(datetime!(2026-06-01 7:00 UTC)), Some(HT));
        assert_eq!(
            zzd.register_for(datetime!(2026-06-01 20:01 UTC)),
            Some(NT),
            "20:01 UTC is 22:01 CEST"
        );
    }

    /// The weekday is read in Berlin too. 23:30 UTC on a Friday is already
    /// Saturday locally — reading the weekday in UTC misclassified the first
    /// local hour of every day.
    #[test]
    fn the_weekday_is_read_in_berlin_too() {
        let all_hours = Zaehlzeitdefinition {
            windows: vec![
                ZaehlzeitFenster::new(HT, 0, MINUTES_PER_DAY).on_days(DayGroup::Weekdays),
            ],
            ..Zaehlzeitdefinition::ht_nt("NB-1", date!(2026 - 01 - 01), 0, 0)
        };

        // Friday 2026-01-02 23:30 UTC is Saturday 00:30 in Berlin.
        let saturday_early = datetime!(2026-01-02 23:30 UTC);
        assert_eq!(saturday_early.weekday(), Weekday::Friday);
        assert_eq!(
            all_hours.register_for(saturday_early),
            Some(NT),
            "Saturday 00:30 Berlin is not a weekday, even though UTC says Friday"
        );

        // Sunday 2026-01-04 23:30 UTC is Monday 00:30 in Berlin.
        let monday_early = datetime!(2026-01-04 23:30 UTC);
        assert_eq!(monday_early.weekday(), Weekday::Sunday);
        assert_eq!(all_hours.register_for(monday_early), Some(HT));
    }

    #[test]
    fn weekends_fall_to_the_fallback() {
        let zzd = Zaehlzeitdefinition::ht_nt("NB-1", date!(2026 - 01 - 01), 6 * 60, 22 * 60);
        assert_eq!(zzd.register_for(datetime!(2026-01-03 9:00 UTC)), Some(NT));
        assert_eq!(zzd.register_for(datetime!(2026-01-04 11:00 UTC)), Some(NT));
    }

    // ── §14a Modul 3 ─────────────────────────────────────────────────────────

    /// The reason the two-register type is gone: Modul 3 has three levels, and
    /// every Netzbetreiber has had to offer it since 1 April 2025.
    #[test]
    fn modul_3_resolves_three_registers() {
        let zzd = Zaehlzeitdefinition::modul_3(
            "NB-14A-3",
            date!(2026 - 01 - 01),
            (17 * 60, 20 * 60),
            (0, 6 * 60),
        );

        assert_eq!(zzd.registers(), vec![HT, NT, ST]);
        assert_eq!(zzd.register_for(datetime!(2026-01-05 17:00 UTC)), Some(HT)); // 18:00
        assert_eq!(zzd.register_for(datetime!(2026-01-05 2:00 UTC)), Some(NT)); // 03:00
        assert_eq!(zzd.register_for(datetime!(2026-01-05 9:00 UTC)), Some(ST)); // 10:00
        assert_eq!(zzd.register_for(datetime!(2026-01-05 20:00 UTC)), Some(ST)); // 21:00

        // Modul 3 bands are about network load, so they apply at the weekend too.
        assert_eq!(zzd.register_for(datetime!(2026-01-04 17:00 UTC)), Some(HT));
    }

    /// A Niedertarif band is normally written 22:00–06:00, which is not a
    /// half-open minute range. It has to become two windows, and forgetting
    /// that matches nothing at all.
    #[test]
    fn a_band_crossing_midnight_becomes_two_windows() {
        let zzd = Zaehlzeitdefinition::modul_3(
            "NB-14A-3",
            date!(2026 - 01 - 01),
            (17 * 60, 20 * 60),
            (22 * 60, 6 * 60), // wraps
        );
        assert_eq!(zzd.windows.len(), 3, "one HT window, two NT windows");

        // 23:00 Berlin and 03:00 Berlin are both NT.
        assert_eq!(zzd.register_for(datetime!(2026-01-05 22:00 UTC)), Some(NT));
        assert_eq!(zzd.register_for(datetime!(2026-01-06 2:00 UTC)), Some(NT));
        // ...and 07:00 Berlin is not.
        assert_eq!(zzd.register_for(datetime!(2026-01-06 6:00 UTC)), Some(ST));

        // A non-wrapping band stays one window.
        assert_eq!(ZaehlzeitFenster::spanning(NT, 6 * 60, 22 * 60).len(), 1);
        // A zero-width band is a whole day, not an empty one.
        let whole_day = ZaehlzeitFenster::spanning(NT, 0, 0);
        assert_eq!(whole_day.len(), 1);
        assert_eq!(
            (whole_day[0].from_minute, whole_day[0].to_minute),
            (0, 1440)
        );
    }

    /// Minute granularity, which the hour-bounded predecessor could not express.
    #[test]
    fn bands_can_start_on_the_half_hour() {
        let zzd = Zaehlzeitdefinition::modul_3(
            "NB-1",
            date!(2026 - 01 - 01),
            (17 * 60 + 30, 19 * 60 + 30),
            (0, 6 * 60),
        );
        // 18:29 Berlin → HT; 17:29 Berlin → ST.
        assert_eq!(zzd.register_for(datetime!(2026-01-05 17:29 UTC)), Some(HT));
        assert_eq!(zzd.register_for(datetime!(2026-01-05 16:29 UTC)), Some(ST));
    }

    // ── seasons ──────────────────────────────────────────────────────────────

    #[test]
    fn a_seasonal_window_only_applies_in_its_months() {
        let winter_ht = Zaehlzeitdefinition {
            windows: vec![
                ZaehlzeitFenster::new(HT, 6 * 60, 22 * 60)
                    .on_days(DayGroup::Weekdays)
                    // November–March.
                    .in_months(0b0000_1100_0000_0111),
            ],
            ..Zaehlzeitdefinition::ht_nt("ZZD-SEASON", date!(2026 - 01 - 01), 0, 0)
        };
        // Thursday 2026-01-15 08:00 Berlin → HT.
        assert_eq!(
            winter_ht.register_for(datetime!(2026-01-15 7:00 UTC)),
            Some(HT)
        );
        // The same clock time in July → not in the mask, so the fallback.
        assert_eq!(
            winter_ht.register_for(datetime!(2026-07-16 6:00 UTC)),
            Some(NT)
        );
    }

    // ── holidays ─────────────────────────────────────────────────────────────

    /// A Feiertag is a Sunday for a Zählzeitdefinition — and only in the Länder
    /// that observe it.
    #[test]
    fn a_feiertag_books_into_the_fallback_register() {
        let base = Zaehlzeitdefinition::ht_nt("ZZD-FT", date!(2026 - 01 - 01), 6 * 60, 22 * 60);
        let midday = datetime!(2026-06-04 8:00 UTC); // Fronleichnam, 10:00 CEST, a Thursday

        assert_eq!(
            base.register_for(midday),
            Some(HT),
            "weekday alone sees an ordinary Thursday"
        );
        assert_eq!(
            base.clone().in_land(Bundesland::By).register_for(midday),
            Some(NT),
            "a Bavarian Feiertag is not a working day"
        );
        assert_eq!(
            base.clone().in_land(Bundesland::Be).register_for(midday),
            Some(HT),
            "...and in Berlin it is still a Thursday"
        );

        // An ordinary Thursday is unaffected in either Land.
        let ordinary = datetime!(2026-06-11 8:00 UTC);
        for land in [Bundesland::By, Bundesland::Be] {
            assert_eq!(
                base.clone().in_land(land).register_for(ordinary),
                Some(HT),
                "{land}"
            );
        }
    }

    /// An all-days window has nowhere else to put a Feiertag, so it keeps it.
    /// Modul 3 bands are all-days, so a Feiertag stays on its network-load band.
    #[test]
    fn an_all_days_window_still_covers_feiertage() {
        let zzd = Zaehlzeitdefinition::modul_3(
            "NB-14A-3",
            date!(2026 - 01 - 01),
            (8 * 60, 12 * 60),
            (0, 6 * 60),
        )
        .in_land(Bundesland::By);
        // Fronleichnam 10:00 CEST is still inside the HT band.
        assert_eq!(zzd.register_for(datetime!(2026-06-04 8:00 UTC)), Some(HT));
    }

    // ── validity ─────────────────────────────────────────────────────────────

    #[test]
    fn validity_bounds_are_inclusive() {
        let zzd = Zaehlzeitdefinition::ht_nt("NB-1", date!(2026 - 01 - 01), 6 * 60, 22 * 60)
            .until(date!(2026 - 06 - 30));
        assert!(zzd.register_for(datetime!(2026-07-01 10:00 UTC)).is_none());
        assert!(zzd.register_for(datetime!(2026-06-30 10:00 UTC)).is_some());
        assert!(zzd.register_for(datetime!(2025-12-31 10:00 UTC)).is_none());
        assert!(zzd.register_for(datetime!(2026-01-01 10:00 UTC)).is_some());
    }

    // ── splitting ────────────────────────────────────────────────────────────

    /// The invariant a billing layer depends on: the register sums add up to
    /// the Arbeitsmenge over the same intervals, with nothing lost or
    /// double-counted.
    #[test]
    fn the_register_sums_reconstruct_the_arbeitsmenge() {
        use crate::{AggregationConfig, aggregate};

        let zzd = Zaehlzeitdefinition::modul_3(
            "NB-14A-3",
            date!(2026 - 01 - 01),
            (17 * 60, 20 * 60),
            (0, 6 * 60),
        );

        // A full winter day of quarter-hours.
        let start = crate::calendar::day_start_utc(date!(2026 - 01 - 05));
        let intervals: Vec<MeterInterval> = (0..96)
            .map(|i| iv(start + time::Duration::minutes(i * 15), dec!(2.5)))
            .collect();

        let split = zzd.split_energy(&intervals);
        let period = aggregate(&intervals, &AggregationConfig::rlm());

        assert_eq!(
            split.values().sum::<Decimal>(),
            period.arbeitsmenge,
            "every interval is booked exactly once"
        );
        assert!(!split.contains_key(&None), "the fallback covers everything");
        // 12 quarter-hours HT (17:00–20:00), 24 NT (00:00–06:00), 60 ST.
        assert_eq!(split[&Some(HT)], dec!(2.5) * dec!(12));
        assert_eq!(split[&Some(NT)], dec!(2.5) * dec!(24));
        assert_eq!(split[&Some(ST)], dec!(2.5) * dec!(60));
    }

    /// The same invariant across a DST day, where the register counts are not
    /// the ones a 96-interval assumption would give.
    #[test]
    fn the_split_holds_across_both_dst_transitions() {
        use crate::{AggregationConfig, aggregate};

        let zzd = Zaehlzeitdefinition::modul_3(
            "NB-14A-3",
            date!(2026 - 01 - 01),
            (17 * 60, 20 * 60),
            (0, 6 * 60),
        );

        // The NT band is 00:00–06:00 local, so the skipped and repeated hours
        // both fall inside it. An ordinary day has 24 NT quarter-hours; the
        // spring day loses four and the autumn day gains four. A fixed 96 —
        // or a fixed 24 — is wrong on both.
        for (day, count, expected_nt) in [
            (date!(2026 - 03 - 29), 92i64, 20u32),
            (date!(2026 - 07 - 20), 96, 24),
            (date!(2026 - 10 - 25), 100, 28),
        ] {
            let start = crate::calendar::day_start_utc(day);
            let intervals: Vec<MeterInterval> = (0..count)
                .map(|i| iv(start + time::Duration::minutes(i * 15), dec!(1)))
                .collect();
            let split = zzd.split_energy(&intervals);
            let period = aggregate(&intervals, &AggregationConfig::rlm());

            assert_eq!(
                split.values().sum::<Decimal>(),
                period.arbeitsmenge,
                "{day}: nothing lost across the transition"
            );
            assert_eq!(
                split[&Some(NT)],
                Decimal::from(expected_nt),
                "{day}: NT quarter-hours"
            );
        }
    }

    /// Energy outside the definition's validity is visible in the `None`
    /// bucket, not silently dropped.
    #[test]
    fn unassigned_energy_is_reported_rather_than_lost() {
        let zzd = Zaehlzeitdefinition::ht_nt("NB-1", date!(2026 - 02 - 01), 6 * 60, 22 * 60);
        let split = zzd.split_energy(&[
            iv(datetime!(2026-01-15 8:00 UTC), dec!(3)), // before valid_from
            iv(datetime!(2026-02-16 8:00 UTC), dec!(4)), // inside
        ]);
        assert_eq!(split[&None], dec!(3));
        assert_eq!(split[&Some(HT)], dec!(4));
    }

    #[test]
    fn non_billable_intervals_are_excluded() {
        let zzd = Zaehlzeitdefinition::ht_nt("NB-1", date!(2026 - 01 - 01), 6 * 60, 22 * 60);
        let mut faulty = iv(datetime!(2026-01-05 8:00 UTC), dec!(99));
        faulty.quality = QualityFlag::Faulty;
        let split = zzd.split_energy(&[iv(datetime!(2026-01-05 9:00 UTC), dec!(4)), faulty]);
        assert_eq!(split[&Some(HT)], dec!(4));
        assert_eq!(split.len(), 1);
    }

    // ── metadata ─────────────────────────────────────────────────────────────

    #[test]
    fn registers_are_sorted_and_deduplicated() {
        let zzd = Zaehlzeitdefinition::modul_3(
            "NB-1",
            date!(2026 - 01 - 01),
            (17 * 60, 20 * 60),
            (22 * 60, 6 * 60), // two NT windows, one register
        );
        assert_eq!(zzd.registers(), vec![HT, NT, ST]);
    }

    #[test]
    fn day_groups_cover_the_week_as_named() {
        use Weekday::{Friday, Monday, Saturday, Sunday, Thursday, Tuesday, Wednesday};
        for day in [Monday, Tuesday, Wednesday, Thursday, Friday] {
            assert!(DayGroup::Weekdays.contains(day));
        }
        assert!(!DayGroup::Weekdays.contains(Saturday));
        assert!(!DayGroup::Weekdays.contains(Sunday));
        assert!(DayGroup::WeekdaysAndSaturday.contains(Saturday));
        assert!(!DayGroup::WeekdaysAndSaturday.contains(Sunday));
        for day in [Monday, Saturday, Sunday] {
            assert!(DayGroup::AllDays.contains(day));
        }
    }
}

#[cfg(test)]
mod modul_3_conformance_tests {
    use super::*;
    use time::macros::date;

    fn conforming() -> Zaehlzeitdefinition {
        Zaehlzeitdefinition::modul_3(
            "NB-14A-3",
            date!(2026 - 01 - 01),
            (17 * 60, 20 * 60),
            (22 * 60, 6 * 60),
        )
        .until(date!(2026 - 12 - 31))
    }

    fn full_context() -> Modul3Context {
        Modul3Context::default()
            .billed_in([Quarter::Q1, Quarter::Q4])
            .at_a_conforming_delivery_point()
    }

    #[test]
    fn a_well_formed_definition_conforms() {
        let (verdict, findings) = assess_modul_3(&conforming(), &full_context());
        assert_eq!(verdict, Modul3Conformance::Conforms, "{findings:?}");
        assert!(findings.is_empty());
    }

    /// *"min. an 2 Stunden pro Tag"* — a 90-minute Hochtarif is short.
    #[test]
    fn a_hochtarif_under_two_hours_is_a_breach() {
        let zzd = Zaehlzeitdefinition::modul_3(
            "NB-1",
            date!(2026 - 01 - 01),
            (17 * 60, 18 * 60 + 30), // 90 minutes
            (22 * 60, 6 * 60),
        )
        .until(date!(2026 - 12 - 31));
        let (verdict, findings) = assess_modul_3(&zzd, &full_context());
        assert_eq!(verdict, Modul3Conformance::Violates);
        assert!(
            findings.contains(&Modul3Finding::HochtarifBelowTwoHours),
            "{findings:?}"
        );

        // Exactly two hours is enough — the bound is inclusive.
        let exact = Zaehlzeitdefinition::modul_3(
            "NB-1",
            date!(2026 - 01 - 01),
            (17 * 60, 19 * 60),
            (22 * 60, 6 * 60),
        )
        .until(date!(2026 - 12 - 31));
        assert_eq!(
            assess_modul_3(&exact, &full_context()).0,
            Modul3Conformance::Conforms,
        );
    }

    /// A weekday-only Hochtarif offers none at all on a Sunday, which the
    /// per-day-class reading catches and a whole-week average would not.
    #[test]
    fn a_weekday_only_hochtarif_fails_on_sundays() {
        let mut zzd = conforming();
        for w in &mut zzd.windows {
            if w.register_id == HT {
                w.days = DayGroup::Weekdays;
            }
        }
        let (verdict, findings) = assess_modul_3(&zzd, &full_context());
        assert_eq!(verdict, Modul3Conformance::Violates);
        assert!(
            findings.contains(&Modul3Finding::HochtarifBelowTwoHours),
            "{findings:?}"
        );
    }

    /// *"Die Preisstufen und Zeitfenster müssen ganzjährig identisch sein."*
    #[test]
    fn a_seasonal_window_varies_across_the_year() {
        let mut zzd = conforming();
        // Winter months only — bits 0,1,10,11.
        for w in &mut zzd.windows {
            if w.register_id == HT {
                w.months_mask = 0b1100_0000_0011;
            }
        }
        let (verdict, findings) = assess_modul_3(&zzd, &full_context());
        assert_eq!(verdict, Modul3Conformance::Violates);
        assert!(
            findings.contains(&Modul3Finding::WindowsVaryAcrossTheYear),
            "{findings:?}"
        );
    }

    /// Without a fallback the Standardtarif has nowhere to book, so part of
    /// the day belongs to no register at all.
    #[test]
    fn a_missing_fallback_leaves_time_uncovered() {
        let mut zzd = conforming();
        zzd.fallback_register = None;
        let (verdict, findings) = assess_modul_3(&zzd, &full_context());
        assert_eq!(verdict, Modul3Conformance::Violates);
        assert!(
            findings.contains(&Modul3Finding::TimeNotFullyCovered),
            "{findings:?}"
        );
        assert!(
            findings.contains(&Modul3Finding::RegistersAreNotHtNtSt),
            "{findings:?}"
        );
    }

    /// *"in mindestens zwei Quartalen eines Jahres"* — and they need not be
    /// adjacent, so Q1 + Q4 passes while Q2 alone does not.
    #[test]
    fn two_quarters_are_required_but_need_not_be_adjacent() {
        let zzd = conforming();

        let one = full_context().billed_in([Quarter::Q2]);
        let (verdict, findings) = assess_modul_3(&zzd, &one);
        assert_eq!(verdict, Modul3Conformance::Violates);
        assert!(findings.contains(&Modul3Finding::FewerThanTwoBilledQuarters));

        // A repeated quarter is still one quarter.
        let repeated = full_context().billed_in([Quarter::Q2, Quarter::Q2]);
        assert_eq!(
            assess_modul_3(&zzd, &repeated).0,
            Modul3Conformance::Violates
        );

        // Non-adjacent is explicitly fine.
        let split = full_context().billed_in([Quarter::Q1, Quarter::Q3]);
        assert_eq!(assess_modul_3(&zzd, &split).0, Modul3Conformance::Conforms);
    }

    /// A missing input is `Unknown`, not `Conforms` — the same distinction
    /// `sharing` and the validation engine draw.
    #[test]
    fn missing_input_is_unknown_rather_than_clean() {
        let zzd = conforming();

        let (verdict, findings) = assess_modul_3(&zzd, &Modul3Context::default());
        assert_eq!(verdict, Modul3Conformance::Unknown);
        assert!(findings.contains(&Modul3Finding::BilledQuartersUnknown));
        assert!(findings.contains(&Modul3Finding::DeliveryPointDataMissing));
        assert!(findings.iter().copied().all(Modul3Finding::is_unknown));

        // A breach outranks an unknown.
        let partial = Modul3Context::default()
            .with_modul_1(false)
            .with_registrierende_leistungsmessung(false)
            .with_intelligentes_messsystem(true);
        let (verdict, findings) = assess_modul_3(&zzd, &partial);
        assert_eq!(verdict, Modul3Conformance::Violates);
        assert!(findings.contains(&Modul3Finding::Modul1NotSelected));
    }

    #[test]
    fn the_delivery_point_preconditions_are_each_reported() {
        let zzd = conforming();
        let ctx = Modul3Context::default()
            .billed_in([Quarter::Q1, Quarter::Q2])
            .with_modul_1(false)
            .with_registrierende_leistungsmessung(true)
            .with_intelligentes_messsystem(false);
        let (verdict, findings) = assess_modul_3(&zzd, &ctx);
        assert_eq!(verdict, Modul3Conformance::Violates);
        for expected in [
            Modul3Finding::Modul1NotSelected,
            Modul3Finding::RegistrierendeLeistungsmessung,
            Modul3Finding::NoIntelligentesMesssystem,
        ] {
            assert!(
                findings.contains(&expected),
                "{expected} missing: {findings:?}"
            );
        }
    }

    #[test]
    fn validity_must_describe_one_calendar_year() {
        let mid_year = Zaehlzeitdefinition::modul_3(
            "NB-1",
            date!(2026 - 04 - 01),
            (17 * 60, 20 * 60),
            (22 * 60, 6 * 60),
        );
        assert!(
            assess_modul_3(&mid_year, &full_context())
                .1
                .contains(&Modul3Finding::ValidityIsNotOneCalendarYear)
        );

        // An open end is "until further notice", not a breach.
        let open = Zaehlzeitdefinition::modul_3(
            "NB-1",
            date!(2026 - 01 - 01),
            (17 * 60, 20 * 60),
            (22 * 60, 6 * 60),
        );
        assert_eq!(
            assess_modul_3(&open, &full_context()).0,
            Modul3Conformance::Conforms
        );
    }

    #[test]
    fn quarters_partition_the_year() {
        let mut seen = Vec::new();
        for q in Quarter::ALL {
            for m in q.months() {
                assert_eq!(Quarter::of_month(m), Some(q));
                seen.push(m);
            }
        }
        seen.sort_unstable();
        assert_eq!(seen, (1u8..=12).collect::<Vec<_>>());
        assert_eq!(Quarter::of_month(0), None);
        assert_eq!(Quarter::of_month(13), None);
    }

    /// A classic HT/NT definition is not a Modul 3 definition, and says so.
    #[test]
    fn a_two_register_definition_is_not_modul_3() {
        let zzd = Zaehlzeitdefinition::ht_nt("NB-1", date!(2026 - 01 - 01), 6 * 60, 22 * 60);
        let (verdict, findings) = assess_modul_3(&zzd, &full_context());
        assert_eq!(verdict, Modul3Conformance::Violates);
        assert!(findings.contains(&Modul3Finding::RegistersAreNotHtNtSt));
    }

    /// Provenance: the NB-assigned id means nothing without whose it is.
    #[test]
    fn a_definition_can_name_the_netzbetreiber_that_published_it() {
        let nb: crate::BdewCode = "9900987654321".parse().unwrap();
        let zzd = conforming().published_by(nb);
        assert_eq!(zzd.netzbetreiber, Some(nb));
        assert_eq!(
            zzd.netzbetreiber.unwrap().vergabestelle(),
            crate::CodeVergabestelle::BdewStrom
        );
    }
}

#[cfg(test)]
mod modul_3_reachability_tests {
    use super::*;
    use time::macros::{date, datetime};

    fn ctx() -> Modul3Context {
        Modul3Context::default()
            .billed_in([Quarter::Q1, Quarter::Q4])
            .at_a_conforming_delivery_point()
    }

    /// The most likely import mistake there is: a Niedertarif band written as
    /// **one** wrapping window. `from > to` matches nothing, so NT is declared,
    /// never applied, and every other rule passes — the day is still fully
    /// covered by the ST fallback and HT is untouched. The calendar reads as a
    /// conforming three-level tariff with one level unreachable.
    #[test]
    fn a_wrapping_band_written_as_one_window_is_caught() {
        let broken = Zaehlzeitdefinition {
            id: "NB-BROKEN".to_owned(),
            valid_from: date!(2026 - 01 - 01),
            valid_to: Some(date!(2026 - 12 - 31)),
            windows: vec![
                ZaehlzeitFenster::new(HT, 17 * 60, 20 * 60),
                ZaehlzeitFenster::new(NT, 22 * 60, 6 * 60), // never matches
            ],
            fallback_register: Some(ST.to_owned()),
            holiday_land: None,
            netzbetreiber: None,
        };

        // Everything else about it looks right, which is why this needs saying.
        assert_eq!(broken.registers(), vec![HT, NT, ST]);
        assert_eq!(
            broken.register_for(datetime!(2026-01-05 22:00 UTC)),
            Some(ST),
            "23:00 local should be NT and is not",
        );

        let (verdict, findings) = assess_modul_3(&broken, &ctx());
        assert_eq!(verdict, Modul3Conformance::Violates);
        assert!(
            findings.contains(&Modul3Finding::RegisterNeverReached),
            "{findings:?}",
        );

        // Split into the two windows it needs, and it conforms.
        let fixed = Zaehlzeitdefinition {
            windows: {
                let mut w = vec![ZaehlzeitFenster::new(HT, 17 * 60, 20 * 60)];
                w.extend(ZaehlzeitFenster::spanning(NT, 22 * 60, 6 * 60));
                w
            },
            ..broken
        };
        assert_eq!(
            assess_modul_3(&fixed, &ctx()).0,
            Modul3Conformance::Conforms
        );
    }

    /// A month-restricted window that leaves another register to cover the gap
    /// varies across the year even though the Hochtarif does not move.
    /// Comparing only the HT and the uncovered minutes missed it.
    #[test]
    fn a_seasonal_swap_between_nt_and_st_varies_across_the_year() {
        let mut zzd = Zaehlzeitdefinition::modul_3(
            "NB-1",
            date!(2026 - 01 - 01),
            (17 * 60, 20 * 60),
            (22 * 60, 6 * 60),
        )
        .until(date!(2026 - 12 - 31));

        // Restrict only the Niedertarif to the winter half. HT is untouched and
        // the ST fallback still covers every minute, so neither the Hochtarif
        // total nor the uncovered total moves — only the NT/ST split does.
        for w in &mut zzd.windows {
            if w.register_id == NT {
                w.months_mask = 0b1100_0000_0011;
            }
        }

        let (verdict, findings) = assess_modul_3(&zzd, &ctx());
        assert_eq!(verdict, Modul3Conformance::Violates);
        assert!(
            findings.contains(&Modul3Finding::WindowsVaryAcrossTheYear),
            "{findings:?}",
        );
        assert!(
            !findings.contains(&Modul3Finding::HochtarifBelowTwoHours),
            "the Hochtarif itself is fine: {findings:?}",
        );
        assert!(
            !findings.contains(&Modul3Finding::TimeNotFullyCovered),
            "and every minute still books somewhere: {findings:?}",
        );
    }

    /// Reachability is judged over every month and day class, so a register
    /// that applies only on Sundays is still reachable.
    #[test]
    fn a_register_used_on_one_day_class_is_reachable() {
        let mut zzd = Zaehlzeitdefinition::modul_3(
            "NB-1",
            date!(2026 - 01 - 01),
            (17 * 60, 20 * 60),
            (22 * 60, 6 * 60),
        )
        .until(date!(2026 - 12 - 31));
        for w in &mut zzd.windows {
            if w.register_id == NT {
                w.days = DayGroup::Weekdays;
            }
        }
        let findings = assess_modul_3(&zzd, &ctx()).1;
        assert!(
            !findings.contains(&Modul3Finding::RegisterNeverReached),
            "{findings:?}",
        );
        // …though restricting it that way does make the year vary by day class,
        // which is a different question the profile comparison does not ask.
        assert!(!findings.contains(&Modul3Finding::WindowsVaryAcrossTheYear));
    }
}
