//! `IntervalResolution` — typed interval length enum replacing raw `u32` seconds.
//!
//! ## Why typed?
//!
//! Raw `u32` seconds (e.g. `900`) are error-prone and opaque. `IntervalResolution`
//! makes the intended granularity explicit and prevents confusion between hourly
//! and daily data in billing aggregation.
//!
//! ## Fixed vs calendar resolutions
//!
//! | Resolution | Length | Kind |
//! |---|---|---|
//! | `QuarterHour` | 900 s | fixed |
//! | `HalfHour` | 1800 s | fixed |
//! | `Hour` | 3600 s | fixed |
//! | `Custom(n)` | n s (never 0, 900, 1800 or 3600) | fixed |
//! | `Day` | 23 h, 24 h **or 25 h** | calendar |
//! | `Month` | 28–31 days ±1 h | calendar |
//! | `Year` | 365 or 366 days | calendar |
//!
//! [`IntervalResolution::fixed_seconds`] returns `Some` only for the fixed
//! group. The calendar group has no second count that is right on every date —
//! a German day is 23 hours long each spring and 25 each autumn — so those
//! lengths must be resolved against an actual date via [`crate::calendar`]:
//!
//! ```rust
//! use metering::{IntervalResolution, calendar};
//! use time::macros::date;
//!
//! assert_eq!(IntervalResolution::QuarterHour.fixed_seconds(), Some(900));
//! assert_eq!(IntervalResolution::Day.fixed_seconds(), None);
//!
//! // A day's real length comes from the calendar, not the enum.
//! assert_eq!(calendar::day_length(date!(2026 - 10 - 25)).whole_hours(), 25);
//! ```

use std::fmt;
use std::str::FromStr;

use crate::error::ParseError;

/// Typed interval resolution for meter data time series.
///
/// The canonical string form is an ISO 8601 duration (`PT15M`, `P1D`, …), which
/// [`Display`](fmt::Display) writes, [`FromStr`] reads, and — with the `serde`
/// feature — `Serialize`/`Deserialize` use as well, so there is exactly one
/// spelling per value. [`label`] gives the German name used on invoices and in
/// operator UIs.
///
/// [`label`]: IntervalResolution::label
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IntervalResolution {
    /// 15-minute intervals (900 s) — standard for RLM, iMSys, SMGW.
    QuarterHour,
    /// 30-minute intervals (1800 s) — some legacy MSB systems.
    HalfHour,
    /// Hourly intervals (3600 s) — Gas, some SLP reconstructed profiles.
    Hour,
    /// One Berlin calendar day — 23 h, 24 h or 25 h depending on DST.
    Day,
    /// One Berlin calendar month — billing period granularity.
    Month,
    /// One Berlin calendar year — 365 or 366 days.
    Year,
    /// A non-standard fixed length, in seconds.
    ///
    /// The payload is opaque on purpose — see [`CustomSeconds`]. Build one with
    /// [`from_seconds`](Self::from_seconds), which is also what [`FromStr`]
    /// uses, so a duration that already has a name comes back under that name.
    Custom(CustomSeconds),
}

/// The second count of an [`IntervalResolution::Custom`] interval.
///
/// Opaque so that a `Custom` can never alias a named variant. `Custom(900)`
/// and [`QuarterHour`](IntervalResolution::QuarterHour) would otherwise be two
/// distinct values meaning one thing — two database keys, two map entries, and
/// a broken round trip, because `Custom(900)` writes `"PT900S"` and `"PT900S"`
/// parses back to `QuarterHour`. [`new`](Self::new) refuses every count that
/// already has a name, and zero along with them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CustomSeconds(u32);

impl CustomSeconds {
    /// A custom length of `secs` seconds.
    ///
    /// `None` for `0` — a zero-length interval is not a resolution — and for
    /// `900`, `1800` and `3600`, which are
    /// [`QuarterHour`](IntervalResolution::QuarterHour),
    /// [`HalfHour`](IntervalResolution::HalfHour) and
    /// [`Hour`](IntervalResolution::Hour).
    #[must_use]
    pub const fn new(secs: u32) -> Option<Self> {
        match secs {
            0 | 900 | 1800 | 3600 => None,
            n => Some(Self(n)),
        }
    }

    /// The second count. Always positive, and never one of the named lengths.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

impl std::fmt::Display for CustomSeconds {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<CustomSeconds> for u32 {
    fn from(c: CustomSeconds) -> u32 {
        c.0
    }
}

impl TryFrom<u32> for CustomSeconds {
    type Error = ParseError;

    fn try_from(secs: u32) -> Result<Self, Self::Error> {
        Self::new(secs).ok_or_else(|| {
            ParseError::format(
                "CustomSeconds",
                &secs.to_string(),
                "a positive second count that is not 900, 1800 or 3600",
            )
        })
    }
}

impl IntervalResolution {
    /// The interval length in seconds, when it is the same on every date.
    ///
    /// `None` for [`Day`](Self::Day), [`Month`](Self::Month) and
    /// [`Year`](Self::Year), whose lengths depend on the calendar and on DST.
    /// Use [`crate::calendar::day_length`], [`crate::calendar::month_length`] or
    /// [`crate::calendar::year_length`] for those.
    ///
    /// Returning `None` rather than an approximation is deliberate: a caller
    /// computing "how many 15-minute intervals should this day hold" from a flat
    /// 86 400 gets 96 on every day of the year, which raises a false alarm on the
    /// 23-hour spring day and — worse — hides a genuine four-interval gap on the
    /// 25-hour autumn one.
    ///
    /// Every other variant answers `Some`, and never `Some(0)`: a
    /// [`CustomSeconds`] cannot hold zero, so no caller is handed a division by
    /// zero.
    #[must_use]
    pub const fn fixed_seconds(self) -> Option<u32> {
        match self {
            Self::QuarterHour => Some(900),
            Self::HalfHour => Some(1800),
            Self::Hour => Some(3600),
            Self::Custom(s) => Some(s.get()),
            Self::Day | Self::Month | Self::Year => None,
        }
    }

    /// A nominal length in seconds, for sizing and ordering only.
    ///
    /// `Day` reports 24 h, `Month` 30 days and `Year` 365 days — none of which
    /// is reliably true. **Never use this for interval counts, coverage checks or
    /// billing arithmetic**; it exists to pre-allocate buffers, order resolutions
    /// by coarseness and render rough labels. [`fixed_seconds`] is the arithmetic
    /// accessor and refuses to guess.
    ///
    /// [`fixed_seconds`]: Self::fixed_seconds
    #[must_use]
    pub const fn nominal_seconds(self) -> u32 {
        match self {
            Self::Day => 86_400,
            Self::Month => 30 * 86_400,
            Self::Year => 365 * 86_400,
            other => match other.fixed_seconds() {
                Some(s) => s,
                None => 0,
            },
        }
    }

    /// `true` when the length is the same on every date — everything but
    /// `Day`, `Month` and `Year`.
    #[must_use]
    pub const fn is_fixed(self) -> bool {
        self.fixed_seconds().is_some()
    }

    /// `true` when the length depends on the calendar: `Day`, `Month`, `Year`.
    #[must_use]
    pub const fn is_calendar(self) -> bool {
        matches!(self, Self::Day | Self::Month | Self::Year)
    }

    /// Human-readable label (German).
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::QuarterHour => "15-Minuten",
            Self::HalfHour => "30-Minuten",
            Self::Hour => "Stunde",
            Self::Day => "Tag",
            Self::Month => "Monat",
            Self::Year => "Jahr",
            Self::Custom(_) => "Benutzerdefiniert",
        }
    }

    /// The canonical ISO 8601 duration form, as written by [`Display`](fmt::Display)
    /// and read by [`FromStr`].
    ///
    /// `Custom(n)` renders as `PT{n}S`, so every value round-trips.
    #[must_use]
    pub fn to_iso8601(self) -> String {
        match self {
            Self::QuarterHour => "PT15M".to_owned(),
            Self::HalfHour => "PT30M".to_owned(),
            Self::Hour => "PT1H".to_owned(),
            Self::Day => "P1D".to_owned(),
            Self::Month => "P1M".to_owned(),
            Self::Year => "P1Y".to_owned(),
            Self::Custom(s) => format!("PT{}S", s.get()),
        }
    }

    /// Create from raw seconds. Returns `None` for zero.
    ///
    /// Never returns [`Day`](Self::Day), [`Month`](Self::Month) or
    /// [`Year`](Self::Year): those are calendar periods, and 86 400 s is only
    /// *usually* a day. `from_seconds(86_400)` is therefore `Custom(86_400)` —
    /// a fixed 24-hour window, which is a different thing from a German
    /// calendar day and is treated as such by [`crate::calendar`].
    #[must_use]
    pub const fn from_seconds(s: u32) -> Option<Self> {
        match s {
            900 => Some(Self::QuarterHour),
            1800 => Some(Self::HalfHour),
            3600 => Some(Self::Hour),
            other => match CustomSeconds::new(other) {
                Some(c) => Some(Self::Custom(c)),
                // Only zero is left, and a zero-length interval is not a
                // resolution.
                None => None,
            },
        }
    }

    /// The resolution a **measured** spacing of `secs` seconds indicates.
    ///
    /// [`from_seconds`](Self::from_seconds) is exact; this is the tolerant
    /// counterpart for a number read off real data, where a transmission
    /// timestamp wanders by a few seconds and a daily series is 23, 24 or 25
    /// hours long depending on the date.
    ///
    /// | Observed | Resolution |
    /// |---|---|
    /// | 750…1 050 s | [`QuarterHour`](Self::QuarterHour) |
    /// | 1 650…1 950 s | [`HalfHour`](Self::HalfHour) |
    /// | 3 300…3 900 s | [`Hour`](Self::Hour) |
    /// | 82 800…90 000 s | [`Day`](Self::Day) — the Berlin calendar day |
    /// | anything else | [`from_seconds`](Self::from_seconds) exactly |
    ///
    /// The daily band is why this exists and why it answers `Day` rather than
    /// `Custom(86_400)`: a series read once a day legitimately contains all
    /// three lengths, and a fixed 24-hour window is a different thing from a
    /// German calendar day.
    ///
    /// `None` for a non-positive spacing, and for one too large to be a
    /// resolution at all.
    ///
    /// ```rust
    /// use metering::IntervalResolution;
    ///
    /// assert_eq!(IntervalResolution::from_observed_seconds(898), Some(IntervalResolution::QuarterHour));
    /// // A 25-hour autumn day is still a day.
    /// assert_eq!(IntervalResolution::from_observed_seconds(90_000), Some(IntervalResolution::Day));
    /// // ...while a fixed 24-hour window read exactly is not.
    /// assert_eq!(IntervalResolution::from_observed_seconds(300), IntervalResolution::from_seconds(300));
    /// assert_eq!(IntervalResolution::from_observed_seconds(0), None);
    /// ```
    #[must_use]
    pub const fn from_observed_seconds(secs: i64) -> Option<Self> {
        match secs {
            750..=1_050 => Some(Self::QuarterHour),
            1_650..=1_950 => Some(Self::HalfHour),
            3_300..=3_900 => Some(Self::Hour),
            // 23 h … 25 h — a Berlin calendar day at either DST transition.
            82_800..=90_000 => Some(Self::Day),
            other => {
                // `u32::try_from` is not const; the bounds check is the same.
                if other <= 0 || other > u32::MAX as i64 {
                    None
                } else {
                    Self::from_seconds(other as u32)
                }
            }
        }
    }

    /// `true` when this resolution supports real-time or near-real-time data.
    ///
    /// Quarter-hour and half-hour are the relevant resolutions for iMSys / SMGW.
    #[must_use]
    pub const fn is_subhourly(self) -> bool {
        matches!(self, Self::QuarterHour | Self::HalfHour)
    }
}

impl fmt::Display for IntervalResolution {
    /// Writes the ISO 8601 duration form, which [`FromStr`] reads back.
    ///
    /// Width, fill and alignment are honoured, as they are for
    /// [`ObisCode`](crate::ObisCode) and every coded enum here, so `{:>6}`
    /// lines a resolution up in a table.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.pad(&self.to_iso8601())
    }
}

/// The shape [`IntervalResolution`] accepts, as rendered in a [`ParseError`].
const ISO8601_FORMAT: &str = "an ISO 8601 duration such as PT15M, PT1H, P1D or PT900S";

impl FromStr for IntervalResolution {
    type Err = ParseError;

    /// Parses the ISO 8601 duration forms this crate emits, case-insensitively.
    ///
    /// Accepts the canonical spellings (`PT15M`, `PT30M`, `PT1H`, `P1D`, `P1M`,
    /// `P1Y`) plus any `PT{n}S`, `PT{n}M` or `PT{n}H`, which normalise onto the
    /// named variants where they coincide — `PT900S` is `QuarterHour`, not
    /// `Custom(900)`, so equal durations compare equal.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let err = || ParseError::format("IntervalResolution", s, ISO8601_FORMAT);
        let upper = s.trim().to_uppercase();
        let seconds = match upper.as_str() {
            "P1D" => return Ok(Self::Day),
            "P1M" => return Ok(Self::Month),
            "P1Y" => return Ok(Self::Year),
            rest => {
                let body = rest.strip_prefix("PT").ok_or_else(err)?;
                let (digits, unit) = body.split_at(body.len().checked_sub(1).ok_or_else(err)?);
                let n: u32 = digits.parse().map_err(|_| err())?;
                match unit {
                    "S" => n,
                    "M" => n.checked_mul(60).ok_or_else(err)?,
                    "H" => n.checked_mul(3600).ok_or_else(err)?,
                    _ => return Err(err()),
                }
            }
        };
        Self::from_seconds(seconds).ok_or_else(err)
    }
}

// ── Serde: the ISO 8601 string, not the Rust variant name ────────────────────

#[cfg(feature = "serde")]
mod serde_impl {
    use super::IntervalResolution;
    use serde::de::{self, Visitor};
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use std::fmt;

    impl Serialize for IntervalResolution {
        /// Writes the ISO 8601 duration — the same string
        /// [`Display`](fmt::Display) writes.
        ///
        /// The derived representation used the Rust variant names
        /// (`"QuarterHour"`, `{"Custom":300}`), giving each value a second
        /// spelling that a rename could silently invalidate. ISO 8601 is an
        /// external standard, so no refactor here can change it.
        fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
            serializer.collect_str(self)
        }
    }

    struct IntervalResolutionVisitor;

    impl Visitor<'_> for IntervalResolutionVisitor {
        type Value = IntervalResolution;

        fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("an ISO 8601 duration string such as \"PT15M\" or \"P1D\"")
        }

        fn visit_str<E: de::Error>(self, v: &str) -> Result<IntervalResolution, E> {
            v.parse().map_err(de::Error::custom)
        }
    }

    impl<'de> Deserialize<'de> for IntervalResolution {
        fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
            deserializer.deserialize_str(IntervalResolutionVisitor)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quarter_hour_is_900s() {
        assert_eq!(IntervalResolution::QuarterHour.fixed_seconds(), Some(900));
    }

    /// The bug this API shape exists to prevent: no fixed second count is right
    /// for a calendar period, so none is offered.
    #[test]
    fn calendar_resolutions_have_no_fixed_length() {
        for r in [
            IntervalResolution::Day,
            IntervalResolution::Month,
            IntervalResolution::Year,
        ] {
            assert_eq!(r.fixed_seconds(), None, "{r} must not claim a fixed length");
            assert!(r.is_calendar() && !r.is_fixed());
        }
        for r in [
            IntervalResolution::QuarterHour,
            IntervalResolution::HalfHour,
            IntervalResolution::Hour,
            IntervalResolution::from_seconds(7200).unwrap(),
        ] {
            assert!(r.is_fixed() && !r.is_calendar(), "{r} must be fixed");
        }
    }

    #[test]
    fn from_seconds_round_trip() {
        for r in [
            IntervalResolution::QuarterHour,
            IntervalResolution::HalfHour,
            IntervalResolution::Hour,
            IntervalResolution::from_seconds(7200).unwrap(),
        ] {
            let s = r.fixed_seconds().unwrap();
            assert_eq!(IntervalResolution::from_seconds(s), Some(r));
        }
    }

    /// 86 400 s is a fixed 24-hour window, which is not the same thing as a
    /// German calendar day.
    #[test]
    fn a_day_is_not_86400_seconds() {
        let fixed = IntervalResolution::from_seconds(86_400).unwrap();
        assert_eq!(fixed.fixed_seconds(), Some(86_400));
        assert_ne!(fixed, IntervalResolution::Day);
        assert!(fixed.is_fixed() && !fixed.is_calendar());
    }

    /// A named length is never reachable as a `Custom`, so one duration has one
    /// value: `Custom(900)` and `QuarterHour` would otherwise be two distinct
    /// keys for one 15-minute grid, and `Custom(900).to_string()` would parse
    /// back as the other one.
    #[test]
    fn a_named_length_is_never_a_custom() {
        for named in [900u32, 1800, 3600] {
            assert_eq!(CustomSeconds::new(named), None, "{named} s has a name");
            assert!(CustomSeconds::try_from(named).is_err());
        }
        assert_eq!(CustomSeconds::new(0), None, "zero is not a resolution");
        assert_eq!(
            IntervalResolution::from_seconds(900),
            Some(IntervalResolution::QuarterHour)
        );
        assert_eq!(CustomSeconds::new(7200).map(CustomSeconds::get), Some(7200));
        assert_eq!(u32::from(CustomSeconds::new(7200).unwrap()), 7200);
        assert_eq!(CustomSeconds::new(7200).unwrap().to_string(), "7200");
    }

    #[test]
    fn zero_returns_none() {
        assert!(IntervalResolution::from_seconds(0).is_none());
    }

    #[test]
    fn custom_seconds_preserved() {
        let r = IntervalResolution::from_seconds(7200).unwrap();
        assert_eq!(r.fixed_seconds(), Some(7200));
        assert!(!r.is_subhourly());
    }

    #[test]
    fn subhourly_detection() {
        assert!(IntervalResolution::QuarterHour.is_subhourly());
        assert!(IntervalResolution::HalfHour.is_subhourly());
        assert!(!IntervalResolution::Hour.is_subhourly());
        assert!(!IntervalResolution::Day.is_subhourly());
    }

    /// `Display` and `FromStr` are inverses over every variant, so a persisted
    /// or logged value always reads back as what was written.
    #[test]
    fn iso8601_round_trips_every_variant() {
        for r in [
            IntervalResolution::QuarterHour,
            IntervalResolution::HalfHour,
            IntervalResolution::Hour,
            IntervalResolution::Day,
            IntervalResolution::Month,
            IntervalResolution::Year,
            IntervalResolution::from_seconds(300).unwrap(),
            IntervalResolution::from_seconds(86_400).unwrap(),
        ] {
            let s = r.to_string();
            assert_eq!(s.parse::<IntervalResolution>(), Ok(r), "round trip {s}");
        }
        assert_eq!(IntervalResolution::QuarterHour.to_string(), "PT15M");
        assert_eq!(IntervalResolution::Day.to_string(), "P1D");
        // Width and alignment are honoured, so a channel listing lines up.
        assert_eq!(format!("{:>6}", IntervalResolution::Day), "   P1D");
        assert_eq!(
            format!("{:<6}|", IntervalResolution::QuarterHour),
            "PT15M |"
        );
        assert_eq!(
            IntervalResolution::from_seconds(300).unwrap().to_string(),
            "PT300S"
        );
    }

    /// Equal durations parse to the same variant, whichever spelling arrives.
    #[test]
    fn equivalent_spellings_normalise() {
        assert_eq!(
            "PT900S".parse::<IntervalResolution>(),
            Ok(IntervalResolution::QuarterHour)
        );
        assert_eq!(
            "pt15m".parse::<IntervalResolution>(),
            Ok(IntervalResolution::QuarterHour)
        );
        assert_eq!(
            "PT60M".parse::<IntervalResolution>(),
            Ok(IntervalResolution::Hour)
        );
        assert_eq!(
            "PT2H".parse::<IntervalResolution>(),
            Ok(IntervalResolution::from_seconds(7200).unwrap())
        );
    }

    #[test]
    fn invalid_resolution_strings_are_rejected() {
        for s in [
            "", "P", "PT", "15M", "PT15X", "PTM", "P1W", "PT-5S", "hourly",
        ] {
            assert!(
                s.parse::<IntervalResolution>().is_err(),
                "{s:?} must not parse"
            );
        }
    }

    /// The nominal length is explicitly an approximation; the test pins that it
    /// is never mistaken for the arithmetic accessor.
    #[test]
    fn nominal_length_is_only_a_hint() {
        assert_eq!(IntervalResolution::Day.nominal_seconds(), 86_400);
        assert_eq!(IntervalResolution::Month.nominal_seconds(), 30 * 86_400);
        assert_eq!(IntervalResolution::Year.nominal_seconds(), 365 * 86_400);
        // ...while the arithmetic accessor refuses all three.
        assert!(IntervalResolution::Month.fixed_seconds().is_none());
    }
}
