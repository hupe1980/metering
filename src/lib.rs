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
//! Regulatory sources quoted in full and the longer worked examples are in the
//! [guides](https://hupe1980.github.io/metering); these docs link out to them.
//!
//! # Modules
//!
//! | Module | Contents |
//! |---|---|
//! | [`interval`] | `MeterInterval` — the Lastgang; `Sparte`, `QualityFlag` |
//! | [`reading`] | `MeterReading` — the Zählerstand, and the ZSG → Lastgang conversion |
//! | [`obis`] | Typed `ObisCode`, one canonical string per channel |
//! | [`ids`] | Typed `MaloId` and `Eic` (check-character validated), `MeloId`, `BdewCode`, `Regelzone` |
//! | [`calendar`] | Europe/Berlin days, months, years **and the 06:00 Gastag** — DST-correct; `DayBoundary` |
//! | [`holiday`] | Bundesland statutory holidays; SLP day typing |
//! | [`resolution`] | `IntervalResolution` — fixed vs calendar lengths |
//! | [`conversion`] | Gas m³ → kWh_Hs and the G 685-3 Zustandszahl, unit normalisation, HeizkostenV warm water |
//! | [`aggregation`] | Billing period: Arbeitsmenge, Spitzenleistung, coverage; the directional balance |
//! | [`zaehlzeit`] | Tariff registers — HT/NT and § 14a Modul 3, with a conformance check |
//! | [`para14a`] | § 14a netzorientierte Steuerung — `P_min,14a` and the netzwirksamer Leistungsbezug |
//! | [`mod@resample`] | Down-sampling to Berlin calendar buckets |
//! | [`validation`] | The rule engine — V01–V09, V11, V12 — order-independent, and it reports which rules ran |
//! | [`quality`] | Hampel filter, and an A/B/C/F grade over the findings |
//! | [`substitute`] | Ersatzwertbildung — 4 methods, with an audit trail |
//! | [`forecast`] | Jahresprognose with a 95 % prediction interval |
//! | [`load_profile`] | SLP classes incl. BDEW 2025 (H25/G25/L25/P25/S25) |
//! | [`gas_slp`] | Gas SLP arithmetic — SigLinDe, Allokationstemperatur, Kundenwert |
//! | [`classification`] | SLP / RLM / iMSys detection from the observed series |
//! | [`virtual_meter`] | Sum / Residual / GGV virtual meters (§ 42b EnWG), per tenant and per community |
//! | [`aggregation_rule`] | The rules `virtual_meter` evaluates |
//! | [`allocation`] | One pool across many claims — `Σ allocated + residual = total` |
//! | [`session`] | A charging session or device log onto the settlement grid, and back into one series |
//! | [`imbalance`] | Jahresmehr-/-mindermengen (GPKE Kap. 8.4) |
//! | [`losses`] | Netzverlust balance (§ 22 Abs. 1 EnWG) |
//! | [`power_quality`] | EN 50160 — statistical, over a week of 10-minute means; VDE-AR-N 4100 Unsymmetrie |
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
//! ## What "exact" means here
//!
//! **No float, and one rounding at most.** The first half is absolute: a
//! metered quantity is a `Decimal` from the wire to the invoice and never
//! passes through `f64`. The second half is the part worth stating, because
//! "exact decimal" is often read as "never rounds", and that is not true of any
//! decimal type. `Decimal` carries 28–29 significant digits, and a division
//! whose quotient does not terminate is rounded to that width — `2 ÷ 3` is
//! `0.666…7` here as anywhere.
//!
//! Addition, subtraction and multiplication of quantities at realistic scales
//! do not round at all, which is why the **conservation laws hold exactly**: a
//! register split reconstructs its Arbeitsmenge, a filled series covers its
//! grid, an allocation splits a consumption into a credited and a drawn part,
//! a resampling preserves the energy it buckets.
//! `tests/quantity_invariants.rs` asserts each of them over generated input.
//!
//! Division is where a choice has to be made, and the crate makes it one of two
//! ways:
//!
//! - **Cut to a documented number of places** when the quotient is a value a
//!   consumer stores, prints or settles on, or when an identity depends on it:
//!   [`ALLOCATION_DP`] (6), [`FORECAST_DP`] (3),
//!   [`SigLinDe::H_VALUE_DP`](gas_slp::SigLinDe::H_VALUE_DP) (6),
//!   [`KUNDENWERT_DP`](gas_slp::KUNDENWERT_DP) (4). A share carrying
//!   twenty-seven decimal places is not a quantity, and it breaks the
//!   subtraction that follows it.
//! - **Leave it at full width** when it is an intermediate nothing downstream
//!   can distinguish: [`allocation_temperature`] feeds only `h_value`, which
//!   crosses into `f64` at once.
//!
//! Where a rounding rule is the *market's* rather than this crate's it is a
//! parameter instead, with a documented default —
//! [`G685Rounding`] is the case where published
//! Netzbetreiber practice demonstrably disagrees with itself.
//!
//! A cut quantity is therefore homogeneous only to its last reported place:
//! doubling every reading doubles a projection to within `2 × 10⁻³` kWh, not
//! exactly, because `round(2x)` and `2·round(x)` differ at a rounding boundary.
//!
//! ## Why a derived quantity carries its method
//!
//! Almost nothing here is a *measured* value: a billing period is a sum, a
//! register delta a difference, a gas kWh a product, an allocation share a
//! quotient. § 25 Nr. 7 MessEV is what permits billing on any of them, and it
//! attaches a condition — *"sofern die Art der Berechnung und die verwendeten
//! Werte für den vorgesehenen Verwendungszweck geeignet sind"*. The method and
//! the inputs must be stateable, so they are: [`GasConversionParams`] has no
//! `Default`, a [`QualityFlag`] travels with every interval, [`substitute`]
//! writes an audit trail, [`ValidationResult::evaluated`] reports which rules
//! ran, and a Netzbetreiber's rounding is a parameter ([`G685Rounding`]).
//! Every number here can be re-derived from what it was given — the
//! [regulatory basis](https://hupe1980.github.io/metering/docs/regulatory-basis/)
//! quotes the provision in full.
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
//! split. EDI@Energy *Allgemeine Festlegungen zu den EDIFACT- und
//! XML-Nachrichten* v6.1c (01.10.2025, binding from 01.04.2026), Kap. 3:
//! *"Die Angabe von Zeiten in einer EDIFACT Nachricht erfolgt in koordinierter
//! Weltzeit (Coordinated Universal Time, UTC). … Alle in den Prozessen
//! genannten Zeitpunkte (inkl. der sich unter Berücksichtigung von Fristen
//! ergebenen Zeitpunkte) nutzen die gesetzliche deutsche Zeit."*
//!
//! Because a calendar period has no fixed second count,
//! [`IntervalResolution::fixed_seconds`] returns `None` for `Day`, `Month` and
//! `Year` rather than an approximation — see [`calendar::intervals_in_day`].
//!
//! **A day is cut in one of two places.** Electricity settles on the Liefertag,
//! which begins at 00:00 local; gas settles on the **Gastag**, which begins at
//! 06:00. [`DayBoundary`] carries that choice through
//! [`ResampleConfig::on`], [`FillGapsConfig::on`] and [`ValidationConfig::on`],
//! so a daily, monthly or yearly gas figure is a whole number of Gastage rather
//! than a calendar total shifted six hours — and a daily gas series is judged
//! against the Gastag's own 23, 24 or 25 hours rather than against a flat
//! 86 400 s. The length is unaffected — both are "one day" — which is why the
//! boundary is a separate parameter and not a resolution.
//!
//! Where a local wall-clock time has to become an instant, the two awkward
//! cases resolve once and consistently: the repeated autumn hour takes the
//! **earlier** pass, so periods tile; the skipped spring hour is pushed
//! **forward by the gap**, so 02:30 on the transition Sunday becomes 03:30 —
//! the convention `java.time`, `chrono` and Python's `zoneinfo` share.
//!
//! # One value, one string
//!
//! Every type here that has a string form has **exactly one** of them. The rule
//! is worth stating because the alternative fails silently: a value with two
//! spellings produces two database keys, two map entries and two "distinct" rows
//! that mean the same thing, and nothing anywhere reports an error.
//!
//! **Every coded enum carries the whole contract**: `ALL`, `CODES`, `as_str`,
//! [`Display`], [`FromStr`], and a `serde` tag that *is* the `as_str` code.
//! That last equality is what lets a consumer generate a database `CHECK`
//! constraint from `CODES` and know the two cannot drift;
//! `tests/code_contract.rs` asserts all six properties for every one of them.
//!
//! - Unit enums ([`Sparte`], [`QualityFlag`], [`MeasurementUnit`]): `as_str`,
//!   [`Display`], [`FromStr`] and the `serde` tag are the same code.
//! - A **code** and a **description** are different things, and the types with
//!   both keep them apart: [`Holiday::as_str`] is `BUSS_UND_BETTAG` and
//!   [`Holiday::name`] is *"Buß- und Bettag"*; [`RegisterUnit::as_str`] is
//!   `KILO_WATT_HOUR` and [`RegisterUnit::symbol`] is `kWh`;
//!   [`Regelzone::as_str`] is `FIFTY_HERTZ` and [`Regelzone::name`] is
//!   *"50Hertz Transmission"*, because a code that has to survive a database
//!   column does not start with a digit.
//! - Where the market's own code **is** a single character, that is the code:
//!   [`EicType::as_str`] is `"Y"`, because an EIC is quoted as
//!   `10YDE-VE-------2` and spelling the same value `AREA` somewhere else
//!   would be the second spelling this rule exists to prevent.
//! - Parsing accepts a few **input aliases** — `WÄRME` for [`Sparte::Waerme`],
//!   `DE-BY` for [`Bundesland::By`], `ÜNB` for [`MarktRolle::Uenb`] — which are
//!   never written back. An alias is not a code and is deliberately absent from
//!   `CODES`.
//!
//! [`Holiday::as_str`]: holiday::Holiday::as_str
//! [`Holiday::name`]: holiday::Holiday::name
//! [`Regelzone::as_str`]: ids::Regelzone::as_str
//! [`Regelzone::name`]: ids::Regelzone::name
//! [`EicType::as_str`]: ids::EicType::as_str
//! [`RegisterUnit::as_str`]: obis::RegisterUnit::as_str
//! [`RegisterUnit::symbol`]: obis::RegisterUnit::symbol
//! - [`ObisCode`]: `1-0:1.8.0`. The `*F` group is omitted when F is 255 ("not
//!   applicable", IEC 62056-6-1) — the spelling MSCONS carries and people type.
//! - [`IntervalResolution`]: the ISO 8601 duration, `PT15M` / `P1D`. Its
//!   `Custom` payload is opaque ([`CustomSeconds`]), so a length with a name
//!   cannot also be spelled as a custom one.
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
//! Each unit enum's tag is identical to its `as_str` code, and the types with a
//! canonical string ([`ObisCode`], [`IntervalResolution`], [`MaloId`],
//! [`MeloId`], [`BdewCode`]) serialise *as* that string, so `serde`,
//! [`std::fmt::Display`] and [`std::str::FromStr`] never disagree. `ObisCode` in
//! particular is stable for a further reason: it is an IEC 62056 identifier and
//! `IntervalResolution` an ISO 8601 duration — both external standards, which no
//! refactor in this crate can rename.
//!
//! ## Instants are RFC 3339, dates are ISO 8601 — in a readable format
//!
//! An instant travels as `"2026-06-01T12:00:00Z"` and a calendar date as
//! `"2026-06-01"`, which is what a `TIMESTAMPTZ` cast, a JSON Schema
//! `format: date-time` and every log viewer already understand.
//!
//! In a **binary** format they keep `time`'s own compact tuple instead. That
//! split is deliberate: [`MeterInterval`] carries two instants and is the
//! hottest type here, and a twenty-byte string per boundary is a poor trade in
//! a format chosen for its packing. `serde` is asked which kind of format it is
//! and the answer decides — see the private `wire` module.
//!
//! ## Quantities are exact decimal strings
//!
//! A quantity travels as `"12.345"` — the characters its `Display` writes — in
//! every format, readable or binary; a decimal string is already the compact
//! form. Reading one back asks for a **string**, so a JSON number is a type
//! error rather than a silent trip through `f64`, and more digits than a
//! `Decimal` holds are refused rather than rounded away.
//!
//! Each field states this itself rather than borrowing `rust_decimal`'s
//! feature-gated impls, because those features are global to a build graph:
//! `serde-str` here would change how every `Decimal` deserialises in crates
//! that never named `metering`, and `serde-float` set by any of them would
//! decide how *these* quantities serialise.
//!
//! ## The hot types survive a non-self-describing format
//!
//! `MeterInterval`, `ObisCode`, [`MeterReading`] and the identifiers round-trip
//! through bincode and postcard. The types with an internally tagged `serde`
//! shape — [`AggregationRule`], [`AllocationKey`] — deliberately do not: a
//! discriminator at a fixed, queryable path is worth more for configuration
//! stored once per delivery point than binary compactness is.
//!
//! Both halves are pinned by a test. Asking for a string is what makes the
//! first half hold: `deserialize_any` is the one question a format without a
//! self-describing wire cannot answer, and it is what an inherited `Decimal`
//! impl asks.
//!
//! # A clean report is not the same as a clean series
//!
//! Four of the eleven validation rules are **opt-in**: they need a number this
//! library refuses to invent — a grid spacing, an outlier threshold, a
//! reference instant, a plant capacity — and leaving the corresponding
//! [`ValidationConfig`] field `None` turns the rule off. Two more are
//! **opt-out**: `negative_energy_is_error = false` retires V03, and a
//! `zero_run_threshold` of `0` retires V05. A [`ValidationResult`] with no
//! issues therefore means *"the rules that ran found nothing"*, which is weaker
//! than "nothing is wrong".
//!
//! So the crate says which ran: [`ValidationConfig::disabled_rules`] before a
//! run, [`ValidationResult::evaluated`] after one, and
//! [`ValidationRuleId::enabling_field`] names the setting that would arm a rule.
//!
//! ```rust
//! use metering::{QualityConfig, Sparte, ValidationRuleId};
//!
//! // A "now" and a nameplate capacity are not properties of a commodity.
//! let cfg = QualityConfig::for_sparte(Sparte::Strom);
//! assert_eq!(cfg.validation.disabled_rules().to_string(), "V08, V12");
//! assert_eq!(
//!     ValidationRuleId::ImplausiblePower.enabling_field(),
//!     Some("max_plant_power_kw"),
//! );
//! ```
//!
//! # Order in, order out
//!
//! Where a function takes a slice of intervals, the order of that slice does
//! not change the answer. A MSCONS delivery merged from two files, a database
//! query without an `ORDER BY` and a `HashMap` iteration all arrive shuffled,
//! so [`aggregate`], [`mod@resample`], [`validate_intervals`], [`fill_gaps`],
//! [`Zaehlzeitdefinition::split_energy`] and [`to_lastgang`] each sort, index
//! or fold over an operation that commutes.
//!
//! A **tie** is where the promise is easiest to break, so both places one can
//! occur are settled explicitly: [`QualityFlag::severity_rank`] gives every
//! flag a distinct rank, and [`aggregate`] breaks a tied peak by the earliest
//! instant. `tests/order_independence.rs` asserts the whole property under
//! proptest.
//!
//! [`spitzenleistung_at`]: BillingPeriod::spitzenleistung_at
//! [`Zaehlzeitdefinition::split_energy`]: zaehlzeit::Zaehlzeitdefinition::split_energy
//!
//! # Enum exhaustiveness
//!
//! **Domain enums here are exhaustive; only error enums are
//! `#[non_exhaustive]`.**
//!
//! `#[non_exhaustive]` charges the consumer a wildcard arm, and that is where
//! the cost lands: a consumer mapping a new [`Messtyp`] or [`SubstituteMethod`]
//! onto their own storage codes wants their build to break so a human decides
//! what it means, not a silent fallback filing a reading under the wrong one.
//! For output that ends up on an invoice, a compile error at upgrade time is
//! the cheaper failure. An unfamiliar *error* needs no such protection — a
//! wildcard still reports a failure — so exactly three types carry the
//! attribute, and all three are failure vocabularies: [`VirtualMeterError`],
//! [`ConversionError`] and [`AnomalyKind`], the reason an [`Anomaly`] refused a
//! difference.
//!
//! So **adding a variant to a domain enum is a breaking change here** and is
//! released as one, and exhaustive `match` over any `ALL` is a supported
//! pattern.
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

#[macro_use]
mod codes;
mod wire;

pub mod aggregation;
pub mod aggregation_rule;
pub mod allocation;
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
pub mod para14a;
pub mod power_quality;
pub mod quality;
pub mod reading;
pub mod resample;
pub mod resolution;
pub mod rollout;
pub mod session;
pub mod sharing;
pub mod substitute;
pub mod validation;
pub mod virtual_meter;
pub mod zaehlzeit;

// ── Re-exports ────────────────────────────────────────────────────────────────

pub use aggregation::{
    AggregationConfig, BillingPeriod, DirectionalEnergy, aggregate, sum_by_direction,
};
pub use aggregation_rule::{AggregationRule, VirtualMeterKind};
pub use allocation::{
    ALLOCATION_DP, AllocatedPart, AllocationBasis, AllocationError, AllocationPart, AllocationRow,
    allocate, allocation_share, validate_key,
};
pub use calendar::{
    DayBoundary, DayKind, day_length, day_start_utc, gas_day_start_utc, intervals_in_day,
    local_day, local_gas_day,
};
pub use classification::{Messtyp, SeriesOrigin, classify_messtyp, detect_interval_length};
pub use conversion::{
    ConversionError, G685FinalRounding, G685Rounding, GasConversionParams, WarmWaterAdjustments,
    ZustandszahlParams, gas_m3_to_kwh_hs, gas_m3_to_kwh_hs_rounded, hoehenzonen_luftdruck_mbar,
    normalize_to_kwh, warm_water_heat_kwh, warm_water_heat_kwh_unmetered, zustandszahl,
};
pub use error::ParseError;
pub use forecast::{AnnualForecast, FORECAST_DP, project_annual_consumption};
pub use gas_slp::{
    SigLinDe, WeekdayFactors, allocation_temperature, gas_daily_quantity, kundenwert,
};
pub use holiday::{Bundesland, Holiday, slp_day_type};
pub use ids::{BdewCode, CodeVergabestelle, Eic, EicType, MaloId, MaloIssuer, MeloId, Regelzone};
pub use imbalance::{ImbalanceSaldo, compute_imbalance};
pub use interval::{Direction, MeasurementUnit, MeterInterval, QualityFlag, Sparte, UnitScale};
pub use lifecycle::{
    MeterExchangeEvent, MeterLifecycleEvent, MeterLifecycleEventType, MeterStatus,
};
pub use load_profile::{DynamicSlpProfile, Dynamization, LoadProfile, SlpDayType, SlpValueTable};
pub use losses::{NetworkLosses, network_losses};
pub use measurement_point::{EnergyFlow, MarktRolle, MeasurementPoint};
pub use measurement_series::{
    MeasurementSeries, MeasurementSource, ProvenanceEntry, ProvenanceEventType,
};
pub use obis::{ObisCode, RegisterUnit};
pub use para14a::{
    GLEICHZEITIGKEITSFAKTOREN, MINDESTLEISTUNG_KW, Para14aConfig, STEUVE_SCHWELLE_KW, SteuVe,
    SteuVeFallgruppe, Verursachungsregel, gleichzeitigkeitsfaktor,
    mindestleistung_direktansteuerung, mindestleistung_ems, netzwirksamer_leistungsbezug,
};
pub use power_quality::{
    En50160Limits, En50160Report, LimitOutcome, Phase, PhaseApparentPower, PowerQualityInterval,
    UNSYMMETRIE_LIMIT_KVA, assess_en50160, exceedance_pct, voltage_percentile,
};
pub use quality::{
    K_MAD, QualityConfig, QualityGrade, QualityReport, hampel_filter, hampel_filter_with_floor,
    score_intervals,
};
pub use reading::{
    Anomaly, AnomalyKind, Lastgang, LastgangConfig, MeterReading, ResultChannel, Rollover,
    consumption_between, detect_reading_cadence, to_lastgang,
};
pub use resample::{ResampleConfig, ResampledBucket, resample};
pub use resolution::{CustomSeconds, IntervalResolution};
pub use rollout::{
    QuotaScope, ROLLOUT_MILESTONES, RolloutMilestone, RolloutObligation,
    classify_rollout_obligation, next_milestone,
};
pub use session::{MeterSample, SessionError, SessionSplitConfig, merge_sessions, split_session};
pub use sharing::{
    Bilanzierungsmethode, Capability, Delivery, DeliveryEvidenceInput, EligibilityBasis, Finding,
    MeteringCapabilityInput, SharingReadiness, Zaehlertyp, assess_capability, assess_delivery,
    combine_readiness,
};
pub use substitute::{
    FillGapsConfig, FilledSeries, SubstituteEntry, SubstituteMethod, SubstitutionReason, fill_gaps,
};
pub use validation::{
    RuleSet, ValidationConfig, ValidationIssue, ValidationResult, ValidationRuleId,
    ValidationSeverity, validate_intervals,
};
pub use virtual_meter::{
    AllocationKey, CommunityInterval, GgvInterval, ParticipantAllocation, VirtualMeterError,
    compute_community_allocation, compute_ggv_allocation, compute_virtual_meter,
};
pub use zaehlzeit::{
    DayGroup, Modul3Conformance, Modul3Context, Modul3Finding, Quarter, ZaehlzeitFenster,
    Zaehlzeitdefinition, assess_modul_3,
};
