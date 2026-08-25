# ⚡ metering

[![Crates.io](https://img.shields.io/crates/v/metering.svg)](https://crates.io/crates/metering)
[![Docs.rs](https://docs.rs/metering/badge.svg)](https://docs.rs/metering)
[![CI](https://github.com/hupe1980/metering/actions/workflows/ci.yml/badge.svg)](https://github.com/hupe1980/metering/actions/workflows/ci.yml)
[![MSRV](https://img.shields.io/badge/MSRV-1.94-blue.svg)](https://blog.rust-lang.org/)
[![License](https://img.shields.io/crates/l/metering.svg)](#-license)

**German energy metering domain library for Rust.** Europe/Berlin calendar
arithmetic — Liefertag *and* Gastag — Zählerstandsgang → Lastgang, gas
m³→kWh_Hs and the SigLinDe gas SLP, Ersatzwertbildung, a robust validation
engine, EN 50160, §14a Modul 3 tariff registers, virtual meters (§42b EnWG),
check-digit-validated MaLo-IDs and Jahresprognose.

> 🧊 **Zero I/O** · ⏱️ **no async** · 🕰️ **no clock** · 🔢 **exact decimal quantities**

It computes **quantities, not money**: what leaves this crate is kWh, m³ and kW,
which a billing layer then prices.

📖 **[Documentation & guides](https://hupe1980.github.io/metering)** ·
🦀 **[API reference](https://docs.rs/metering)**

---

## 📦 Installation

```bash
cargo add metering

# ...or with serde for every public type:
cargo add metering --features serde
```

**MSRV:** Rust `1.94` (edition 2024), pinned in
[`rust-toolchain.toml`](rust-toolchain.toml) and verified by a CI lane.

---

## 🚀 Quick start

```rust
use metering::{AggregationConfig, MeterInterval, QualityFlag, aggregate};
use rust_decimal::dec;
use time::macros::datetime;

let intervals = vec![MeterInterval {
    from: datetime!(2026-06-01 0:00 UTC),
    to:   datetime!(2026-06-01 0:15 UTC),
    value: dec!(2.345),
    quality: QualityFlag::Measured,
    obis_code: Some("1-0:1.8.0".parse().unwrap()),
}];

let period = aggregate(&intervals, &AggregationConfig::rlm());
println!("Arbeitsmenge:    {} kWh", period.arbeitsmenge);
println!("Spitzenleistung: {:?} kW", period.spitzenleistung_kw);
println!("...reached at:   {:?}", period.spitzenleistung_at);
println!("Coverage:        {:.1} %", period.coverage_pct);
```

---

## 🗓️ Why this exists: a German day is not 24 hours

The single most consequential thing the crate gets right.

| Day | Length | Quarter-hours |
|---|---|---|
| ordinary | 24 h | 96 |
| last Sunday in March (spring forward) | **23 h** | **92** |
| last Sunday in October (fall back) | **25 h** | **100** |

```rust
use metering::{IntervalResolution, calendar};
use time::macros::{date, datetime};

assert_eq!(calendar::intervals_in_day(date!(2026 - 03 - 29), IntervalResolution::QuarterHour), Some(92));
assert_eq!(calendar::intervals_in_day(date!(2026 - 10 - 25), IntervalResolution::QuarterHour), Some(100));

// March 2026 holds 2 972 quarter-hours, not 31 × 96 = 2 976.
assert_eq!(calendar::intervals_in_month(date!(2026 - 03 - 01), IntervalResolution::QuarterHour), Some(2_972));

// A German day starts at 23:00 UTC in winter, 22:00 UTC in summer.
assert_eq!(calendar::day_start_utc(date!(2026 - 01 - 15)), datetime!(2026-01-14 23:00 UTC));
assert_eq!(calendar::day_start_utc(date!(2026 - 07 - 15)), datetime!(2026-07-14 22:00 UTC));
```

A completeness check built on a hard-coded 96 raises a false alarm every spring
and — worse — **hides a genuine four-interval gap every autumn**, because 96 of
an expected 100 looks complete.

### ...and gas does not start its day at midnight

Gas balances on the **Gastag**, 06:00 to 06:00 local. That is the same day cut
six hours later, not a different length — so it is a *boundary*, and
`DayBoundary` carries it into the places a daily grid is actually built:

```rust
use metering::{MeterInterval, QualityFlag, ResampleConfig, calendar, resample};
use rust_decimal::dec;
use time::{Duration, macros::date};

let start = calendar::gas_day_start_utc(date!(2026 - 01 - 15));
let series: Vec<MeterInterval> = (0..48).map(|i| MeterInterval {
    from: start + Duration::hours(i),
    to:   start + Duration::hours(i + 1),
    value: dec!(1),
    quality: QualityFlag::Measured,
    obis_code: None,
}).collect();

// Two whole Gastage...
let gas_days = resample(&series, &ResampleConfig::to_gas_daily());
assert_eq!(gas_days.len(), 2);
assert_eq!(gas_days[0].is_complete(), Some(true));

// ...where the calendar day makes three partial buckets out of the same data.
let calendar_days = resample(
    &series,
    &ResampleConfig::new(metering::IntervalResolution::Hour, metering::IntervalResolution::Day),
);
assert_eq!(calendar_days.len(), 3);
```

→ [Time and the calendar](https://hupe1980.github.io/metering/docs/time-and-calendar/)

### The whole pipeline, end to end

```bash
cargo run --example pipeline
```

[`examples/pipeline.rs`](examples/pipeline.rs) walks one Liefertag from the
Zählerstandsgang the gateway delivered through to the §14a register split —
differencing, validation, Ersatzwertbildung, aggregation and grading — on the
25-hour autumn DST day, with a wrapped register and a corrupt reading planted in
it. It asserts its own invariants, so CI runs it as a test.

---

## 🧭 What's in it

| Area | What it does | Guide |
|---|---|---|
| **Calendar** | Berlin days, months, years **and the 06:00 Gastag**; `DayBoundary` carries the choice into resampling and gap filling; DST-correct interval counts; Bundesland holidays | [→](https://hupe1980.github.io/metering/docs/time-and-calendar/) |
| **Readings** | Zählerstandsgang → Lastgang, register rollover, meter exchange | [→](https://hupe1980.github.io/metering/docs/readings/) |
| **Identifiers** | `MaloId` with the BDEW check digit verified at the parse; `MeloId` | [→](https://hupe1980.github.io/metering/docs/identifiers/) |
| **Validation** | Order-independent rules V01–V12 (V10 retired), Hampel outlier test, A/B/C/F grading, and a `RuleSet` saying which rules actually ran | [→](https://hupe1980.github.io/metering/docs/validation/) |
| **Ersatzwerte** | Four methods, calendar-aware grid, audit trail of what actually ran | [→](https://hupe1980.github.io/metering/docs/substitute-values/) |
| **Tariff registers** | HT/NT and §14a Modul 3 in one mechanism | [→](https://hupe1980.github.io/metering/docs/tariff-registers/) |
| **Gas & units** | m³→kWh_Hs, G 685 rounding, the SigLinDe gas SLP, exact-rational unit normalisation | [→](https://hupe1980.github.io/metering/docs/gas-and-units/) |
| **Power quality** | EN 50160 as the statistical test it actually is | [→](https://hupe1980.github.io/metering/docs/power-quality/) |
| **Virtual meters** | Sum, Residual, GGV allocation (§42b EnWG) — net grid draw *and* the allocated share | [→](https://hupe1980.github.io/metering/docs/virtual-meters/) |
| **End to end** | The full MSB pipeline as a runnable example | [→](https://hupe1980.github.io/metering/docs/pipeline/) |

---

## 🎯 Scope

This crate computes quantities. Four neighbouring concerns are deliberately
**out of scope**, each with a better home:

| Not here | Why | Where instead |
|---|---|---|
| Money — prices, tariffs, invoices | the output is kWh, m³ and kW | your billing layer |
| EDIFACT / XML market messages | parsing a MSCONS is not arithmetic | [`mako`](https://github.com/hupe1980/mako) |
| Fristen — counting Werktage to a deadline | a process-engine concern | your process engine |
| SMGW certificates, device inventory | PKI and asset tracking, not quantities | your device management |

The third is the least obvious. The crate *does* carry a German statutory
holiday calendar, because SLP day typing and tariff-register classification
cannot be done without one — but it counts no business days.

---

## 🧱 Design constraints

- **Determinism.** No function reads the system clock, the filesystem or the
  network. Where an instant is needed it is a parameter, so equal inputs give
  equal outputs. CI enforces this with a grep over non-test code.
- **Exact decimals for quantities, `f64` only for statistics.** The two meet in
  one place — the outlier rule converts values to run the Hampel filter — and
  nothing a float touches is written back into a quantity.
- **One value, one string — and one meaning, one value.** `ObisCode` and
  `IntervalResolution` each have exactly one canonical spelling, and two
  distinct values can never mean the same thing: `IntervalResolution::Custom`
  refuses a length that already has a name. Both held by a proptest suite.
- **Every coded enum carries the whole contract** — `ALL`, `CODES`, `as_str`,
  `Display`, `FromStr`, and a `serde` tag that *is* the code. Generate a
  database `CHECK` constraint from `CODES` and it cannot drift from what the
  crate writes; one test asserts all six properties for all thirty-four enums.
- **A clean validation report says which rules ran.** Four of the eleven are
  opt-in — they need a number the library will not invent — so
  `ValidationResult::evaluated` and `ValidationConfig::disabled_rules()` make
  the difference between "found nothing" and "never looked" a fact you can log
  or assert on.
- **No second copy of a fact.** A register's unit comes from its OBIS code, a
  meter exchange's date from its instant, a series' worst quality from its
  intervals — never from a field that can drift out of step.
- **Serde tags are semver-covered**, pinned literally by a test.
- **Domain enums are exhaustive**; only error enums are `#[non_exhaustive]`.
- **Unknown is not good.** Where a quantity cannot be determined the API says
  so — an `Option`, or an error — rather than returning a benign-looking
  default.

→ [Design constraints](https://hupe1980.github.io/metering/docs/design/)

---

## ⚖️ Regulatory basis

Every provision is quoted from the published text and dated, and the library is
explicit about the claims it *cannot* verify — the 2025 SLP dynamisation
function is published as an image, G 685's final rounding diverges between
Netzbetreiber, VDE-AR-N 4400 is paywalled. Those are parameters, not constants.

→ [Regulatory basis](https://hupe1980.github.io/metering/docs/regulatory-basis/)

Nothing here is legal advice.

---

## 🧪 Testing

```bash
just ci     # everything CI runs, in CI order
just test   # cargo test --all-features
just purity # no clock, no I/O, no unsafe outside comments
just site   # serve the documentation site locally
```

Beyond the unit tests:

- `tests/code_contract.rs` — every coded enum, six properties each: `ALL` vs
  `CODES`, `as_str` is `Display`, `FromStr` inverts it, codes are distinct, the
  `serde` tag *is* the code, and an unknown code is an error
- `tests/berlin_calendar.rs` — DST interval counts against the tz database
- `tests/string_canonicalisation.rs` — proptest: stability, totality,
  idempotence, injectivity of every string form
- `tests/serde_representation.rs` — every wire tag pinned literally
- `tests/proptest_validation.rs` — validation invariants under random input
- `tests/regulatory_showcase.rs` — worked examples from the published sources
- `tests/doc_samples.rs` — every code block in this README and on the
  documentation site, compiled and run

---

## 🤝 Contributing

Issues and pull requests welcome. A change that touches a regulated calculation
should cite the provision it implements — and if a citation here is wrong,
saying so is the most valuable issue you can file.

---

## 📄 License

Licensed under either of [Apache-2.0](LICENSE-APACHE) or [MIT](LICENSE-MIT) at
your option.
