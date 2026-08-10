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
//! Earlier releases modelled the first of those separately, as a `TariffWindow`
//! with hour-granularity bounds and exactly two outcomes. That type is gone. It
//! could not express Modul 3 — which **every Netzbetreiber has been obliged to
//! offer since 1 April 2025**, and which has *three* tariff levels — and it
//! could not express a band starting at half past the hour. Two mechanisms for
//! one question also meant two places to fix when Feiertage turned out to
//! matter, and only one of them got fixed.
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
    pub valid_from: Date,
    /// Last day (inclusive); `None` = open-ended.
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
        }
    }

    /// A § 14a EnWG Modul 3 definition: [`HT`], [`NT`] and [`ST`] for the rest.
    ///
    /// Since **1 April 2025** every Netzbetreiber must offer Modul 3, and its
    /// three levels are why this crate no longer has a two-register type. The
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
        }
    }

    /// Treat `land`'s statutory holidays as non-working days (builder style).
    #[must_use]
    pub fn in_land(mut self, land: Bundesland) -> Self {
        self.holiday_land = Some(land);
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
    #[must_use]
    pub fn split_energy(&self, intervals: &[MeterInterval]) -> BTreeMap<Option<String>, Decimal> {
        let mut sums: BTreeMap<Option<String>, Decimal> = BTreeMap::new();
        for iv in intervals.iter().filter(|iv| iv.quality.is_billable()) {
            let register = self.register_for(iv.from).map(str::to_owned);
            *sums.entry(register).or_insert(Decimal::ZERO) += iv.value;
        }
        sums
    }
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
        assert_eq!(split[&Some(HT.to_owned())], dec!(2.5) * dec!(12));
        assert_eq!(split[&Some(NT.to_owned())], dec!(2.5) * dec!(24));
        assert_eq!(split[&Some(ST.to_owned())], dec!(2.5) * dec!(60));
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
                split[&Some(NT.to_owned())],
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
        assert_eq!(split[&Some(HT.to_owned())], dec!(4));
    }

    #[test]
    fn non_billable_intervals_are_excluded() {
        let zzd = Zaehlzeitdefinition::ht_nt("NB-1", date!(2026 - 01 - 01), 6 * 60, 22 * 60);
        let mut faulty = iv(datetime!(2026-01-05 8:00 UTC), dec!(99));
        faulty.quality = QualityFlag::Faulty;
        let split = zzd.split_energy(&[iv(datetime!(2026-01-05 9:00 UTC), dec!(4)), faulty]);
        assert_eq!(split[&Some(HT.to_owned())], dec!(4));
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
