//! Zählzeitdefinition — resolving a timestamp to a tariff register.
//!
//! A definition is ordered windows over (months × day group × time band), each
//! naming a register, plus a fallback. That covers the classic **Zweitarif**
//! ([`Zaehlzeitdefinition::ht_nt`]), **§ 14a EnWG Modul 3**
//! ([`Zaehlzeitdefinition::modul_3`], checked by [`assess_modul_3`]) and any
//! definition a Netzbetreiber transmits over UTILTS.
//!
//! Timestamps are matched in Europe/Berlin local time, so band boundaries are
//! DST-correct. A gesetzlicher Feiertag can be treated as a non-working day via
//! [`Zaehlzeitdefinition::holiday_land`]. Energy is split into registers by
//! [`Zaehlzeitdefinition::split_energy`].
//!
//! ## Example — a §14a Modul 3 definition
//!
//! ```rust
//! use metering::billing::zaehlzeit::{HT, NT, ST, Quarter, Zaehlzeitdefinition};
//! use time::macros::{date, datetime};
//!
//! // HT 17:00–20:00, NT 00:00–06:00, billed in Q1 and Q4; ST for the rest.
//! let zzd = Zaehlzeitdefinition::modul_3(
//!     "NB-14A-3",
//!     date!(2026 - 01 - 01),
//!     (17 * 60, 20 * 60),
//!     (0, 6 * 60),
//!     &[Quarter::Q1, Quarter::Q4],
//! )?;
//!
//! // Monday 18:00 Berlin in January → Hochtarif.
//! assert_eq!(zzd.register_for(datetime!(2026-01-05 17:00 UTC)), Some(HT));
//! // Monday 03:00 Berlin → Niedertarif.
//! assert_eq!(zzd.register_for(datetime!(2026-01-05 2:00 UTC)), Some(NT));
//! // The same 18:00 in July is outside the billed quarters → Standardtarif.
//! assert_eq!(zzd.register_for(datetime!(2026-07-06 16:00 UTC)), Some(ST));
//! # Ok::<(), metering::billing::zaehlzeit::ZaehlzeitError>(())
//! ```

use std::collections::{BTreeMap, BTreeSet};

use rust_decimal::Decimal;
use time::{Date, OffsetDateTime, Weekday};
use time_tz::{OffsetDateTimeExt as _, timezones};

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use crate::ids::BdewCode;
use crate::series::interval::MeterInterval;
use crate::time::holiday::Bundesland;

/// Hochtarif — the conventional register id for the peak band.
pub const HT: &str = "HT";
/// Niedertarif — the conventional register id for the off-peak band.
pub const NT: &str = "NT";
/// Standardtarif — the § 14a Modul 3 band for everything that is neither.
pub const ST: &str = "ST";

/// All twelve months active in a window's month mask (bit 0 = January).
pub const ALL_MONTHS: u16 = 0x0FFF;

/// Minutes in a day, the inclusive upper bound of a window's end.
pub const MINUTES_PER_DAY: u16 = 24 * 60;

/// Why a window or a definition was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum ZaehlzeitError {
    /// A window needs `from < to ≤ 1440` minutes since local midnight.
    #[error("window {from}–{to} is not inside one day (from < to ≤ 1440)")]
    Bounds {
        /// Start, minutes since local midnight.
        from: u16,
        /// End, minutes since local midnight.
        to: u16,
    },
    /// A month mask uses bits above the twelfth month, or none at all.
    #[error("month mask {0:#06x} is not a non-empty set of the twelve months")]
    MonthMask(u16),
    /// A Modul 3 definition names no billed quarter.
    #[error("a Modul 3 definition needs at least one billed quarter")]
    NoBilledQuarter,
}

/// Which days of the week a window applies to (holidays:
/// [`Zaehlzeitdefinition::holiday_land`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
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

crate::ids::codes::string_codes! {
    DayGroup;
}

/// One validated window of a Zählzeitdefinition.
///
/// Bounds are **minutes since local midnight**, `[from, to)`. A band crossing
/// midnight is two windows; [`spanning`](Self::spanning) builds them.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(try_from = "RawFenster", into = "RawFenster"))]
pub struct ZaehlzeitFenster {
    register_id: String,
    months_mask: u16,
    days: DayGroup,
    from_minute: u16,
    to_minute: u16,
}

impl ZaehlzeitFenster {
    /// An all-year, all-week window over `[from_minute, to_minute)`.
    ///
    /// # Errors
    ///
    /// [`ZaehlzeitError::Bounds`] unless `from_minute < to_minute ≤ 1440`; for
    /// a band like `22:00–06:00` use [`spanning`](Self::spanning).
    pub fn new(
        register_id: impl Into<String>,
        from_minute: u16,
        to_minute: u16,
    ) -> Result<Self, ZaehlzeitError> {
        if from_minute >= to_minute || to_minute > MINUTES_PER_DAY {
            return Err(ZaehlzeitError::Bounds {
                from: from_minute,
                to: to_minute,
            });
        }
        Ok(Self {
            register_id: register_id.into(),
            months_mask: ALL_MONTHS,
            days: DayGroup::AllDays,
            from_minute,
            to_minute,
        })
    }

    /// The one or two windows covering a band from `from_minute` to
    /// `to_minute`, splitting at midnight when the band wraps; equal bounds are
    /// the whole day.
    ///
    /// ```rust
    /// use metering::billing::zaehlzeit::{NT, ZaehlzeitFenster};
    ///
    /// assert_eq!(ZaehlzeitFenster::spanning(NT, 22 * 60, 6 * 60)?.len(), 2);
    /// assert_eq!(ZaehlzeitFenster::spanning(NT, 6 * 60, 22 * 60)?.len(), 1);
    /// assert!(ZaehlzeitFenster::spanning(NT, 6 * 60, 1500).is_err());
    /// # Ok::<(), metering::billing::zaehlzeit::ZaehlzeitError>(())
    /// ```
    ///
    /// # Errors
    ///
    /// [`ZaehlzeitError::Bounds`] when a bound lies beyond 24:00, or the start
    /// is 24:00.
    pub fn spanning(
        register_id: impl Into<String>,
        from_minute: u16,
        to_minute: u16,
    ) -> Result<Vec<Self>, ZaehlzeitError> {
        let id = register_id.into();
        if from_minute >= MINUTES_PER_DAY || to_minute > MINUTES_PER_DAY {
            return Err(ZaehlzeitError::Bounds {
                from: from_minute,
                to: to_minute,
            });
        }
        if from_minute < to_minute {
            return Ok(vec![Self::new(id, from_minute, to_minute)?]);
        }
        if from_minute == to_minute {
            return Ok(vec![Self::new(id, 0, MINUTES_PER_DAY)?]);
        }
        let mut out = vec![Self::new(id.clone(), from_minute, MINUTES_PER_DAY)?];
        if to_minute > 0 {
            out.push(Self::new(id, 0, to_minute)?);
        }
        Ok(out)
    }

    /// Restrict the window to a day group (builder style).
    #[must_use]
    pub const fn on_days(mut self, days: DayGroup) -> Self {
        self.days = days;
        self
    }

    /// Restrict the window to the months in `mask`, bit 0 = January.
    ///
    /// # Errors
    ///
    /// [`ZaehlzeitError::MonthMask`] for an empty mask or one with bits above
    /// December.
    pub fn in_months(mut self, mask: u16) -> Result<Self, ZaehlzeitError> {
        if mask == 0 || mask & !ALL_MONTHS != 0 {
            return Err(ZaehlzeitError::MonthMask(mask));
        }
        self.months_mask = mask;
        Ok(self)
    }

    /// The register this window books into.
    #[must_use]
    pub fn register_id(&self) -> &str {
        &self.register_id
    }

    /// The months the window is active in, bit 0 = January.
    #[must_use]
    pub const fn months_mask(&self) -> u16 {
        self.months_mask
    }

    /// The day group the window applies to.
    #[must_use]
    pub const fn days(&self) -> DayGroup {
        self.days
    }

    /// Start, minutes since local midnight (inclusive).
    #[must_use]
    pub const fn from_minute(&self) -> u16 {
        self.from_minute
    }

    /// End, minutes since local midnight (exclusive).
    #[must_use]
    pub const fn to_minute(&self) -> u16 {
        self.to_minute
    }

    /// `true` when the Berlin-local (month, day group, minute) falls inside.
    ///
    /// A holiday matches only [`DayGroup::AllDays`] windows.
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

/// The unvalidated wire shape of a window.
#[cfg(feature = "serde")]
#[derive(Serialize, Deserialize)]
struct RawFenster {
    register_id: String,
    months_mask: u16,
    days: DayGroup,
    from_minute: u16,
    to_minute: u16,
}

#[cfg(feature = "serde")]
impl TryFrom<RawFenster> for ZaehlzeitFenster {
    type Error = ZaehlzeitError;

    fn try_from(raw: RawFenster) -> Result<Self, Self::Error> {
        Self::new(raw.register_id, raw.from_minute, raw.to_minute)?
            .on_days(raw.days)
            .in_months(raw.months_mask)
    }
}

#[cfg(feature = "serde")]
impl From<ZaehlzeitFenster> for RawFenster {
    fn from(w: ZaehlzeitFenster) -> Self {
        Self {
            register_id: w.register_id,
            months_mask: w.months_mask,
            days: w.days,
            from_minute: w.from_minute,
            to_minute: w.to_minute,
        }
    }
}

/// A named Zählzeitdefinition with validity and ordered windows.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[non_exhaustive]
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
    pub fallback_register: Option<String>,
    /// Bundesland whose statutory holidays are treated as non-working days.
    /// `None` classifies by weekday alone.
    pub holiday_land: Option<Bundesland>,
    /// Marktpartner-ID of the publishing Netzbetreiber; [`id`](Self::id) is
    /// unique only within that operator.
    pub netzbetreiber: Option<BdewCode>,
}

impl Zaehlzeitdefinition {
    /// A definition from its windows and fallback, valid from `valid_from`.
    #[must_use]
    pub fn new(
        id: impl Into<String>,
        valid_from: Date,
        windows: Vec<ZaehlzeitFenster>,
        fallback_register: Option<String>,
    ) -> Self {
        Self {
            id: id.into(),
            valid_from,
            valid_to: None,
            windows,
            fallback_register,
            holiday_land: None,
            netzbetreiber: None,
        }
    }

    /// The classic Zweitarif: [`HT`] inside the band on weekdays, [`NT`]
    /// everywhere else.
    ///
    /// The band is Berlin local time and may cross midnight; each
    /// Netzbetreiber sets its own.
    ///
    /// ```rust
    /// use metering::billing::zaehlzeit::{HT, NT, Zaehlzeitdefinition};
    /// use time::macros::{date, datetime};
    ///
    /// let zzd = Zaehlzeitdefinition::ht_nt("NB-1", date!(2026 - 01 - 01), 6 * 60, 22 * 60)?;
    /// assert_eq!(zzd.register_for(datetime!(2026-01-05 8:00 UTC)), Some(HT));
    /// assert_eq!(zzd.register_for(datetime!(2026-01-05 21:00 UTC)), Some(NT));
    /// assert_eq!(zzd.register_for(datetime!(2026-01-04 11:00 UTC)), Some(NT));
    /// # Ok::<(), metering::billing::zaehlzeit::ZaehlzeitError>(())
    /// ```
    ///
    /// # Errors
    ///
    /// [`ZaehlzeitError::Bounds`] for a bound beyond 24:00.
    pub fn ht_nt(
        id: impl Into<String>,
        valid_from: Date,
        from_minute: u16,
        to_minute: u16,
    ) -> Result<Self, ZaehlzeitError> {
        let windows = ZaehlzeitFenster::spanning(HT, from_minute, to_minute)?
            .into_iter()
            .map(|w| w.on_days(DayGroup::Weekdays))
            .collect();
        Ok(Self::new(id, valid_from, windows, Some(NT.to_owned())))
    }

    /// A § 14a EnWG **Modul 3** definition: [`HT`] and [`NT`] in the quarters
    /// the Netzbetreiber bills them, [`ST`] everywhere else.
    ///
    /// BDEW AWH Modul 3 v1.1 (07.02.2025) § 2: *"Der Netzbetreiber hat das
    /// Wahlrecht, den Gültigkeitszeitraum auf einzelne Quartale zu
    /// beschränken"*. The bands apply on **all days**, in Berlin local time,
    /// and may cross midnight; an HT/NT overlap resolves to HT.
    ///
    /// # Errors
    ///
    /// [`ZaehlzeitError::Bounds`] for a bound beyond 24:00,
    /// [`ZaehlzeitError::NoBilledQuarter`] for an empty quarter list.
    pub fn modul_3(
        id: impl Into<String>,
        valid_from: Date,
        hochtarif: (u16, u16),
        niedertarif: (u16, u16),
        billed_quarters: &[Quarter],
    ) -> Result<Self, ZaehlzeitError> {
        let mask = billed_quarters.iter().fold(0u16, |m, q| m | q.month_mask());
        if mask == 0 {
            return Err(ZaehlzeitError::NoBilledQuarter);
        }
        let mut windows = Vec::new();
        for (register, (from, to)) in [(HT, hochtarif), (NT, niedertarif)] {
            for w in ZaehlzeitFenster::spanning(register, from, to)? {
                windows.push(w.in_months(mask)?);
            }
        }
        Ok(Self::new(id, valid_from, windows, Some(ST.to_owned())))
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

    /// Every register this definition can book into, sorted and deduplicated —
    /// including the fallback, which appears in no window.
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
    /// Matched in Europe/Berlin local time. `None` when the definition is not
    /// valid on that day, or no window and no fallback covers the time.
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

    /// `true` when the register changes somewhere inside `[from, to)`.
    ///
    /// Exact: registers change only on whole minutes (minute bounds, whole-hour
    /// Berlin offsets).
    fn straddles(&self, from: OffsetDateTime, to: OffsetDateTime) -> bool {
        let first = self.register_for(from);
        let next_minute = (from.unix_timestamp().div_euclid(60) + 1) * 60;
        let Ok(mut at) = OffsetDateTime::from_unix_timestamp(next_minute) else {
            return true;
        };
        while at < to {
            if self.register_for(at) != first {
                return true;
            }
            at += time::Duration::MINUTE;
        }
        false
    }

    /// Split intervals into per-register sums, reporting what could not be
    /// split: a billable interval wholly in one register goes to
    /// [`per_register`](RegisterSplit::per_register), one crossing a register
    /// change to [`straddling`](RegisterSplit::straddling), one no register
    /// covers to [`unassigned`](RegisterSplit::unassigned), and a non-billable
    /// one to [`non_billable`](RegisterSplit::non_billable).
    ///
    /// `None` only when a register sum overflows.
    ///
    /// ```rust
    /// use metering::billing::zaehlzeit::{HT, Zaehlzeitdefinition};
    /// use metering::{MeterInterval, QualityFlag};
    /// use rust_decimal::dec;
    /// use time::macros::{date, datetime};
    ///
    /// let zzd = Zaehlzeitdefinition::ht_nt("NB-1", date!(2026 - 01 - 01), 6 * 60, 22 * 60)?;
    /// let midday = MeterInterval::quarter_hour(datetime!(2026-01-05 9:00 UTC), dec!(3), QualityFlag::Measured)?;
    /// // An hour across 06:00 local holds both registers.
    /// let across = MeterInterval::hour(datetime!(2026-01-05 4:30 UTC), dec!(1), QualityFlag::Measured)?;
    ///
    /// let split = zzd.split_energy(&[midday, across]).unwrap();
    /// assert_eq!(split.per_register[HT], dec!(3));
    /// assert_eq!(split.straddling.len(), 1);
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    #[must_use]
    pub fn split_energy<'a>(&'a self, intervals: &[MeterInterval]) -> Option<RegisterSplit<'a>> {
        let mut split = RegisterSplit {
            per_register: BTreeMap::new(),
            straddling: Vec::new(),
            unassigned: Vec::new(),
            non_billable: Vec::new(),
        };
        for iv in intervals {
            if !iv.quality().is_billable() {
                split.non_billable.push(iv.clone());
                continue;
            }
            if self.straddles(iv.from(), iv.to()) {
                split.straddling.push(iv.clone());
                continue;
            }
            match self.register_for(iv.from()) {
                Some(register) => {
                    let sum = split.per_register.entry(register).or_insert(Decimal::ZERO);
                    *sum = sum.checked_add(iv.value())?;
                }
                None => split.unassigned.push(iv.clone()),
            }
        }
        Some(split)
    }
}

/// The outcome of [`Zaehlzeitdefinition::split_energy`].
///
/// `Σ per_register + Σ straddling + Σ unassigned` is the billable energy of the
/// input; the keys are strings from
/// [`registers`](Zaehlzeitdefinition::registers).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct RegisterSplit<'a> {
    /// Billable energy per register, for intervals wholly inside one.
    pub per_register: BTreeMap<&'a str, Decimal>,
    /// Billable intervals whose span crosses a register change. Resample finer
    /// or split them at the boundary before booking them.
    pub straddling: Vec<MeterInterval>,
    /// Billable intervals no register covers.
    pub unassigned: Vec<MeterInterval>,
    /// Intervals excluded as non-billable.
    pub non_billable: Vec<MeterInterval>,
}

impl RegisterSplit<'_> {
    /// `true` when every billable interval landed in a register.
    #[must_use]
    pub fn is_complete(&self) -> bool {
        self.straddling.is_empty() && self.unassigned.is_empty()
    }
}

// ── Modul 3 conformance ──────────────────────────────────────────────────────

/// A calendar quarter, January to March being the first.
///
/// BDEW *Anwendungshilfe für die Umsetzung von Modul 3* v1.1 (07.02.2025),
/// §2: *"Dabei wird das Jahr in kalenderjährliche Quartale beginnend mit
/// Januar unterteilt."*
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
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

    /// The quarter's months as a window month mask (bit 0 = January).
    #[must_use]
    pub const fn month_mask(self) -> u16 {
        0b111 << (3 * (self as u16))
    }
}

/// Shortest Hochtarif window the Modul 3 rules admit, in minutes.
///
/// BDEW AWH Modul 3 v1.1, §2: *"Hochlasttarif (HT): min. an 2 Stunden pro
/// Tag"*.
pub const MODUL_3_MIN_HOCHTARIF_MINUTES: u16 = 120;

/// Fewest calendar quarters the tariffs must be billed in.
///
/// BDEW AWH Modul 3 v1.1, §2: *"Die Zeitfenster und insofern die drei
/// Netzentgelttarife müssen in mindestens zwei Quartalen eines Jahres
/// abgerechnet werden."*
pub const MODUL_3_MIN_BILLED_QUARTERS: usize = 2;

/// Why a Zählzeitdefinition does not meet the Modul 3 rules.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Modul3Finding {
    /// The definition does not book into exactly [`HT`], [`NT`] and [`ST`].
    RegistersAreNotHtNtSt,
    /// A register is declared but no instant of any day ever resolves to it.
    RegisterNeverReached,
    /// Some part of the day falls into no register at all.
    TimeNotFullyCovered,
    /// The Hochtarif window is shorter than
    /// [`MODUL_3_MIN_HOCHTARIF_MINUTES`] on a day class of a billed month.
    HochtarifBelowTwoHours,
    /// The windows differ between billed months: *"Die Preisstufen und
    /// Zeitfenster müssen ganzjährig identisch sein"*.
    WindowsVaryAcrossTheYear,
    /// The billed months do not make up whole calendar quarters — the
    /// Wahlrecht restricts the validity *"auf einzelne Quartale"*, not months.
    BilledMonthsSplitAQuarter,
    /// Fewer than [`MODUL_3_MIN_BILLED_QUARTERS`] quarters are billed.
    FewerThanTwoBilledQuarters,
    /// Validity does not describe one calendar year.
    ValidityIsNotOneCalendarYear,
    /// The delivery point has not selected Modul 1.
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
        Self::BilledMonthsSplitAQuarter,
        Self::FewerThanTwoBilledQuarters,
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
            Self::BilledMonthsSplitAQuarter => "BILLED_MONTHS_SPLIT_A_QUARTER",
            Self::FewerThanTwoBilledQuarters => "FEWER_THAN_TWO_BILLED_QUARTERS",
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
            Self::DeliveryPointDataMissing => "(not a rule — the input did not say)",
            _ => "BDEW AWH Modul 3 v1.1 (07.02.2025) §2",
        }
    }

    /// `true` when the finding is about missing input rather than a breach.
    #[must_use]
    pub const fn is_unknown(self) -> bool {
        matches!(self, Self::DeliveryPointDataMissing)
    }
}

/// Whether a Zählzeitdefinition meets the Modul 3 rules.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
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

crate::ids::codes::string_codes! {
    Quarter;
    Modul3Finding;
    Modul3Conformance;
}

/// The delivery-point facts [`assess_modul_3`] needs beyond the windows. A
/// fact left unset is reported as
/// [`Modul3Finding::DeliveryPointDataMissing`], not guessed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct Modul3Context {
    modul_1_selected: Option<bool>,
    registrierende_leistungsmessung: Option<bool>,
    intelligentes_messsystem: Option<bool>,
}

impl Modul3Context {
    /// Nothing known yet.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            modul_1_selected: None,
            registrierende_leistungsmessung: None,
            intelligentes_messsystem: None,
        }
    }

    /// Whether the delivery point has selected Modul 1 (required).
    #[must_use]
    pub const fn modul_1(mut self, selected: bool) -> Self {
        self.modul_1_selected = Some(selected);
        self
    }

    /// Whether the Marktlokation is metered by registrierende
    /// Leistungsmessung — Modul 3 requires that it is **not**.
    #[must_use]
    pub const fn registrierende_leistungsmessung(mut self, rlm: bool) -> Self {
        self.registrierende_leistungsmessung = Some(rlm);
        self
    }

    /// Whether an intelligentes Messsystem is installed (required).
    #[must_use]
    pub const fn intelligentes_messsystem(mut self, imsys: bool) -> Self {
        self.intelligentes_messsystem = Some(imsys);
        self
    }

    /// The three preconditions a conforming point satisfies: Modul 1
    /// selected, no RLM, iMSys installed.
    #[must_use]
    pub const fn at_a_conforming_delivery_point(self) -> Self {
        self.modul_1(true)
            .registrierende_leistungsmessung(false)
            .intelligentes_messsystem(true)
    }
}

/// The outcome of [`assess_modul_3`].
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Modul3Assessment {
    /// The verdict: a breach outranks an unknown.
    pub verdict: Modul3Conformance,
    /// Every finding, in rule order.
    pub findings: Vec<Modul3Finding>,
    /// The quarters in which the definition books into a register other than
    /// [`ST`] — the quarters it bills, read from the windows themselves.
    pub billed_quarters: Vec<Quarter>,
}

/// Assess a Zählzeitdefinition against the § 14a **Modul 3** rules.
///
/// | Rule (BDEW AWH Modul 3 v1.1 §2) | Finding |
/// |---|---|
/// | three Netzentgelttarife HT/NT/ST | [`RegistersAreNotHtNtSt`] |
/// | each of them reachable | [`RegisterNeverReached`] |
/// | every instant books into one of them | [`TimeNotFullyCovered`] |
/// | HT at least two hours per day | [`HochtarifBelowTwoHours`] |
/// | windows *ganzjährig identisch* across the billed quarters | [`WindowsVaryAcrossTheYear`] |
/// | validity restricted to *einzelne Quartale* | [`BilledMonthsSplitAQuarter`] |
/// | billed in at least two quarters | [`FewerThanTwoBilledQuarters`] |
/// | set per calendar year | [`ValidityIsNotOneCalendarYear`] |
/// | only with Modul 1, iMSys, no RLM | from [`Modul3Context`] |
///
/// A month is **billed** when any minute of any day class books into a
/// register other than [`ST`] (read from the same windows
/// [`register_for`](Zaehlzeitdefinition::register_for) uses). Day profiles are
/// compared only across billed months, so an all-ST Q2/Q3 conforms. *"min. an
/// 2 Stunden pro Tag"* is checked per day class: weekday, Saturday, Sunday,
/// and holiday where [`holiday_land`](Zaehlzeitdefinition::holiday_land) is
/// set. Price corridors and the publication deadline are not checked.
///
/// [`RegistersAreNotHtNtSt`]: Modul3Finding::RegistersAreNotHtNtSt
/// [`RegisterNeverReached`]: Modul3Finding::RegisterNeverReached
/// [`TimeNotFullyCovered`]: Modul3Finding::TimeNotFullyCovered
/// [`HochtarifBelowTwoHours`]: Modul3Finding::HochtarifBelowTwoHours
/// [`WindowsVaryAcrossTheYear`]: Modul3Finding::WindowsVaryAcrossTheYear
/// [`BilledMonthsSplitAQuarter`]: Modul3Finding::BilledMonthsSplitAQuarter
/// [`FewerThanTwoBilledQuarters`]: Modul3Finding::FewerThanTwoBilledQuarters
/// [`ValidityIsNotOneCalendarYear`]: Modul3Finding::ValidityIsNotOneCalendarYear
///
/// ```rust
/// use metering::billing::zaehlzeit::{
///     Modul3Conformance, Modul3Context, Quarter, Zaehlzeitdefinition, assess_modul_3,
/// };
/// use time::macros::date;
///
/// let zzd = Zaehlzeitdefinition::modul_3(
///     "NB-14A-3",
///     date!(2026 - 01 - 01),
///     (17 * 60, 20 * 60), // Hochtarif 17:00–20:00
///     (22 * 60, 6 * 60),  // Niedertarif 22:00–06:00, wrapping
///     &[Quarter::Q1, Quarter::Q4],
/// )?
/// .until(date!(2026 - 12 - 31));
///
/// let ctx = Modul3Context::new().at_a_conforming_delivery_point();
/// let a = assess_modul_3(&zzd, &ctx);
/// assert_eq!(a.verdict, Modul3Conformance::Conforms, "{:?}", a.findings);
/// assert_eq!(a.billed_quarters, [Quarter::Q1, Quarter::Q4]);
/// # Ok::<(), metering::billing::zaehlzeit::ZaehlzeitError>(())
/// ```
#[must_use]
pub fn assess_modul_3(zzd: &Zaehlzeitdefinition, ctx: &Modul3Context) -> Modul3Assessment {
    let mut findings = Vec::new();

    if zzd.registers() != vec![HT, NT, ST] {
        findings.push(Modul3Finding::RegistersAreNotHtNtSt);
    }

    let classes = day_classes(zzd);
    let months: Vec<Vec<DayProfile<'_>>> = (0u8..12)
        .map(|month0| {
            classes
                .iter()
                .map(|&class| day_profile(zzd, month0, class))
                .collect()
        })
        .collect();
    let billed_month = |profiles: &Vec<DayProfile<'_>>| {
        profiles
            .iter()
            .any(|p| p.keys().any(|r| r.is_some_and(|r| r != ST)))
    };
    let billed: Vec<bool> = months.iter().map(billed_month).collect();

    if months.iter().flatten().any(|p| p.contains_key(&None)) {
        findings.push(Modul3Finding::TimeNotFullyCovered);
    }

    let mut billed_profiles = months
        .iter()
        .zip(&billed)
        .filter(|(_, b)| **b)
        .map(|(p, _)| p);
    let first = billed_profiles.next();
    let minutes = |profile: &DayProfile<'_>, register: &str| -> u16 {
        profile.get(&Some(register)).copied().unwrap_or(0)
    };
    if first.is_none_or(|f| {
        f.iter()
            .any(|p| minutes(p, HT) < MODUL_3_MIN_HOCHTARIF_MINUTES)
    }) {
        findings.push(Modul3Finding::HochtarifBelowTwoHours);
    }
    if let Some(first) = first
        && !billed_profiles.all(|other| other == first)
    {
        findings.push(Modul3Finding::WindowsVaryAcrossTheYear);
    }

    // A register nobody can ever be charged is not one of the three levels.
    let reachable: BTreeSet<&str> = months
        .iter()
        .flatten()
        .flat_map(|p| p.keys().copied())
        .flatten()
        .collect();
    if zzd.registers().iter().any(|r| !reachable.contains(r)) {
        findings.push(Modul3Finding::RegisterNeverReached);
    }

    let mut billed_quarters = Vec::new();
    let mut split = false;
    for q in Quarter::ALL {
        let n = q
            .months()
            .iter()
            .filter(|m| billed[usize::from(**m) - 1])
            .count();
        match n {
            3 => billed_quarters.push(q),
            0 => {}
            _ => split = true,
        }
    }
    if split {
        findings.push(Modul3Finding::BilledMonthsSplitAQuarter);
    }
    if billed_quarters.len() < MODUL_3_MIN_BILLED_QUARTERS {
        findings.push(Modul3Finding::FewerThanTwoBilledQuarters);
    }

    // An open end is "until further notice" and not itself a breach.
    let starts_a_year = zzd.valid_from.month() == time::Month::January && zzd.valid_from.day() == 1;
    let ends_that_year = zzd.valid_to.is_none_or(|end| {
        end.year() == zzd.valid_from.year()
            && end.month() == time::Month::December
            && end.day() == 31
    });
    if !starts_a_year || !ends_that_year {
        findings.push(Modul3Finding::ValidityIsNotOneCalendarYear);
    }

    let mut missing = false;
    for (value, required, breach) in [
        (ctx.modul_1_selected, true, Modul3Finding::Modul1NotSelected),
        (
            ctx.registrierende_leistungsmessung,
            false,
            Modul3Finding::RegistrierendeLeistungsmessung,
        ),
        (
            ctx.intelligentes_messsystem,
            true,
            Modul3Finding::NoIntelligentesMesssystem,
        ),
    ] {
        match value {
            Some(v) if v == required => {}
            Some(_) => findings.push(breach),
            None => missing = true,
        }
    }
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
    Modul3Assessment {
        verdict,
        findings,
        billed_quarters,
    }
}

/// Wall-clock minutes per register for one kind of day; `None` keys the
/// uncovered minutes.
type DayProfile<'a> = BTreeMap<Option<&'a str>, u16>;

/// The kinds of day a definition can tell apart: a weekday, a Saturday, a
/// Sunday, and a holiday where a [`Bundesland`] was named.
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

/// Resolve one (month, day class) into its register profile, once per segment
/// between window bounds — exact, since matching is piecewise constant.
fn day_profile(zzd: &Zaehlzeitdefinition, month0: u8, class: (Weekday, bool)) -> DayProfile<'_> {
    let (weekday, is_holiday) = class;
    let mut breakpoints: Vec<u16> = vec![0, MINUTES_PER_DAY];
    for w in &zzd.windows {
        breakpoints.push(w.from_minute);
        breakpoints.push(w.to_minute);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::series::interval::QualityFlag;
    use rust_decimal::dec;
    use time::macros::{date, datetime};

    const ALL_YEAR: [Quarter; 4] = Quarter::ALL;

    fn iv(from: OffsetDateTime, kwh: Decimal) -> MeterInterval {
        MeterInterval::quarter_hour(from, kwh, QualityFlag::Measured).unwrap()
    }

    fn ht_nt() -> Zaehlzeitdefinition {
        Zaehlzeitdefinition::ht_nt("NB-1", date!(2026 - 01 - 01), 6 * 60, 22 * 60).unwrap()
    }

    fn modul_3(quarters: &[Quarter]) -> Zaehlzeitdefinition {
        Zaehlzeitdefinition::modul_3(
            "NB-14A-3",
            date!(2026 - 01 - 01),
            (17 * 60, 20 * 60),
            (22 * 60, 6 * 60),
            quarters,
        )
        .unwrap()
        .until(date!(2026 - 12 - 31))
    }

    fn day(date: Date, kwh: Decimal) -> Vec<MeterInterval> {
        let p = crate::DayBoundary::Strom.day(date).unwrap();
        let n = p.count(crate::Resolution::QUARTER_HOUR).unwrap();
        (0..i64::from(n))
            .map(|i| iv(p.start() + time::Duration::minutes(15 * i), kwh))
            .collect()
    }

    /// A window is validated at construction: `from < to ≤ 1440`, a month
    /// mask inside twelve bits.
    #[test]
    fn a_window_is_validated() {
        assert!(ZaehlzeitFenster::new(HT, 0, MINUTES_PER_DAY).is_ok());
        assert_eq!(
            ZaehlzeitFenster::new(NT, 22 * 60, 6 * 60),
            Err(ZaehlzeitError::Bounds {
                from: 1320,
                to: 360
            })
        );
        assert!(ZaehlzeitFenster::new(HT, 60, 60).is_err());
        assert!(ZaehlzeitFenster::new(HT, 60, 1441).is_err());
        let w = ZaehlzeitFenster::new(HT, 60, 120).unwrap();
        assert_eq!(
            w.clone().in_months(0x1000),
            Err(ZaehlzeitError::MonthMask(0x1000))
        );
        assert_eq!(w.clone().in_months(0), Err(ZaehlzeitError::MonthMask(0)));
        assert_eq!(w.in_months(0b11).unwrap().months_mask(), 0b11);
        assert!(ZaehlzeitFenster::spanning(NT, 1440, 60).is_err());
        assert!(ZaehlzeitFenster::spanning(NT, 60, 1441).is_err());
    }

    #[test]
    fn a_band_crossing_midnight_becomes_two_windows() {
        let w = ZaehlzeitFenster::spanning(NT, 22 * 60, 6 * 60).unwrap();
        assert_eq!(w.len(), 2);
        assert_eq!((w[0].from_minute(), w[0].to_minute()), (1320, 1440));
        assert_eq!((w[1].from_minute(), w[1].to_minute()), (0, 360));
        // 22:00–00:00 is one window, not a second empty one.
        assert_eq!(ZaehlzeitFenster::spanning(NT, 22 * 60, 0).unwrap().len(), 1);
        let whole = ZaehlzeitFenster::spanning(NT, 0, 0).unwrap();
        assert_eq!((whole[0].from_minute(), whole[0].to_minute()), (0, 1440));
    }

    #[test]
    fn quarter_month_masks_partition_the_year() {
        let all = Quarter::ALL.iter().fold(0u16, |m, q| m | q.month_mask());
        assert_eq!(all, ALL_MONTHS);
        assert_eq!(Quarter::Q1.month_mask(), 0b111);
        assert_eq!(Quarter::Q4.month_mask(), 0b1110_0000_0000);
        for q in Quarter::ALL {
            for m in q.months() {
                assert_eq!(Quarter::of_month(m), Some(q));
                assert_ne!(q.month_mask() & (1 << (m - 1)), 0);
            }
        }
    }

    #[test]
    fn ht_nt_resolves_in_berlin_local_time() {
        let zzd = ht_nt();
        assert_eq!(zzd.register_for(datetime!(2026-01-05 8:00 UTC)), Some(HT));
        assert_eq!(zzd.register_for(datetime!(2026-01-05 21:00 UTC)), Some(NT));
        assert_eq!(zzd.register_for(datetime!(2026-01-05 20:59 UTC)), Some(HT));
        assert_eq!(zzd.register_for(datetime!(2026-06-01 7:00 UTC)), Some(HT));
        assert_eq!(zzd.register_for(datetime!(2026-06-01 20:01 UTC)), Some(NT));
        // Weekends fall to the fallback.
        assert_eq!(zzd.register_for(datetime!(2026-01-03 9:00 UTC)), Some(NT));
    }

    /// 23:30 UTC on a Friday is already Saturday in Berlin.
    #[test]
    fn the_weekday_is_read_in_berlin_too() {
        let zzd = Zaehlzeitdefinition::new(
            "NB-1",
            date!(2026 - 01 - 01),
            vec![
                ZaehlzeitFenster::new(HT, 0, MINUTES_PER_DAY)
                    .unwrap()
                    .on_days(DayGroup::Weekdays),
            ],
            Some(NT.to_owned()),
        );
        assert_eq!(zzd.register_for(datetime!(2026-01-02 23:30 UTC)), Some(NT));
        assert_eq!(zzd.register_for(datetime!(2026-01-04 23:30 UTC)), Some(HT));
    }

    #[test]
    fn a_feiertag_books_into_the_fallback_register() {
        let midday = datetime!(2026-06-04 8:00 UTC); // Fronleichnam, a Thursday
        assert_eq!(ht_nt().register_for(midday), Some(HT));
        assert_eq!(
            ht_nt().in_land(Bundesland::By).register_for(midday),
            Some(NT)
        );
        assert_eq!(
            ht_nt().in_land(Bundesland::Be).register_for(midday),
            Some(HT)
        );
    }

    #[test]
    fn validity_bounds_are_inclusive() {
        let zzd = ht_nt().until(date!(2026 - 06 - 30));
        assert!(zzd.register_for(datetime!(2026-07-01 10:00 UTC)).is_none());
        assert!(zzd.register_for(datetime!(2026-06-30 10:00 UTC)).is_some());
        assert!(zzd.register_for(datetime!(2025-12-31 10:00 UTC)).is_none());
    }

    #[test]
    fn modul_3_resolves_three_registers_on_all_days() {
        let zzd = modul_3(&ALL_YEAR);
        assert_eq!(zzd.registers(), vec![HT, NT, ST]);
        assert_eq!(zzd.register_for(datetime!(2026-01-05 17:00 UTC)), Some(HT)); // 18:00
        assert_eq!(zzd.register_for(datetime!(2026-01-05 22:00 UTC)), Some(NT)); // 23:00
        assert_eq!(zzd.register_for(datetime!(2026-01-06 2:00 UTC)), Some(NT)); // 03:00
        assert_eq!(zzd.register_for(datetime!(2026-01-05 9:00 UTC)), Some(ST)); // 10:00
        assert_eq!(zzd.register_for(datetime!(2026-01-04 17:00 UTC)), Some(HT)); // Sunday
        assert!(matches!(
            Zaehlzeitdefinition::modul_3("X", date!(2026 - 01 - 01), (0, 60), (60, 120), &[]),
            Err(ZaehlzeitError::NoBilledQuarter)
        ));
    }

    /// A Modul 3 definition restricted to billed quarters (the AWH's
    /// Wahlrecht) conforms, and books no HT energy outside them.
    #[test]
    fn a_definition_restricted_to_billed_quarters_conforms_and_books_st_elsewhere() {
        let zzd = modul_3(&[Quarter::Q1, Quarter::Q4]);
        let a = assess_modul_3(&zzd, &Modul3Context::new().at_a_conforming_delivery_point());
        assert_eq!(a.verdict, Modul3Conformance::Conforms, "{:?}", a.findings);
        assert_eq!(a.billed_quarters, vec![Quarter::Q1, Quarter::Q4]);

        // A July day books everything into ST — the lookup agrees with the
        // assessment, which read the same windows.
        let july = zzd
            .split_energy(&day(date!(2026 - 07 - 06), dec!(1)))
            .unwrap();
        assert!(july.is_complete());
        assert_eq!(july.per_register.keys().copied().collect::<Vec<_>>(), [ST]);
        assert_eq!(july.per_register[ST], dec!(96));

        let january = zzd
            .split_energy(&day(date!(2026 - 01 - 05), dec!(1)))
            .unwrap();
        assert_eq!(january.per_register[HT], dec!(12));
        assert_eq!(january.per_register[NT], dec!(32));
        assert_eq!(january.per_register[ST], dec!(52));
    }

    /// Billed months that do not form whole quarters break the *"auf einzelne
    /// Quartale"* rule.
    #[test]
    fn billed_months_must_form_whole_quarters() {
        let mut zzd = modul_3(&ALL_YEAR);
        for w in &mut zzd.windows {
            w.months_mask = 0b1100_0000_0011; // Jan, Feb, Nov, Dec
        }
        let a = assess_modul_3(&zzd, &Modul3Context::new().at_a_conforming_delivery_point());
        assert_eq!(a.verdict, Modul3Conformance::Violates);
        assert!(
            a.findings
                .contains(&Modul3Finding::BilledMonthsSplitAQuarter)
        );
        assert!(
            a.findings
                .contains(&Modul3Finding::FewerThanTwoBilledQuarters)
        );
        assert!(a.billed_quarters.is_empty());
    }

    #[test]
    fn two_quarters_are_required_but_need_not_be_adjacent() {
        let ctx = Modul3Context::new().at_a_conforming_delivery_point();
        let one = assess_modul_3(&modul_3(&[Quarter::Q2, Quarter::Q2]), &ctx);
        assert_eq!(one.verdict, Modul3Conformance::Violates);
        assert!(
            one.findings
                .contains(&Modul3Finding::FewerThanTwoBilledQuarters)
        );
        assert_eq!(
            assess_modul_3(&modul_3(&[Quarter::Q1, Quarter::Q3]), &ctx).verdict,
            Modul3Conformance::Conforms
        );
    }

    /// *"min. an 2 Stunden pro Tag"* — 90 minutes is short; 120 is enough.
    #[test]
    fn a_hochtarif_under_two_hours_is_a_breach() {
        let ctx = Modul3Context::new().at_a_conforming_delivery_point();
        let short = Zaehlzeitdefinition::modul_3(
            "NB-1",
            date!(2026 - 01 - 01),
            (17 * 60, 18 * 60 + 30),
            (22 * 60, 6 * 60),
            &ALL_YEAR,
        )
        .unwrap();
        assert!(
            assess_modul_3(&short, &ctx)
                .findings
                .contains(&Modul3Finding::HochtarifBelowTwoHours)
        );
        let exact = Zaehlzeitdefinition::modul_3(
            "NB-1",
            date!(2026 - 01 - 01),
            (17 * 60, 19 * 60),
            (22 * 60, 6 * 60),
            &ALL_YEAR,
        )
        .unwrap();
        assert_eq!(
            assess_modul_3(&exact, &ctx).verdict,
            Modul3Conformance::Conforms
        );
    }

    #[test]
    fn a_weekday_only_hochtarif_fails_on_sundays() {
        let mut zzd = modul_3(&ALL_YEAR);
        for w in &mut zzd.windows {
            if w.register_id == HT {
                w.days = DayGroup::Weekdays;
            }
        }
        let a = assess_modul_3(&zzd, &Modul3Context::new().at_a_conforming_delivery_point());
        assert!(a.findings.contains(&Modul3Finding::HochtarifBelowTwoHours));
    }

    /// Restricting only NT to part of the billed months swaps NT and ST there,
    /// HT unchanged: the profiles still differ across billed months.
    #[test]
    fn an_nt_st_swap_within_the_billed_months_varies_across_the_year() {
        let mut zzd = modul_3(&ALL_YEAR);
        for w in &mut zzd.windows {
            if w.register_id == NT {
                w.months_mask = 0b1100_0000_0011;
            }
        }
        let a = assess_modul_3(&zzd, &Modul3Context::new().at_a_conforming_delivery_point());
        assert!(
            a.findings
                .contains(&Modul3Finding::WindowsVaryAcrossTheYear)
        );
        assert!(!a.findings.contains(&Modul3Finding::HochtarifBelowTwoHours));
    }

    #[test]
    fn a_missing_fallback_leaves_time_uncovered() {
        let mut zzd = modul_3(&ALL_YEAR);
        zzd.fallback_register = None;
        let a = assess_modul_3(&zzd, &Modul3Context::new().at_a_conforming_delivery_point());
        assert!(a.findings.contains(&Modul3Finding::TimeNotFullyCovered));
        assert!(a.findings.contains(&Modul3Finding::RegistersAreNotHtNtSt));
    }

    #[test]
    fn missing_delivery_point_data_is_unknown_and_a_breach_outranks_it() {
        let zzd = modul_3(&ALL_YEAR);
        let a = assess_modul_3(&zzd, &Modul3Context::new());
        assert_eq!(a.verdict, Modul3Conformance::Unknown);
        assert_eq!(a.findings, vec![Modul3Finding::DeliveryPointDataMissing]);

        let ctx = Modul3Context::new()
            .modul_1(false)
            .registrierende_leistungsmessung(true)
            .intelligentes_messsystem(false);
        let a = assess_modul_3(&zzd, &ctx);
        assert_eq!(a.verdict, Modul3Conformance::Violates);
        for f in [
            Modul3Finding::Modul1NotSelected,
            Modul3Finding::RegistrierendeLeistungsmessung,
            Modul3Finding::NoIntelligentesMesssystem,
        ] {
            assert!(a.findings.contains(&f), "{f}");
        }
    }

    #[test]
    fn validity_must_describe_one_calendar_year() {
        let ctx = Modul3Context::new().at_a_conforming_delivery_point();
        let mid_year = Zaehlzeitdefinition::modul_3(
            "NB-1",
            date!(2026 - 04 - 01),
            (17 * 60, 20 * 60),
            (22 * 60, 6 * 60),
            &ALL_YEAR,
        )
        .unwrap();
        assert!(
            assess_modul_3(&mid_year, &ctx)
                .findings
                .contains(&Modul3Finding::ValidityIsNotOneCalendarYear)
        );
        let mut open = modul_3(&ALL_YEAR);
        open.valid_to = None;
        assert_eq!(
            assess_modul_3(&open, &ctx).verdict,
            Modul3Conformance::Conforms
        );
    }

    #[test]
    fn a_two_register_definition_is_not_modul_3() {
        let a = assess_modul_3(
            &ht_nt(),
            &Modul3Context::new().at_a_conforming_delivery_point(),
        );
        assert!(a.findings.contains(&Modul3Finding::RegistersAreNotHtNtSt));
    }

    #[test]
    fn a_definition_can_name_the_netzbetreiber_that_published_it() {
        let nb: crate::ids::BdewCode = "9900987654329".parse().unwrap();
        assert_eq!(modul_3(&ALL_YEAR).published_by(nb).netzbetreiber, Some(nb));
    }

    /// The register sums reconstruct the Arbeitsmenge across both DST days.
    #[test]
    fn the_split_holds_across_both_dst_transitions() {
        let zzd = Zaehlzeitdefinition::modul_3(
            "NB-14A-3",
            date!(2026 - 01 - 01),
            (17 * 60, 20 * 60),
            (0, 6 * 60),
            &ALL_YEAR,
        )
        .unwrap();
        for (d, expected_nt) in [
            (date!(2026 - 03 - 29), 20u32),
            (date!(2026 - 07 - 20), 24),
            (date!(2026 - 10 - 25), 28),
        ] {
            let slots = day(d, dec!(1));
            let split = zzd.split_energy(&slots).unwrap();
            assert!(split.is_complete());
            assert_eq!(
                split.per_register.values().copied().sum::<Decimal>(),
                Decimal::from(slots.len()),
                "{d}"
            );
            assert_eq!(split.per_register[NT], Decimal::from(expected_nt), "{d}");
        }
    }

    /// An interval across a band boundary is reported, not booked whole into
    /// the band it starts in.
    #[test]
    fn a_straddling_interval_is_reported_not_attributed() {
        let zzd = ht_nt();
        // 05:30–06:30 local on a Monday crosses the 06:00 HT start.
        let hour = MeterInterval::hour(
            datetime!(2026-01-05 4:30 UTC),
            dec!(4),
            QualityFlag::Measured,
        )
        .unwrap();
        // 06:00–07:00 does not.
        let inside = MeterInterval::hour(
            datetime!(2026-01-05 5:00 UTC),
            dec!(5),
            QualityFlag::Measured,
        )
        .unwrap();
        let split = zzd.split_energy(&[hour.clone(), inside]).unwrap();
        assert_eq!(split.straddling, vec![hour]);
        assert_eq!(split.per_register[HT], dec!(5));
        assert!(!split.per_register.contains_key(NT));
        assert!(!split.is_complete());
    }

    #[test]
    fn unassigned_and_non_billable_intervals_are_reported_rather_than_lost() {
        let zzd =
            Zaehlzeitdefinition::ht_nt("NB-1", date!(2026 - 02 - 01), 6 * 60, 22 * 60).unwrap();
        let before = iv(datetime!(2026-01-15 8:00 UTC), dec!(3));
        let faulty = iv(datetime!(2026-02-16 9:00 UTC), dec!(99)).with_quality(QualityFlag::Faulty);
        let split = zzd
            .split_energy(&[
                before.clone(),
                iv(datetime!(2026-02-16 8:00 UTC), dec!(4)),
                faulty.clone(),
            ])
            .unwrap();
        assert_eq!(split.unassigned, vec![before]);
        assert_eq!(split.non_billable, vec![faulty]);
        assert_eq!(split.per_register[HT], dec!(4));
    }

    #[test]
    fn day_groups_cover_the_week_as_named() {
        use Weekday::{Monday, Saturday, Sunday};
        assert!(DayGroup::Weekdays.contains(Monday));
        assert!(!DayGroup::Weekdays.contains(Saturday));
        assert!(DayGroup::WeekdaysAndSaturday.contains(Saturday));
        assert!(!DayGroup::WeekdaysAndSaturday.contains(Sunday));
        assert!(DayGroup::AllDays.contains(Sunday));
    }

    /// A wrapping band written as one window is refused at construction and
    /// over serde.
    #[cfg(feature = "serde")]
    #[test]
    fn a_wrapping_window_is_refused_on_the_wire() {
        let json = r#"{"register_id":"NT","months_mask":4095,"days":"ALL_DAYS","from_minute":1320,"to_minute":360}"#;
        assert!(serde_json::from_str::<ZaehlzeitFenster>(json).is_err());
        let ok = ZaehlzeitFenster::new(NT, 0, 360).unwrap();
        let back: ZaehlzeitFenster =
            serde_json::from_str(&serde_json::to_string(&ok).unwrap()).unwrap();
        assert_eq!(back, ok);
    }
}
