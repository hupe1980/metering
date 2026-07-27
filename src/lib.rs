//! German energy metering domain library.
//!
//! A **standalone**, **pure** library for meter data calculations required by
//! BDEW MaKo, MsbG, GasGVV, and EnWG. Zero I/O, no async, no clock, exact
//! decimal quantities.
//!
//! It computes **energy and volume**, not money: there is no currency, price or
//! tariff-rate type here. What leaves this crate is kWh, m³ and kW, which a
//! billing layer then prices.
//!
//! # Modules
//!
//! | Module | Contents |
//! |---|---|
//! | [`interval`] | `MeterInterval`, `Sparte`, `QualityFlag`, `demand_kw()` |
//! | [`calendar`] | Europe/Berlin calendar days, months and years — DST-correct interval counts |
//! | [`error`] | `ParseError` — the single error type every `FromStr` here returns |
//! | [`conversion`] | Gas m³ → kWh_Hs (§25 Nr. 4 MessEV / DVGW G 685) |
//! | [`aggregation`] | Billing period: `arbeitsmenge_kwh`, `spitzenleistung_kw`, HT/NT |
//! | [`classification`] | SLP/RLM/iMSys detection, interval length |
//! | [`imbalance`] | Mehr-/Mindermengensaldo (§ 13 StromNZV, compute_imbalance) |
//! | [`quality`] | Hampel-filter quality scoring (M7), `score_intervals_raw` for f64 |
//! | [`validation`] | V01–V11 validation engine, order-independent |
//! | [`substitute`] | § 60 Abs. 2 MsbG Ersatzwertbildung (4 methods, slot matching) |
//! | [`forecast`] | § 60 Abs. 2 MsbG Jahresprognose with 95% confidence bounds |
//! | [`load_profile`] | SLP classes incl. BDEW 2025 (H25/G25/L25/P25/S25) + Dynamisierung |
//! | [`zaehlzeit`] | Zählzeitdefinition — time-variable register resolution (§14a EnWG) |
//! | [`rollout`] | §29 MsbG Pflichteinbaufälle + §45 MsbG Rollout-Fahrplan |
//! | [`smgw`] | SMGW sessions, certificates, CLS channels (BSI TR-03109) |
//! | [`virtual_meter`] | Sum/Residual/GGV virtual meters (§42b EnWG) |
//!
//! # Numeric types
//!
//! Every **metered quantity** is [`rust_decimal::Decimal`] — `value_kwh`,
//! `arbeitsmenge_kwh`, `spitzenleistung_kw`, the HT/NT split, the gas m³→kWh_Hs
//! conversion, GGV allocations and substitute values are exact decimal
//! arithmetic end to end, never routed through a float and back.
//!
//! `f64` is used, deliberately, for **statistics and diagnostics**:
//! `coverage_pct`, the Hampel filter's threshold and median absolute deviation,
//! [`ValidationConfig::spike_factor`] and the forecast confidence bounds. Those
//! are comparisons and indicators, not amounts.
//!
//! The two meet in exactly one place, documented at the call site: the V04 spike
//! rule converts a value to `f64` to compare it against the `f64` spike factor.
//! That comparison decides whether to *flag* an interval; it never alters one.
//!
//! # Determinism
//!
//! **No function in this crate reads the system clock, the filesystem, the
//! network or any other ambient state.** Every input is an argument, so equal
//! inputs always produce equal outputs, and any result can be replayed or
//! cached. Where a timestamp is needed — a provenance entry, an SMGW
//! communication-fault assessment — it is a parameter:
//!
//! ```rust
//! # use metering::{MeasurementSeries, MeasurementSource};
//! # use time::macros::datetime;
//! let series = MeasurementSeries::new(
//!     "51238696780",
//!     None,
//!     vec![],
//!     MeasurementSource::ManualEntry { operator_id: "ops-1".into(), reason: "test".into() },
//!     datetime!(2026-01-02 09:30 UTC), // ← supplied, never sampled
//! );
//! ```
//!
//! A clock read is ambient state in the same family as I/O: it makes
//! construction non-deterministic, so two values built from identical inputs are
//! never equal and no storage layer can write a round-trip test. Callers that
//! *have* a clock pass `OffsetDateTime::now_utc()`; callers replaying an archive
//! pass the archived instant and get the archived value back. CI enforces this
//! with a grep over non-test code.
//!
//! # Time is Europe/Berlin
//!
//! German metering periods are **local calendar periods**. A day, a month and a
//! §13 StromNZV settlement period all begin at 00:00 Europe/Berlin — 23:00 UTC
//! the previous day in winter, 22:00 UTC in summer — and a day is 23 or 25 hours
//! long at the DST transitions. [`calendar`] owns those rules; [`mod@resample`]
//! buckets through it, and [`tariff_window`] and [`zaehlzeit`] resolve local
//! windows through the same tz database. Interval timestamps themselves are
//! always UTC.
//!
//! Because a calendar period has no fixed second count,
//! [`IntervalResolution::fixed_seconds`] returns `None` for `Day`, `Month` and
//! `Year` rather than an approximation — see [`calendar::intervals_in_day`].
//!
//! # Serde representation stability
//!
//! With the `serde` feature enabled, the emitted representation — enum tags and
//! struct field names — is **part of the public API and covered by semver**.
//! Consumers persisting these values are relying on a wire format, so a rename
//! is a breaking change and will be released as one. `tests/serde_representation.rs`
//! pins every tag literally so the commitment is mechanical rather than a promise.
//!
//! Each unit enum's tag is identical to its `as_str` code, so `serde`,
//! [`std::fmt::Display`] and [`std::str::FromStr`] never disagree.
//!
//! # Enum exhaustiveness
//!
//! **Domain enums here are exhaustive; only error enums are
//! `#[non_exhaustive]`.** This is a deliberate choice, not an oversight.
//!
//! `#[non_exhaustive]` buys the *library* the freedom to add a variant without
//! a major version, and charges the *consumer* a wildcard arm for it. That
//! wildcard is where the cost lands: when a new [`Messtyp`],
//! [`SubstituteMethod`] or [`QualityFlag`] appears, a consumer mapping this
//! crate's vocabulary onto their own storage codes wants their build to break,
//! so a human decides what the new variant means. With a wildcard they instead
//! get a silent fallback — a reading filed under the wrong Messtyp, a
//! substitute value attributed to the wrong method. For a crate whose output
//! ends up on an invoice, a compile error at upgrade time is the cheaper
//! failure by a wide margin.
//!
//! Error enums are the opposite case, and [`VirtualMeterError`] is marked
//! accordingly: a consumer that wildcards an unfamiliar error still does the
//! right thing — it reports a failure — so there is nothing to protect.
//!
//! The consequence is that **adding a variant to a domain enum is a breaking
//! change here**, and will be released as one. Adding a variant to an error enum
//! is not. Exhaustive `match` over `Sparte::ALL`, `QualityFlag::ALL`,
//! `MeasurementUnit::ALL` or `LoadProfile::ALL` is a supported pattern, and each
//! `ALL` is covered by a test that fails if it falls out of step with `CODES`.
//!
//! # Quick start — billing period
//!
//! ```rust
//! use metering::{MeterInterval, QualityFlag, aggregate, AggregationConfig};
//! use rust_decimal::Decimal;
//! use time::macros::datetime;
//!
//! let iv = MeterInterval {
//!     from: datetime!(2026-06-01 0:00 UTC),
//!     to:   datetime!(2026-06-01 0:15 UTC),
//!     value_kwh: Decimal::from_str_exact("2.345").unwrap(),
//!     quality: QualityFlag::Measured,
//!     obis_code: None,
//! };
//! let period = aggregate(&[iv], &AggregationConfig::rlm_strom());
//! assert!(period.arbeitsmenge_kwh > Decimal::ZERO);
//! ```
//!
//! # Quick start — Gas m³ → kWh_Hs
//!
//! ```rust
//! use metering::gas_m3_to_kwh_hs;
//! use rust_decimal::Decimal;
//!
//! let kwh = gas_m3_to_kwh_hs(
//!     Decimal::from(100u32),
//!     Decimal::from_str_exact("10.55").unwrap(),
//!     Decimal::from_str_exact("0.9764").unwrap(),
//! );
//! assert!(kwh > Decimal::from(1000u32));
//! ```
//!
//! # Quick start — quality scoring (f64 API, e.g. from DB)
//!
//! ```rust
//! use metering::{score_intervals_raw, QualityGrade};
//!
//! let values = vec![2.3_f64, 2.4, 2.3, 2.5, 2.2, 2.4, 2.3];
//! let grade = score_intervals_raw(&values, 3, 3.0);
//! assert_eq!(grade, QualityGrade::A);
//! ```
#![deny(unsafe_code)]
#![warn(missing_docs)]

pub mod aggregation;
pub mod aggregation_rule;
pub mod calendar;
pub mod classification;
pub mod conversion;
pub mod demand;
pub mod error;
pub mod forecast;
pub mod imbalance;
pub mod interval;
pub mod lifecycle;
pub mod load_profile;
pub mod losses;
pub mod measurement_point;
pub mod measurement_series;
pub mod obis;
pub mod power_quality;
pub mod quality;
pub mod register;
pub mod resample;
pub mod resolution;
pub mod rollout;
pub mod sharing;
pub mod smgw;
pub mod substitute;
pub mod tariff_window;
pub mod validation;
pub mod virtual_meter;
pub mod zaehlzeit;

// ── Re-exports ────────────────────────────────────────────────────────────────

pub use aggregation::{AggregationConfig, BillingPeriod, HtNtSplit, aggregate};
pub use aggregation_rule::AggregationRule;
pub use calendar::{DayKind, day_length, day_start_utc, intervals_in_day, local_day};
pub use classification::{Messtyp, classify_messtyp, detect_interval_length};
pub use conversion::{
    GasConversionParams, WarmWaterAdjustments, gas_m3_to_kwh_hs, normalize_interval_to_kwh,
    warm_water_heat_kwh, warm_water_heat_kwh_unmetered,
};
pub use demand::{DemandInterval, DemandWindow};
pub use error::ParseError;
pub use forecast::{
    AnnualForecast, ForecastMethod, SubstituteValueEntry, project_annual_consumption,
    substitute_values,
};
pub use imbalance::{ImbalanceSaldo, compute_imbalance};
pub use interval::{MeasurementUnit, MeterInterval, QualityFlag, Sparte, UnitScale};
pub use lifecycle::{
    MeterExchangeEvent, MeterLifecycleEvent, MeterLifecycleEventType, MeterStatus,
};
pub use load_profile::LoadProfile;
pub use losses::{NetworkLosses, network_losses};
pub use measurement_point::{EnergyFlow, MarktRolle, MeasurementPoint};
pub use measurement_series::{
    MeasurementSeries, MeasurementSource, ProvenanceEntry, ProvenanceEventType,
};
pub use obis::ObisCode;
pub use power_quality::PowerQualityInterval;
pub use quality::{
    QualityConfig, QualityGrade, QualityReport, hampel_filter, hampel_filter_with_floor,
    score_intervals, score_intervals_f64, score_intervals_raw,
};
pub use register::{EnergyDirection, MeterRegister, RegisterUnit};
pub use resample::{ResampleConfig, ResampledBucket, resample};
pub use resolution::IntervalResolution;
pub use smgw::{
    CertificateType, ClsChannel, ClsChannelStatus, ClsDeviceType, GatewayCertificate,
    GatewayStatus, SmgwSession,
};
pub use substitute::{
    FillGapsConfig, SubstituteMethod, SubstitutionReason, fill_gaps, fill_gaps_with_config,
    linear_interpolation,
};
pub use tariff_window::{HtNtSchedule, TariffWindow, TariffWindowDays};
pub use validation::{
    ValidationConfig, ValidationIssue, ValidationResult, ValidationRuleId, ValidationSeverity,
    validate_intervals,
};
pub use virtual_meter::{VirtualMeterError, compute_virtual_meter};
