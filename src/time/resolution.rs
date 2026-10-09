//! [`Resolution`] — the closed set of interval lengths a metered series uses.
//!
//! | Resolution | Length | Kind |
//! |---|---|---|
//! | [`Minutes(m)`](Resolution::Minutes) | `m` minutes, `m` dividing 60 | fixed |
//! | [`Hour`](Resolution::Hour) | 3 600 s | fixed |
//! | [`Day`](Resolution::Day) | 23 h, 24 h **or 25 h** | calendar |
//! | [`Month`](Resolution::Month) | 28–31 days ±1 h | calendar |
//! | [`Year`](Resolution::Year) | 365 or 366 days | calendar |
//!
//! A minute step divides the hour, so every fixed step divides every Berlin
//! day. A calendar length is resolved against a date via
//! [`DayBoundary`](crate::DayBoundary):
//!
//! ```rust
//! use metering::{DayBoundary, Resolution};
//! use time::macros::date;
//!
//! assert_eq!(Resolution::QUARTER_HOUR.fixed_seconds(), Some(900));
//! assert_eq!(Resolution::Day.fixed_seconds(), None);
//!
//! let autumn = DayBoundary::Strom.day(date!(2026 - 10 - 25)).unwrap();
//! assert_eq!(autumn.duration().whole_hours(), 25);
//! assert_eq!(autumn.count(Resolution::QUARTER_HOUR), Some(100));
//! ```

use std::fmt;
use std::str::FromStr;

use crate::error::{ParseError, ParseErrorKind};

/// A sub-hourly step in whole minutes, always a divisor of 60 below 60 (60
/// itself is [`Resolution::Hour`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MinuteStep(u8);

impl MinuteStep {
    /// Every admissible step, ascending.
    pub const ALL: [Self; 11] = [
        Self(1),
        Self(2),
        Self(3),
        Self(4),
        Self(5),
        Self(6),
        Self(10),
        Self(12),
        Self(15),
        Self(20),
        Self(30),
    ];

    /// The step of `minutes`, or `None` unless it divides 60 and is below 60.
    #[must_use]
    pub const fn new(minutes: u8) -> Option<Self> {
        match minutes {
            1 | 2 | 3 | 4 | 5 | 6 | 10 | 12 | 15 | 20 | 30 => Some(Self(minutes)),
            _ => None,
        }
    }

    /// The step in minutes.
    #[must_use]
    pub const fn get(self) -> u8 {
        self.0
    }
}

/// The length of one interval of a metered series.
///
/// The string form is an ISO 8601 duration (`PT15M`, `PT1H`, `P1D`, `P1M`,
/// `P1Y`), written by [`Display`](fmt::Display), read by [`FromStr`] and used
/// on the `serde` wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Resolution {
    /// A fixed step of whole minutes dividing the hour.
    Minutes(MinuteStep),
    /// One hour (3 600 s) — the gas settlement slot.
    Hour,
    /// One Berlin calendar day (or Gastag) — 23 h, 24 h or 25 h.
    Day,
    /// One Berlin calendar month.
    Month,
    /// One Berlin calendar year.
    Year,
}

/// The shape [`Resolution`] accepts, as rendered in a [`ParseError`].
const ISO8601_FORMAT: &str =
    "an ISO 8601 duration: PT{m}M with m dividing 60, PT1H, P1D, P1M or P1Y (e.g. PT15M)";

impl Resolution {
    /// 15 minutes — the German electricity settlement slot (RLM, iMSys).
    pub const QUARTER_HOUR: Self = Self::Minutes(MinuteStep(15));
    /// 30 minutes.
    pub const HALF_HOUR: Self = Self::Minutes(MinuteStep(30));

    /// A minute resolution; `Some(Hour)` for 60, `None` unless `minutes`
    /// divides 60.
    #[must_use]
    pub const fn minutes(minutes: u8) -> Option<Self> {
        if minutes == 60 {
            return Some(Self::Hour);
        }
        match MinuteStep::new(minutes) {
            Some(step) => Some(Self::Minutes(step)),
            None => None,
        }
    }

    /// The interval length in seconds, when it is the same on every date;
    /// `None` for [`Day`](Self::Day), [`Month`](Self::Month) and
    /// [`Year`](Self::Year), whose lengths depend on the calendar and on DST.
    #[must_use]
    pub const fn fixed_seconds(self) -> Option<u32> {
        match self {
            Self::Minutes(step) => Some(step.0 as u32 * 60),
            Self::Hour => Some(3600),
            Self::Day | Self::Month | Self::Year => None,
        }
    }

    /// A nominal length in seconds, for ordering and sizing only — `Day` is
    /// 24 h, `Month` 30 days, `Year` 365 days. Not for counts or billing.
    #[must_use]
    pub const fn nominal_seconds(self) -> u32 {
        match self {
            Self::Minutes(step) => step.0 as u32 * 60,
            Self::Hour => 3600,
            Self::Day => 86_400,
            Self::Month => 30 * 86_400,
            Self::Year => 365 * 86_400,
        }
    }

    /// `true` for the fixed-length group: [`Minutes`](Self::Minutes) and
    /// [`Hour`](Self::Hour).
    #[must_use]
    pub const fn is_fixed(self) -> bool {
        self.fixed_seconds().is_some()
    }

    /// `true` for the calendar group: `Day`, `Month`, `Year`.
    #[must_use]
    pub const fn is_calendar(self) -> bool {
        !self.is_fixed()
    }

    /// `true` for every step shorter than an hour.
    #[must_use]
    pub const fn is_subhourly(self) -> bool {
        matches!(self, Self::Minutes(_))
    }

    /// Human-readable German label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::QUARTER_HOUR => "Viertelstunde",
            Self::Minutes(_) => "Minuten",
            Self::Hour => "Stunde",
            Self::Day => "Tag",
            Self::Month => "Monat",
            Self::Year => "Jahr",
        }
    }

    /// The fixed resolution of exactly `secs` seconds, if one exists; never
    /// `Day`, `Month` or `Year`.
    #[must_use]
    pub const fn from_seconds(secs: u32) -> Option<Self> {
        if secs == 0 || !secs.is_multiple_of(60) || secs > 3600 {
            return None;
        }
        Self::minutes((secs / 60) as u8)
    }

    /// The resolution an observed spacing of `secs` seconds indicates.
    ///
    /// A fixed step matches within ±5 %; a day matches 23–25 h, a month
    /// 28 days − 1 h to 31 days + 1 h, a year 365 days − 1 h to 366 days + 1 h.
    ///
    /// ```rust
    /// use metering::Resolution;
    ///
    /// assert_eq!(Resolution::from_observed_seconds(898), Some(Resolution::QUARTER_HOUR));
    /// assert_eq!(Resolution::from_observed_seconds(90_000), Some(Resolution::Day));
    /// // March: 31 days minus the spring-forward hour.
    /// assert_eq!(Resolution::from_observed_seconds(31 * 86_400 - 3_600), Some(Resolution::Month));
    /// assert_eq!(Resolution::from_observed_seconds(366 * 86_400), Some(Resolution::Year));
    /// assert_eq!(Resolution::from_observed_seconds(7_200), None);
    /// assert_eq!(Resolution::from_observed_seconds(0), None);
    /// ```
    #[must_use]
    pub fn from_observed_seconds(secs: i64) -> Option<Self> {
        const HOUR: i64 = 3_600;
        const DAY: i64 = 86_400;
        match secs {
            s if s <= 0 => None,
            82_800..=90_000 => Some(Self::Day),
            s if (28 * DAY - HOUR..=31 * DAY + HOUR).contains(&s) => Some(Self::Month),
            s if (365 * DAY - HOUR..=366 * DAY + HOUR).contains(&s) => Some(Self::Year),
            s => MinuteStep::ALL
                .into_iter()
                .map(Self::Minutes)
                .chain([Self::Hour])
                .find(|r| {
                    let step = r.fixed_seconds().map_or(0, i64::from);
                    (s - step).abs() * 20 <= step
                }),
        }
    }
}

impl fmt::Display for Resolution {
    /// Writes the ISO 8601 duration form, which [`FromStr`] reads back.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Minutes(step) => f.pad(&format!("PT{}M", step.0)),
            Self::Hour => f.pad("PT1H"),
            Self::Day => f.pad("P1D"),
            Self::Month => f.pad("P1M"),
            Self::Year => f.pad("P1Y"),
        }
    }
}

impl FromStr for Resolution {
    type Err = ParseError;

    /// Parses `PT{n}S`, `PT{n}M`, `PT{n}H`, `P1D`, `P1M` and `P1Y`,
    /// case-insensitively and trimmed; fixed forms are normalised (`PT900S` and
    /// `PT60M` read as `PT15M`/`PT1H`).
    ///
    /// # Errors
    ///
    /// [`Charset`](ParseErrorKind::Charset) for a malformed string,
    /// [`Range`](ParseErrorKind::Range) for a well-formed duration outside
    /// the set.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let err = |kind| ParseError::format(kind, "Resolution", s, ISO8601_FORMAT);
        let t = s.trim();
        if !t.is_ascii() {
            return Err(err(ParseErrorKind::Charset));
        }
        let upper = t.to_ascii_uppercase();
        match upper.as_str() {
            "P1D" => return Ok(Self::Day),
            "P1M" => return Ok(Self::Month),
            "P1Y" => return Ok(Self::Year),
            _ => {}
        }
        let body = upper
            .strip_prefix("PT")
            .ok_or_else(|| err(ParseErrorKind::Charset))?;
        let Some((unit_at, unit)) = body.char_indices().last() else {
            return Err(err(ParseErrorKind::Charset));
        };
        let digits = &body[..unit_at];
        if digits.is_empty() || digits.len() > 9 || !digits.bytes().all(|b| b.is_ascii_digit()) {
            return Err(err(ParseErrorKind::Charset));
        }
        let n: u32 = digits.parse().map_err(|_| err(ParseErrorKind::Charset))?;
        let secs = match unit {
            'S' => n,
            'M' => n
                .checked_mul(60)
                .ok_or_else(|| err(ParseErrorKind::Range))?,
            'H' => n
                .checked_mul(3600)
                .ok_or_else(|| err(ParseErrorKind::Range))?,
            _ => return Err(err(ParseErrorKind::Charset)),
        };
        Self::from_seconds(secs).ok_or_else(|| err(ParseErrorKind::Range))
    }
}

#[cfg(feature = "serde")]
impl serde::Serialize for Resolution {
    /// Writes the `Display` string.
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

#[cfg(feature = "serde")]
impl<'de> serde::Deserialize<'de> for Resolution {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = <std::borrow::Cow<'de, str> as serde::Deserialize>::deserialize(deserializer)?;
        text.parse().map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [Resolution; 15] = [
        Resolution::Minutes(MinuteStep(1)),
        Resolution::Minutes(MinuteStep(2)),
        Resolution::Minutes(MinuteStep(3)),
        Resolution::Minutes(MinuteStep(4)),
        Resolution::Minutes(MinuteStep(5)),
        Resolution::Minutes(MinuteStep(6)),
        Resolution::Minutes(MinuteStep(10)),
        Resolution::Minutes(MinuteStep(12)),
        Resolution::QUARTER_HOUR,
        Resolution::Minutes(MinuteStep(20)),
        Resolution::HALF_HOUR,
        Resolution::Hour,
        Resolution::Day,
        Resolution::Month,
        Resolution::Year,
    ];

    #[test]
    fn the_minute_set_is_exactly_the_divisors_of_sixty() {
        for m in 0..=255u8 {
            let expected = m > 0 && m < 60 && 60 % m == 0;
            assert_eq!(MinuteStep::new(m).is_some(), expected, "{m}");
        }
        assert_eq!(Resolution::minutes(60), Some(Resolution::Hour));
        assert_eq!(Resolution::minutes(7), None);
    }

    #[test]
    fn calendar_resolutions_have_no_fixed_length() {
        for r in [Resolution::Day, Resolution::Month, Resolution::Year] {
            assert_eq!(r.fixed_seconds(), None);
            assert!(r.is_calendar() && !r.is_fixed() && !r.is_subhourly());
        }
    }

    #[test]
    fn subhourly_is_every_minute_step_and_nothing_else() {
        for r in ALL {
            assert_eq!(r.is_subhourly(), matches!(r, Resolution::Minutes(_)), "{r}");
        }
        assert!(!Resolution::Hour.is_subhourly());
    }

    #[test]
    fn iso8601_round_trips_every_variant() {
        for r in ALL {
            let s = r.to_string();
            assert_eq!(s.parse::<Resolution>(), Ok(r), "round trip {s}");
        }
        assert_eq!(Resolution::QUARTER_HOUR.to_string(), "PT15M");
        assert_eq!(format!("{:>6}", Resolution::Day), "   P1D");
    }

    #[test]
    fn equivalent_spellings_normalise() {
        let q = Ok(Resolution::QUARTER_HOUR);
        assert_eq!("PT900S".parse::<Resolution>(), q);
        assert_eq!("pt15m".parse::<Resolution>(), q);
        assert_eq!("PT60M".parse::<Resolution>(), Ok(Resolution::Hour));
    }

    #[test]
    fn invalid_strings_are_rejected_without_panicking() {
        for s in [
            "",
            "P",
            "PT",
            "15M",
            "PT15X",
            "PTM",
            "P1W",
            "PT-5S",
            "PT+15M",
            "hourly",
            "PT2H",
            "PT7M",
            "PT0M",
            "PT1€",
            "PT€",
            "€",
            "PT99999999999M",
            "P1DT1H",
        ] {
            assert!(s.parse::<Resolution>().is_err(), "{s:?} must not parse");
        }
        assert_eq!(
            "PT7M".parse::<Resolution>().unwrap_err().kind(),
            ParseErrorKind::Range
        );
        assert_eq!(
            "PT1€".parse::<Resolution>().unwrap_err().kind(),
            ParseErrorKind::Charset
        );
    }

    #[test]
    fn observed_lengths_map_onto_the_closed_set() {
        assert_eq!(
            Resolution::from_observed_seconds(900),
            Some(Resolution::QUARTER_HOUR)
        );
        assert_eq!(
            Resolution::from_observed_seconds(3_590),
            Some(Resolution::Hour)
        );
        assert_eq!(
            Resolution::from_observed_seconds(82_800),
            Some(Resolution::Day)
        );
        assert_eq!(
            Resolution::from_observed_seconds(28 * 86_400),
            Some(Resolution::Month)
        );
        assert_eq!(
            Resolution::from_observed_seconds(31 * 86_400 + 3_600),
            Some(Resolution::Month)
        );
        assert_eq!(
            Resolution::from_observed_seconds(365 * 86_400),
            Some(Resolution::Year)
        );
        assert_eq!(Resolution::from_observed_seconds(2 * 86_400), None);
        assert_eq!(Resolution::from_observed_seconds(-900), None);
    }
}
