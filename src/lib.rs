//! German energy-metering quantities: exact, cited, and calendar-correct.
//!
//! A pure library — no I/O, no async, no clock — for the quantities German
//! metering is settled on. Every quantity is a
//! [`Decimal`](rust_decimal::Decimal), every rounding names its places and
//! mode ([`precision`]), and every rule cites the clause it implements. It
//! computes kWh, m³ and kW, not money.
//!
//! Time is Europe/Berlin. A [`DayBoundary`] — `Strom` at 00:00, `Gas` at
//! 06:00 — names days, months and years as UTC [`Period`]s of their real
//! length (23, 24 or 25 hours a day). Interval data enters as
//! [`MeterInterval`]s, validated once into a [`Series`] — sorted, on one grid,
//! one channel — which the rest of the crate consumes.
//!
//! # Quick start
//!
//! **Aggregate a series** into the quantities a billing period is invoiced on:
//!
//! ```rust
//! use metering::prelude::*;
//! use time::{Duration, macros::date};
//!
//! let day = DayBoundary::Strom.day(date!(2026 - 06 - 01)).unwrap();
//! let slots = (0..96)
//!     .map(|i| MeterInterval::quarter_hour(day.start() + Duration::minutes(15 * i), dec!(1), QualityFlag::Measured))
//!     .collect::<Result<Vec<_>, _>>()?;
//! let series = Series::new(Resolution::QUARTER_HOUR, DayBoundary::Strom, slots)?;
//!
//! let period = aggregate(&series, day)?;
//! assert_eq!(period.arbeitsmenge, dec!(96));            // kWh
//! assert_eq!(period.spitzenleistung_kw, Some(dec!(4))); // 1 kWh in 15 min
//! assert_eq!(period.coverage.pct(), Some(dec!(100)));
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
//!
//! **Validate, then substitute** — a missing quarter-hour is an error finding,
//! and a gap up to 2 h is interpolated between validated measured values:
//!
//! ```rust
//! use metering::prelude::*;
//! use metering::time::holiday::Bundesland;
//! use metering::vee::substitute::{Method, SubstitutionReason};
//! use metering::vee::validation::Grade;
//! use time::{Duration, macros::{date, datetime}};
//!
//! let day = DayBoundary::Strom.day(date!(2026 - 06 - 01)).unwrap();
//! let kwh = |i: i64| Decimal::new(200 + i, 3); // a ramp, 0.200 … 0.295 kWh
//! let slots = (0..96)
//!     .filter(|i| *i != 50) // one quarter-hour never arrived
//!     .map(|i| MeterInterval::quarter_hour(day.start() + Duration::minutes(15 * i), kwh(i), QualityFlag::Measured))
//!     .collect::<Result<Vec<_>, _>>()?;
//! let series = Series::new(Resolution::QUARTER_HOUR, DayBoundary::Strom, slots)?;
//!
//! let rules = Rules::strom(day, datetime!(2026-07-01 0:00 UTC), None);
//! let report = validate(&series, &rules);
//! assert_eq!(report.grade(), Grade::C);
//!
//! let policy = Policy::strom(Bundesland::Be, SubstitutionReason::CommunicationFailure);
//! let filled = substitute(&series, &report, &policy)?;
//! assert_eq!(filled.substitutes[0].method, Method::Interpolation);
//! assert_eq!(filled.substitutes[0].value, dec!(0.25)); // between 0.249 and 0.251
//! assert_eq!(validate(&filled.series, &rules).grade(), Grade::A);
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
//!
//! **The Gastag and gas conversion** — a gas day runs 06:00 to 06:00 local,
//! and a metered volume becomes kWh_Hs through Brennwert and Zustandszahl:
//!
//! ```rust
//! use metering::gas::conversion::gas_m3_to_kwh_hs;
//! use metering::prelude::*;
//! use time::macros::{date, datetime};
//!
//! let gastag = DayBoundary::Gas.day(date!(2026 - 01 - 15)).unwrap();
//! assert_eq!(gastag.start(), datetime!(2026-01-15 5:00 UTC));
//! // The Gastag across the autumn change is 25 hours long.
//! assert_eq!(DayBoundary::Gas.day(date!(2026 - 10 - 24)).unwrap().duration().whole_hours(), 25);
//!
//! // 100 m³ × 11.2 kWh/m³ × 0.95
//! assert_eq!(gas_m3_to_kwh_hs(dec!(100), dec!(11.2), dec!(0.95)), Some(dec!(1064.000)));
//! ```
//!
//! # Features
//!
//! `serde` adds `Serialize`/`Deserialize` to the coded enums, identifiers,
//! inputs and most results, and enables `time/serde`; the covered types and
//! wire representation are under
//! [Design](https://hupe1980.github.io/metering/docs/design/#serde).
//!
//! # Modules
//!
//! | Group | Modules |
//! |---|---|
//! | Series, time, IDs | [`series`] (intervals, [`Series`], readings, resampling), [`time`] (calendar, holidays, resolution), [`ids`] (MaLo, MeLo, EIC, BDEW codes, OBIS) |
//! | VEE | [`vee`] — validation and grading, substitute values, Messtyp |
//! | Billing | [`billing`] — aggregation, Zählzeiten, reactive energy, imbalance, losses, forecast |
//! | SLP | [`slp`] — `strom` (BDEW SLP), `gas` (SigLinDe) |
//! | Gas, heat | [`gas`] (DVGW G 685 conversion), [`heat`] (HeizkostenV) |
//! | Grid | [`grid`] — § 14a EnWG, Ausfallarbeit, EN 50160, rollout |
//! | Allocation | [`allocation`] — UTILTS Berechnungsformel, § 42b community, § 42c sharing, sessions |
//! | EEG | [`eeg`] — MiSpeL, EEG § 51/§ 51a, EnFG § 46 |
//! | Cross-cutting | [`precision`] (every rounding), [`error`], [`prelude`] |
//!
//! Start with the [guides](https://hupe1980.github.io/metering); the
//! [design constraints](https://hupe1980.github.io/metering/docs/design/) and
//! [regulatory basis](https://hupe1980.github.io/metering/docs/regulatory-basis/)
//! are the reference for rules and sources.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod wire;

pub mod allocation;
pub mod billing;
pub mod eeg;
pub mod error;
pub mod gas;
pub mod grid;
pub mod heat;
pub mod ids;
pub mod precision;
pub mod series;
pub mod slp;
pub mod time;
pub mod vee;

pub use error::ParseError;
pub use ids::obis::ObisCode;
pub use ids::{MaloId, MeloId};
pub use series::Series;
pub use series::interval::{Direction, MeterInterval, QualityFlag, Sparte, Unit};
pub use time::calendar::{DayBoundary, Period};
pub use time::resolution::Resolution;

/// The common pipeline in one import: the core nouns, the entry points from
/// readings to a billing period, and `Decimal` with its `dec!` literal.
pub mod prelude {
    pub use crate::billing::aggregation::{BillingPeriod, aggregate};
    pub use crate::series::reading::{LastgangConfig, MeterReading, to_lastgang};
    pub use crate::series::resample::resample;
    pub use crate::vee::substitute::{Filled, Policy, substitute};
    pub use crate::vee::validation::{Report, Rules, validate};
    pub use crate::{
        DayBoundary, Direction, MaloId, MeloId, MeterInterval, ObisCode, ParseError, Period,
        QualityFlag, Resolution, Series, Sparte, Unit,
    };
    pub use rust_decimal::{Decimal, dec};
}

/// The README and the guide pages, compiled as doctests: every `rust` block
/// published outside this crate's source builds and runs against the current
/// API. A block that is not meant to compile is fenced `text`.
#[cfg(doctest)]
mod published_prose {
    #[doc = include_str!("../README.md")]
    struct Readme;
    #[doc = include_str!("../site/content/_index.md")]
    struct SiteIndex;
    #[doc = include_str!("../site/content/docs/getting-started.md")]
    struct GettingStarted;
    #[doc = include_str!("../site/content/docs/time-and-calendar.md")]
    struct TimeAndCalendar;
    #[doc = include_str!("../site/content/docs/series-and-readings.md")]
    struct SeriesAndReadings;
    #[doc = include_str!("../site/content/docs/identifiers.md")]
    struct Identifiers;
    #[doc = include_str!("../site/content/docs/validation-and-substitution.md")]
    struct ValidationAndSubstitution;
    #[doc = include_str!("../site/content/docs/billing-quantities.md")]
    struct BillingQuantities;
    #[doc = include_str!("../site/content/docs/gas-and-units.md")]
    struct GasAndUnits;
    #[doc = include_str!("../site/content/docs/heat.md")]
    struct Heat;
    #[doc = include_str!("../site/content/docs/grid.md")]
    struct Grid;
    #[doc = include_str!("../site/content/docs/power-quality.md")]
    struct PowerQuality;
    #[doc = include_str!("../site/content/docs/allocation.md")]
    struct Allocation;
    #[doc = include_str!("../site/content/docs/eeg.md")]
    struct Eeg;
    #[doc = include_str!("../site/content/docs/design.md")]
    struct Design;
}
