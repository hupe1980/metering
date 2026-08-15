//! German energy metering domain library.
//!
//! A **standalone**, **pure** library for the quantity calculations German
//! energy metering requires — MsbG, EnWG, StromNEV, MessEV and the BNetzA
//! Festlegungen. Zero I/O, no async, no clock, exact decimal quantities.
//!
//! It computes **energy and volume**, not money: there is no currency, price or
//! tariff-rate type here. What leaves this crate is kWh, m³ and kW, which a
//! billing layer then prices.
//!
//! # Modules
//!
//! | Module | Contents |
//! |---|---|
//! | [`interval`] | `MeterInterval` — the Lastgang; `Sparte`, `QualityFlag` |
//! | [`reading`] | `MeterReading` — the Zählerstand, and the ZSG → Lastgang conversion |
//! | [`obis`] | Typed `ObisCode`, one canonical string per channel |
//! | [`ids`] | Typed `MaloId` (check-digit validated) and `MeloId` |
//! | [`calendar`] | Europe/Berlin days, months, years **and the 06:00 Gastag** — DST-correct |
//! | [`holiday`] | Bundesland statutory holidays; SLP day typing |
//! | [`resolution`] | `IntervalResolution` — fixed vs calendar lengths |
//! | [`conversion`] | Gas m³ → kWh_Hs, unit normalisation, HeizkostenV warm water |
//! | [`aggregation`] | Billing period: Arbeitsmenge, Spitzenleistung, coverage |
//! | [`zaehlzeit`] | Tariff registers — HT/NT and § 14a Modul 3 |
//! | [`mod@resample`] | Down-sampling to Berlin calendar buckets |
//! | [`validation`] | The rule engine — V01–V09, V11, V12 — order-independent |
//! | [`quality`] | Hampel filter, and an A/B/C/F grade over the findings |
//! | [`substitute`] | Ersatzwertbildung — 4 methods, with an audit trail |
//! | [`forecast`] | Jahresprognose with a 95 % prediction interval |
//! | [`load_profile`] | SLP classes incl. BDEW 2025 (H25/G25/L25/P25/S25) |
//! | [`gas_slp`] | Gas SLP arithmetic — SigLinDe, Allokationstemperatur, Kundenwert |
//! | [`classification`] | SLP / RLM / iMSys detection from the observed series |
//! | [`virtual_meter`] | Sum / Residual / GGV virtual meters (§ 42b EnWG) |
//! | [`aggregation_rule`] | The rules `virtual_meter` evaluates |
//! | [`imbalance`] | Jahresmehr-/-mindermengen (GPKE Kap. 8.4) |
//! | [`losses`] | Netzverlust balance (§ 22 Abs. 1 EnWG) |
//! | [`power_quality`] | EN 50160 — statistical, over a week of 10-minute means |
//! | [`measurement_point`] | What is metered, and on whose account |
//! | [`measurement_series`] | A named series with its provenance |
//! | [`lifecycle`] | Meter installation, exchange and retirement events |
//! | [`rollout`] | § 29 MsbG Pflichteinbaufälle + § 45 MsbG Rollout-Fahrplan |
//! | [`sharing`] | § 42c EnWG Energy-Sharing eligibility |
//! | [`error`] | `ParseError` — the one error every `FromStr` here returns |
//!
//! # Numeric types
//!
//! Every **metered quantity** is [`rust_decimal::Decimal`] — `value`,
//! `arbeitsmenge`, `spitzenleistung_kw`, the per-register split, the gas
//! m³→kWh_Hs conversion, GGV allocations and substitute values are exact
//! decimal arithmetic end to end, never routed through a float and back.
//!
//! `f64` is used, deliberately, for **statistics and diagnostics**:
//! `coverage_pct`, the Hampel filter's median absolute deviation and threshold,
//! and the forecast prediction interval. Those are comparisons and indicators,
//! not amounts.
//!
//! The two meet in exactly one place: the V04 outlier rule converts values to
//! `f64` to run the Hampel filter over them. That comparison decides whether to
//! *flag* an interval; it never alters one. Nothing a float touches is ever
//! written back into a quantity.
//!
//! # Determinism
//!
//! **No function in this crate reads the system clock, the filesystem, the
//! network or any other ambient state.** Every input is an argument, so equal
//! inputs always produce equal outputs, and any result can be replayed or
//! cached. Where a timestamp is needed — a provenance entry, a validity check,
//! a "was this reading in the future" test — it is a parameter:
//!
//! ```rust
//! # use metering::{MeasurementSeries, MeasurementSource};
//! # use time::macros::datetime;
//! let series = MeasurementSeries::new(
//!     "51238696781".parse()?, // a MaloId — the check digit is verified
//!     None,
//!     vec![],
//!     MeasurementSource::ManualEntry { operator_id: "ops-1".into(), reason: "test".into() },
//!     datetime!(2026-01-02 09:30 UTC), // ← supplied, never sampled
//! );
//! # Ok::<(), metering::ParseError>(())
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
//! German metering periods are **local calendar periods**. A Liefertag, a
//! Liefermonat and a settlement year all begin at 00:00 Europe/Berlin — 23:00
//! UTC the previous day in winter, 22:00 UTC in summer — and a day is 23 or 25
//! hours long at the DST transitions. [`calendar`] owns those rules;
//! [`mod@resample`] buckets through it, and [`zaehlzeit`] and
//! [`holiday`] resolve local windows and dates through the same tz database.
//!
//! Interval timestamps themselves are always **UTC**, which is the market's own
//! split. EDI@Energy *Allgemeine Festlegungen* v6.1b, Kap. 3: *"Die Angabe von
//! Zeiten in einer EDIFACT Nachricht erfolgt in koordinierter Weltzeit (UTC) …
//! Alle in den Prozessen genannten Zeitpunkte … nutzen die gesetzliche deutsche
//! Zeit."*
//!
//! Because a calendar period has no fixed second count,
//! [`IntervalResolution::fixed_seconds`] returns `None` for `Day`, `Month` and
//! `Year` rather than an approximation — see [`calendar::intervals_in_day`].
//!
//! # One value, one string
//!
//! Every type here that has a string form has **exactly one** of them. The rule
//! is worth stating because the alternative fails silently: a value with two
//! spellings produces two database keys, two map entries and two "distinct" rows
//! that mean the same thing, and nothing anywhere reports an error.
//!
//! - Unit enums ([`Sparte`], [`QualityFlag`], [`MeasurementUnit`]): `as_str`,
//!   [`Display`], [`FromStr`] and the `serde` tag are the same code.
//! - [`ObisCode`]: `1-0:1.8.0`. The `*F` group is omitted when F is 255 ("not
//!   applicable", IEC 62056-6-1) — the spelling MSCONS carries and people type.
//! - [`IntervalResolution`]: the ISO 8601 duration, `PT15M` / `P1D`.
//! - [`MaloId`]: the eleven digits, with the **check digit verified** at the
//!   parse; [`MeloId`]: the 33-character Zählpunktbezeichnung, uppercased.
//!
//! Parsing is deliberately lenient where writing is not: [`ObisCode`] also
//! accepts `1-0:1.8.0*255`, leading zeros and surrounding whitespace, and
//! [`IntervalResolution`] accepts `PT900S` and lower case. Every accepted
//! spelling maps onto the one canonical output, so
//!
//! ```rust
//! # use metering::ObisCode;
//! // ...whichever spelling arrived, one key comes out.
//! assert_eq!(ObisCode::normalize("1-0:1.8.0*255")?, "1-0:1.8.0");
//! assert_eq!(ObisCode::normalize("  1-0:01.8.0 ")?, "1-0:1.8.0");
//! # Ok::<(), metering::ParseError>(())
//! ```
//!
//! `s.parse()?.to_string() == s` holds for every canonical `s`, and
//! `tests/string_canonicalisation.rs` holds stability, totality, idempotence and
//! injectivity under proptest.
//!
//! Where a code legitimately carries a storage group — a historical billing
//! period, `1-0:1.8.0*1` — it is never elided, because there the suffix is
//! information rather than noise.
//!
//! [`Display`]: std::fmt::Display
//! [`FromStr`]: std::str::FromStr
//!
//! # Serde representation stability
//!
//! With the `serde` feature enabled, the emitted representation — enum tags and
//! struct field names — is **part of the public API and covered by semver**.
//! Consumers persisting these values are relying on a wire format, so a rename
//! is a breaking change and will be released as one. `tests/serde_representation.rs`
//! pins every tag literally so the commitment is mechanical rather than a promise.
//!
//! You do not need to define your own storage codes to insulate yourself from
//! renames here; if you would rather anyway, that is a deliberate choice and not
//! a hedge against an unstated policy.
//!
//! Each unit enum's tag is identical to its `as_str` code, and the two types
//! with a canonical string ([`ObisCode`], [`IntervalResolution`]) serialise *as*
//! that string, so `serde`, [`std::fmt::Display`] and [`std::str::FromStr`]
//! never disagree. `ObisCode` in particular is stable for a further reason: it
//! is an IEC 62056 identifier and `IntervalResolution` an ISO 8601 duration —
//! both external standards, which no refactor in this crate can rename.
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
//!     value: Decimal::from_str_exact("2.345").unwrap(),
//!     quality: QualityFlag::Measured,
//!     obis_code: None,
//! };
//! let period = aggregate(&[iv], &AggregationConfig::rlm());
//! assert!(period.arbeitsmenge > Decimal::ZERO);
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
//! # Quick start — validate, then grade
//!
//! ```rust
//! use metering::{MeterInterval, QualityConfig, QualityGrade, QualityFlag, score_intervals};
//! use rust_decimal::dec;
//! use time::{Duration, macros::datetime};
//!
//! let day: Vec<MeterInterval> = (0..96).map(|i| MeterInterval {
//!     from: datetime!(2026-06-01 0:00 UTC) + Duration::minutes(i * 15),
//!     to:   datetime!(2026-06-01 0:00 UTC) + Duration::minutes(i * 15 + 15),
//!     value: dec!(2.0),
//!     quality: QualityFlag::Measured,
//!     obis_code: None,
//! }).collect();
//!
//! let report = score_intervals(&day, &QualityConfig::default());
//! assert_eq!(report.grade, QualityGrade::A);
//! assert!(report.issues.is_empty());
//! ```
//!
//! # Scope — what belongs here and what does not
//!
//! This crate computes **quantities**. It deliberately excludes three
//! neighbouring concerns, each of which has its own home:
//!
//! | Not here | Why | Where |
//! |---|---|---|
//! | Money — prices, tariffs, invoices | what leaves here is kWh, m³ and kW | a billing layer |
//! | EDIFACT / XML market messages | parsing a MSCONS is not arithmetic | [`mako`](https://github.com/hupe1980/mako) |
//! | Fristen — counting Werktage to a GPKE deadline | a process-engine concern | a process engine |
//!
//! The third is the least obvious. [`holiday`] does carry a German statutory
//! holiday calendar, because SLP day typing and HT/NT classification cannot be
//! done without one — but it counts no business days and knows nothing of the
//! EDI@Energy rule that a holiday in one Bundesland counts nationwide.
#![deny(unsafe_code)]
#![warn(missing_docs)]

pub mod aggregation;
pub mod aggregation_rule;
pub mod calendar;
pub mod classification;
pub mod conversion;
pub mod error;
pub mod forecast;
pub mod gas_slp;
pub mod holiday;
pub mod ids;
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
pub mod reading;
pub mod resample;
pub mod resolution;
pub mod rollout;
pub mod sharing;
pub mod substitute;
pub mod validation;
pub mod virtual_meter;
pub mod zaehlzeit;

// ── Re-exports ────────────────────────────────────────────────────────────────

pub use aggregation::{AggregationConfig, BillingPeriod, aggregate};
pub use aggregation_rule::{AggregationRule, VirtualMeterKind};
pub use calendar::{DayKind, day_length, day_start_utc, intervals_in_day, local_day};
pub use classification::{Messtyp, SeriesOrigin, classify_messtyp, detect_interval_length};
pub use conversion::{
    G685FinalRounding, G685Rounding, GasConversionParams, WarmWaterAdjustments, gas_m3_to_kwh_hs,
    gas_m3_to_kwh_hs_rounded, normalize_to_kwh, warm_water_heat_kwh, warm_water_heat_kwh_unmetered,
};
pub use error::ParseError;
pub use forecast::{AnnualForecast, project_annual_consumption};
pub use gas_slp::{
    SigLinDe, WeekdayFactors, allocation_temperature, gas_daily_quantity, kundenwert,
};
pub use holiday::{Bundesland, Holiday, slp_day_type};
pub use ids::{MaloId, MaloIssuer, MeloId};
pub use imbalance::{ImbalanceSaldo, compute_imbalance};
pub use interval::{MeasurementUnit, MeterInterval, QualityFlag, Sparte, UnitScale};
pub use lifecycle::{
    MeterExchangeEvent, MeterLifecycleEvent, MeterLifecycleEventType, MeterStatus,
};
pub use load_profile::{DynamicSlpProfile, Dynamization, LoadProfile, SlpDayType};
pub use losses::{NetworkLosses, network_losses};
pub use measurement_point::{EnergyFlow, MarktRolle, MeasurementPoint};
pub use measurement_series::{
    MeasurementSeries, MeasurementSource, ProvenanceEntry, ProvenanceEventType,
};
pub use obis::{ObisCode, RegisterUnit};
pub use power_quality::{
    En50160Limits, En50160Report, LimitOutcome, PowerQualityInterval, assess_en50160,
};
pub use quality::{
    K_MAD, QualityConfig, QualityGrade, QualityReport, hampel_filter, hampel_filter_with_floor,
    score_intervals,
};
pub use reading::{
    Anomaly, AnomalyKind, Lastgang, LastgangConfig, MeterReading, Rollover, to_lastgang,
};
pub use resample::{ResampleConfig, ResampledBucket, resample};
pub use resolution::IntervalResolution;
pub use rollout::{RolloutObligation, classify_rollout_obligation};
pub use sharing::{Bilanzierungsmethode, Capability, Delivery, SharingReadiness, Zaehlertyp};
pub use substitute::{
    FillGapsConfig, FilledSeries, SubstituteEntry, SubstituteMethod, SubstitutionReason, fill_gaps,
};
pub use validation::{
    ValidationConfig, ValidationIssue, ValidationResult, ValidationRuleId, ValidationSeverity,
    validate_intervals,
};
pub use virtual_meter::{VirtualMeterError, compute_virtual_meter};
pub use zaehlzeit::{DayGroup, ZaehlzeitFenster, Zaehlzeitdefinition};
