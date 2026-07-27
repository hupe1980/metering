# ⚡ metering

[![Crates.io](https://img.shields.io/crates/v/metering.svg)](https://crates.io/crates/metering)
[![Docs.rs](https://docs.rs/metering/badge.svg)](https://docs.rs/metering)
[![CI](https://github.com/hupe1980/metering/actions/workflows/ci.yml/badge.svg)](https://github.com/hupe1980/metering/actions/workflows/ci.yml)
[![MSRV](https://img.shields.io/badge/MSRV-1.94-blue.svg)](https://blog.rust-lang.org/)
[![License](https://img.shields.io/crates/l/metering.svg)](#-license)

**German energy metering domain library** — Europe/Berlin calendar arithmetic,
interval types, gas conversion (incl. G 685 rounding), billing period
aggregation, SLP/RLM/iMSys classification, BDEW 2025 load profiles with
Dynamisierung, Zählzeitdefinition resolution, MsbG rollout obligations,
BSI TR-03109 SMGW lifecycle, virtual meters (§42b EnWG GGV Solarpaket I),
resampling, forecasting with confidence bounds, and Hampel-filter quality scoring.

> 🧊 **Zero I/O** · ⏱️ **no async** · 🕰️ **no clock** · 🔢 **exact decimal quantities**

`metering` is a **standalone, dependency-light domain crate**. It performs no
network or disk access, spawns no runtime, never reads the system clock, and
never represents a metered quantity as a floating-point number. You bring the
data; it does the arithmetic — deterministically, and with the regulatory basis
documented at every call site.

It computes **energy and volume**, not money: there is no currency, price or
tariff-rate type anywhere in the crate. What leaves here is kWh, m³ and kW,
which a billing layer then prices.

---

## 📦 Installation

```bash
cargo add metering
```

or in `Cargo.toml`:

```toml
[dependencies]
metering = "0.15"
```

With `serde` support for all public types:

```toml
[dependencies]
metering = { version = "0.15", features = ["serde"] }
```

**MSRV:** Rust `1.94` (edition 2024). The MSRV is pinned in
[`rust-toolchain.toml`](rust-toolchain.toml) and verified by a dedicated CI lane.

> ⚠️ **0.15 is a breaking release.** It fixes UTC-vs-Berlin bucketing, DST
> interval counts, a mislabelled OBIS constant and the ambient clock reads. See
> [CHANGELOG.md](CHANGELOG.md) for the full migration table.

---

## 🚀 Quick start

```rust
use metering::{AggregationConfig, MeterInterval, QualityFlag, aggregate};
use rust_decimal::dec;
use time::macros::datetime;

let intervals = vec![MeterInterval {
    from: datetime!(2026-06-01 0:00 UTC),
    to:   datetime!(2026-06-01 0:15 UTC),
    value_kwh: dec!(2.345),
    quality: QualityFlag::Measured,
    obis_code: Some("1-0:1.8.0*255".parse().unwrap()),
}];

let period = aggregate(&intervals, &AggregationConfig::rlm_strom());
println!("Arbeitsmenge:    {} kWh", period.arbeitsmenge_kwh);
println!("Spitzenleistung: {:?} kW", period.spitzenleistung_kw);
println!("Coverage:        {:.1} %", period.coverage_pct);
```

---

## 🗓️ Time is Europe/Berlin

The single most consequential thing this crate gets right. German metering
periods are **local calendar periods**, and a German day is not 24 hours long:

| Day | Length | Quarter-hours |
|---|---|---|
| ordinary | 24 h | 96 |
| last Sunday in March (spring forward) | **23 h** | **92** |
| last Sunday in October (fall back) | **25 h** | **100** |

```rust
use metering::{IntervalResolution, calendar};
use time::macros::{date, datetime};

// A day's real length — never assumed, always resolved.
assert_eq!(calendar::intervals_in_day(date!(2026 - 03 - 29), IntervalResolution::QuarterHour), Some(92));
assert_eq!(calendar::intervals_in_day(date!(2026 - 10 - 25), IntervalResolution::QuarterHour), Some(100));

// March 2026 holds 2 972 quarter-hours, not 31 × 96 = 2 976.
assert_eq!(calendar::intervals_in_month(date!(2026 - 03 - 01), IntervalResolution::QuarterHour), Some(2_972));

// A German day starts at 00:00 Berlin — 23:00 UTC the day before, in winter.
assert_eq!(calendar::day_start_utc(date!(2026 - 01 - 15)), datetime!(2026-01-14 23:00 UTC));
assert_eq!(calendar::day_start_utc(date!(2026 - 07 - 15)), datetime!(2026-07-14 22:00 UTC)); // CEST
assert_eq!(calendar::day_length(date!(2026 - 10 - 25)).whole_hours(), 25);
```

Two failure modes this removes from every downstream consumer:

- **Grouping on the UTC date** books the first hour (winter) or two hours
  (summer) of every German day into the previous one — wrong daily *and* monthly
  totals for every customer on every day, not just at the transitions. A §13
  StromNZV Mehr-/Mindermengensaldo computed on UTC months is wrong every month.
- **Assuming 96 intervals per day** raises a false alarm every spring and, worse,
  **masks a genuine four-interval gap every autumn** — 96 of an expected 100
  looks complete.

`resample()` buckets `Day`/`Month`/`Year` through this module, so
`expected_count` is 92 or 100 on the transition days automatically. Because a
calendar period has no fixed second count, `IntervalResolution::fixed_seconds()`
returns `None` for `Day`, `Month` and `Year` rather than an approximation.

The rules come from the IANA tz database via `time-tz`, so historical
transitions are right too — Germany's 1980 switch was on 6 April, not the last
Sunday in March.

---

## 🕰️ Determinism

**No function in this crate reads the system clock**, the filesystem or the
network. Where a timestamp is needed it is a parameter:

```rust
use metering::{MeasurementSeries, MeasurementSource};
use time::macros::datetime;

let series = MeasurementSeries::new(
    "51238696780",
    None,
    vec![],
    MeasurementSource::ManualEntry { operator_id: "ops-1".into(), reason: "correction".into() },
    datetime!(2026-01-02 09:30 UTC), // ← supplied, never sampled
);
```

A clock read is ambient state in the same family as I/O: it makes construction
non-deterministic, so two values built from identical inputs are never equal and
no storage layer can write a round-trip test. Callers that *have* a clock pass
`OffsetDateTime::now_utc()`; callers replaying an archive pass the archived
instant and get the archived value back. `just purity` (and a CI job) greps
non-comment source for clock reads, I/O and `unsafe` so the guarantee cannot
regress silently.

---

## 🗂️ Modules at a glance

| Module | Key types | Purpose |
|---|---|---|
| `interval` | `MeterInterval`, `QualityFlag`, `Sparte` | Core interval + quality types |
| `calendar` | `day_start_utc()`, `day_length()`, `intervals_in_day()`, `days_between()`, `DayKind` | Europe/Berlin calendar days, months, years — DST-correct interval counts |
| `error` | `ParseError` | The single error type every `FromStr` returns |
| `obis` | `ObisCode` | Typed OBIS codes (IEC 62056-21 / BDEW) with `default_resolution()` |
| `validation` | `validate_intervals()`, `ValidationIssue` (V01–V11) | Gap / overlap / spike (statistical + plant-capacity ceiling) / DST / rollover detection |
| `substitute` | `fill_gaps()`, `SubstituteMethod`, `SubstitutionReason` | § 60 Abs. 2 MsbG Ersatzwertbildung |
| `forecast` | `project_annual_consumption()`, `substitute_values()` | § 60 Abs. 2 MsbG Jahresprognose (with 95 % confidence bounds) + §17 Abs. 2 prior-period gap-fill |
| `resample` | `resample()`, `ResampledBucket` | Down-sample 15-min → hourly / daily / monthly |
| `virtual_meter` | `compute_virtual_meter()`, `AggregationRule` | §42b EnWG GGV: `GgvConstantAllocation` (CCI+ZG6) + `GgvProportionalAllocation` (variable); Residuallast (§42a) |
| `smgw` | `SmgwSession`, `ClsChannel`, `GatewayCertificate` | BSI TR-03109 gateway lifecycle, §14a CLS channels |
| `measurement_point` | `MeasurementPoint`, `MarktRolle`, `EnergyFlow` | MaLo + MeLo + OBIS + role binding |
| `measurement_series` | `MeasurementSeries`, `MeasurementSource`, `ProvenanceEntry` | § 60 Abs. 6 MsbG provenance / explainability |
| `register` | `MeterRegister`, `EnergyDirection`, `RegisterUnit` | HT/NT register + Wandlerfaktor metadata |
| `aggregation` | `aggregate()`, `BillingPeriod`, `HtNtSplit` | § 12 StromNZV Spitzenleistung + HT/NT |
| `classification` | `classify_messtyp()`, `detect_interval_length()`, `Messtyp` | SLP/RLM/iMSys (§3/§ 12 StromNZV, §41a EnWG) |
| `quality` | `score_intervals()`, `QualityGrade` | Hampel-filter quality scoring (A/B/C/F) |
| `demand` | `DemandWindow`, `DemandInterval` | 15-min demand / Spitzenleistung |
| `tariff_window` | `TariffWindow`, `HtNtSchedule` | DST-aware HT/NT window classification |
| `load_profile` | `LoadProfile`, `Dynamization`, `DynamicSlpProfile` | SLP classes incl. BDEW 2025 (H25/G25/L25/P25/S25); Dynamisierungsfunktion (factors 4 dp, result 3 dp) |
| `zaehlzeit` | `Zaehlzeitdefinition`, `ZaehlzeitFenster` | §14a EnWG / UTILTS time-variable register resolution, DST-correct, `split_energy()` |
| `rollout` | `classify_rollout_obligation()`, `ROLLOUT_MILESTONES` | §29 MsbG Pflichteinbaufälle (>6 000 kWh/a, §14a, >7 kW) + §45 Rollout-Fahrplan |
| `conversion` | `gas_m3_to_kwh_hs()`, `G685Rounding` | §25 Nr. 4 MessEV / DVGW G 685 incl. published NB rounding practice (z 4 dp, Hs 3 dp; final rounding configurable) |
| `imbalance` | `compute_imbalance()`, `ImbalanceSaldo` | § 13 StromNZV Mehr-/Mindermengensaldo |
| `resolution` | `IntervalResolution` | Typed interval lengths (15-min / hourly / daily …) |
| `sharing` | §42c EnWG eligibility predicates | Energy-Sharing metering eligibility — pure capability/delivery rules over `Messtyp` and `IntervalResolution` |
| `lifecycle` | `MeterExchangeEvent`, `MeterStatus` | WiM meter exchange domain events |
| `losses` | `network_losses()`, `NetworkLosses` | Netzverluste from Einspeisung vs. Entnahme |
| `power_quality` | `PowerQualityInterval` | Voltage / frequency quality samples alongside energy |

---

## 🧱 Design constraints

| Constraint | Detail |
|---|---|
| 🧊 **No I/O** | All inputs are passed as arguments. |
| ⏱️ **No async** | Synchronous throughout. |
| 🕰️ **No clock** | No `now_utc()` anywhere; timestamps are parameters. CI-enforced. |
| 🔢 **Exact quantities** | Every metered value is `rust_decimal::Decimal`. `f64` appears only in statistics and coverage — see below. |
| 🎯 **Deterministic** | Same inputs always produce the same output — replayable and cacheable. |
| 🔒 **No `unsafe`** | `#![deny(unsafe_code)]`, also checked by the purity job. |
| 🪶 **Small dependency set** | `rust_decimal`, `time`, `time-tz`, `uuid`, `thiserror` — `serde` optional. |

### 🔢 Where `f64` is and is not

The crate is not float-free, and claiming so would be false. The rule is
**which number reaches a bill**:

| Kind | Type | Examples |
|---|---|---|
| Metered quantity | `Decimal` | `value_kwh`, `arbeitsmenge_kwh`, `spitzenleistung_kw`, `ht_kwh`, gas m³→kWh_Hs, GGV allocations, substitute values |
| Statistic / diagnostic | `f64` | `coverage_pct`, Hampel `hampel_t` / `min_sigma` / MAD, `spike_factor`, forecast confidence bounds |

Every value in the first row is exact decimal arithmetic end to end — no
intermediate is ever converted to `f64` and back. The second row is
deliberately `f64`: a threshold, a percentage and a median-absolute-deviation
are comparisons and diagnostics, not amounts, and a Hampel filter in `Decimal`
would buy nothing but a `sqrt` implementation.

The one place they meet is documented at the call site: the V04 spike rule
converts a value to `f64` only to compare it against the `f64` `spike_factor`.
The comparison decides whether to *flag* an interval; it never alters one.
Forecast confidence bounds are likewise `f64`-derived and explicitly
informational — `projected_annual_kwh` itself stays exact `Decimal`.

---

## 🧩 Core types

### `MeterInterval`

A single timestamped energy reading:

```rust
pub struct MeterInterval {
    pub from: OffsetDateTime,       // UTC, inclusive
    pub to:   OffsetDateTime,       // UTC, exclusive
    pub value_kwh: Decimal,
    pub quality: QualityFlag,
    pub obis_code: Option<ObisCode>, // parsed, not a string
}
```

`obis_code` is a parsed `ObisCode`, so the same channel identifier has the same
type and value wherever it appears and `MeterInterval::obis_code ==
MeasurementSeries::obis_code` is a comparison rather than a parse that might fail
on data already accepted. Parse at the boundary — `"1-0:1.8.0*255".parse()?` —
and a malformed code is rejected there, where the message is still available to
report.

`iv.berlin_day()` gives the Europe/Berlin calendar day for grouping.

### `Sparte` and `MeasurementUnit`

```rust
pub enum Sparte          { Strom, Gas, Waerme, Wasser }
pub enum MeasurementUnit { KiloWattHour, CubicMetre }
```

A Sparte has **two** units, and conflating them is a correctness bug:

| `Sparte` | `measured_unit()` | `billing_unit()` | `requires_conversion()` |
|---|---|---|---|
| `Strom` | kWh | kWh | no |
| `Gas` | **m³** | **kWh** | **yes** |
| `Waerme` | kWh | kWh | no |
| `Wasser` | m³ | m³ | no |

A gas meter registers volume; its energy content is derived from Brennwert and
Zustandszahl (`gas_m3_to_kwh_hs`). `requires_conversion()` lets an ingest path
require those parameters before storing a value in an energy column.

`Wasser` is the one Sparte billed in a volume — water has no calorific value, so
the gas conversion does not apply to it. For the heat share of warm water see
[HeizkostenV §9 Abs. 2](#-warm-water--heat-energy-heizkostenv-9-abs-2).

### Wire units vs storage units

Storage is canonical — exactly two units reach a database, so no consumer has to
know the table below exists. The **wire** is liberal, because real devices are:

```rust
MeasurementUnit::parse("MWh")         // None — would need rescaling
MeasurementUnit::parse_scaled("MWh")  // Some(UnitScale { KiloWattHour, 1000, 1 })
```

| Accepted | Canonical | Factor |
|---|---|---|
| `kWh`, `kWh_th`, `kWh_Hs` | kWh | 1 |
| `Wh` / `MWh` / `GWh` | kWh | 1/1000 · 1000 · 10⁶ |
| `GJ` | kWh | **2500/9** |
| `MJ` | kWh | **5/18** |
| `m³`, `m3`, `cbm` | m³ | 1 |
| `l`, `ltr`, `liter` | m³ | 1/1000 |

**No German law prescribes a unit for heat meters.** MID Annex VI (MI-004) has no
units clause, and EN 1434-1 cl. 6.3.1 permits *"Joules, Watt-hours or decimal
multiples of those units"* — so a GJ meter is exactly as compliant as a kWh one.
German heat meters ship with kWh, MWh or GJ registers depending on the ordered
variant (ista sensonic 3 is sold in both kWh and GJ; Zenner multidata WR3 offers
MJ), and water submeters commonly report litres. The register unit is therefore
**device metadata, not a constant**.

UN/ECE Rec 20 codes are also accepted, and their mnemonics do not follow the unit
symbols:

| Rec 20 | Means | Trap |
|---|---|---|
| `MTQ` | cubic metre | not `M3` |
| `GV` | gigajoule | not `GJ` |
| `3B` | megajoule | `MJ` is not a Rec 20 code |
| `WHR` / `JOU` / `KJO` | watt hour / joule / kilojoule | |

Rec 20 also assigns `GJ` to gram per millilitre. This crate reads `GJ` as
gigajoule, since no Sparte modelled here carries a density; callers emitting Rec 20
codes should send `GV`.

The GJ and MJ factors are held as **exact rationals**, not decimals: 1 GJ is
277.7… kWh, so a `Decimal` factor would lose precision on every reading.
`UnitScale::apply` multiplies before dividing, making `3.6 GJ` exactly `1000 kWh`.

`parse` rejects anything needing a rescale, so a caller must go through
`parse_scaled` to obtain a factor.

### `QualityFlag` (8 variants)

| Variant | Billable | Description |
|---|---|---|
| `Measured` | ✅ | Direct meter reading |
| `Estimated` | ✅ | Prognosewert (valid for Abschlag per § 60 Abs. 2 MsbG) |
| `Substituted` | ✅ | § 60 Abs. 2 MsbG Ersatzwert |
| `Calculated` | ✅ | Derived (e.g. Residuallast) |
| `Corrected` | ✅ | Retroactive correction |
| `Preliminary` | ✅ | Vorläufiger Wert |
| `Faulty` | ❌ | Must not be billed |
| `Unknown` | ❌ | Quality not determinable |

---

## 🔥 Gas conversion: m³ → kWh_Hs

Implements the Brennwertkorrektur formula per **§25 Nr. 4 MessEV** / **DVGW G 685**:

```rust
use metering::gas_m3_to_kwh_hs;
use rust_decimal::dec;

// kWh_Hs = m³ × Hs × Z  (rounded to 6 dp)
let kwh = gas_m3_to_kwh_hs(dec!(100), dec!(10.55), dec!(0.9764));
```

`gas_m3_to_kwh_hs_rounded` applies the published Netzbetreiber rounding practice
(`G685Rounding`: z to 4 dp, Hs to 3 dp, final result configurable).

---

## 🚿 Warm water → heat energy (HeizkostenV §9 Abs. 2)

```rust
warm_water_heat_kwh(volume_m3, mean_temp_c, WarmWaterAdjustments::NONE)
warm_water_heat_kwh_unmetered(flaeche_m2, adjustments)
```

```text
Q [kWh/a] = 2.5 × V [m³] × (t_w [°C] − 10)      // Satz 2, metered volume
Q [kWh/a] = 32 × A_Wohn [m²]                    // Satz 4, floor area
```

Both are fallbacks. **§9 Abs. 2 Satz 1 requires a Wärmezähler**; Satz 2 applies
only where measurement is possible *"nur mit einem unzumutbar hohen Aufwand"*, and
Satz 4 only *"in Ausnahmefällen"* where **neither** the heat quantity **nor** the
volume can be measured.

These are *Zahlenwertgleichungen* — numerical-value equations, not dimensionally
consistent — so the constants carry no unit, and they bundle **different** things:

| Constant | Covers (§9 Abs. 2 Satz 3/5) |
|---|---|
| 2.5 | Erzeugeraufwandszahl, mittlere spezifische Wärmekapazität des Wassers, Wärmeverluste für Warmwasserspeicher, Verteilung einschließlich Zirkulation, Messdatenerhebungen |
| 32 | Nutzwärmebedarf für Warmwasser, Erzeugeraufwandszahl, Messdatenerhebungen — **no** Speicher-, Verteilungs- oder Zirkulationsverluste |

Because the Erzeugeraufwandszahl sits inside both constants, **Q is generator-input
heat, not delivered useful heat**.

`t_w` is *"die gemessene oder geschätzte mittlere Temperatur"* — an estimate is
permitted and no default or cap is prescribed. `A_Wohn` is the *"Wohn- oder
Nutzfläche"*, not living area alone.

### `WarmWaterAdjustments`

| Field | Effect | Trigger |
|---|---|---|
| `brennwert_erdgas` | × 1.11 | brennwertbezogene Abrechnung von Erdgas |
| `eigenstaendige_gewerbliche_waermelieferung` | ÷ 1.15 | **eigenständige** gewerbliche Wärmelieferung |
| `monovalente_waermepumpe` | × 0.30 | Betrieb einer monovalenten Wärmepumpe |

A struct of flags rather than an enum: §9 Abs. 2 Satz 6 applies these to the result
of *"den Zahlenwertgleichungen in Satz 2 oder 4"* and does not make them exclusive,
so a heat-pump system under eigenständige gewerbliche Wärmelieferung takes both.
`eigenständig` is a term of art (cf. §1 Abs. 1 Nr. 2); ordinary commercial heat
supply does not qualify.

A warm-water meter therefore carries **two quantities**: the m³ billed as water,
and the kWh this apportions out of the building's heating bill. The metered and
floor-area forms are separate functions rather than one `Option` parameter, because
they are different evidentiary categories.

---

## 🔍 Validation engine (V01–V11)

```rust
use metering::{ValidationConfig, validate_intervals};

let result = validate_intervals(&intervals, &ValidationConfig::default());
println!("Issues:         {}", result.issues.len());
println!("Billing blocked: {}", result.billing_block_count());
```

| Rule | ID | Detects |
|---|---|---|
| Gap detected | V01 | Missing intervals between adjacent reads |
| Overlap detected | V02 | Two intervals covering the same timestamp |
| Negative energy | V03 | `value_kwh < 0` for import registers |
| Impossible spike | V04 | `value_kwh > spike_factor × window_median` |
| Suspicious zero run | V05 | Long sequence of zero values |
| Inconsistent interval | V06 | Mixed 15-min and 60-min in same series |
| DST ambiguity | V07 | Potential local-time leak at CEST→CET fall-back |
| Future timestamp | V08 | `from > now` |
| Non-billable quality | V09 | `Faulty` or `Unknown` intervals in billing window |
| Register rollover | V10 | Counter reset without meter-exchange event |
| Unordered series | V11 | Input not ascending by `from` — usually a broken merge upstream |

`validate_intervals` is **order-independent**: the adjacency rules (V01, V02,
V05, V07, V10) are evaluated in timestamp order whatever order you supply, so a
shuffled series no longer produces a cascade of spurious gap and overlap errors.
The disorder itself is reported once as V11 (a warning, not a billing block), and
`interval_index` on every finding still points into **your** slice, not the
internal ordering.

---

## 🩹 § 60 Abs. 2 MsbG substitute value generation

```rust
use metering::{FillGapsConfig, fill_gaps, fill_gaps_with_config, project_annual_consumption};

// Automatic: linear for short gaps, carry-forward for long
let filled = fill_gaps(&intervals, 900, period_from, period_to);

// Prior-period averaging per § 60 Abs. 2 MsbG
let filled = fill_gaps_with_config(
    &intervals, 900, period_from, period_to,
    &FillGapsConfig::prior_period(prior_week_intervals),
);

// Annual forecast (Jahresprognose) — None for an empty input series
if let Some(forecast) = project_annual_consumption("MALO_ID", &intervals, None) {
    println!("Projected annual kWh: {}", forecast.projected_annual_kwh);
    // 95 % bounds are diagnostics — `None` with fewer than two observed days
    println!("95 % CI: {:?} … {:?}",
        forecast.confidence_lower_kwh, forecast.confidence_upper_kwh);
}
```

The projection scales the daily average to the **target year's real length** —
366 days in a leap year, not a flat 365, which would understate a leap-year
Jahresprognose by 0.27 %. Daily sums are grouped by Berlin calendar day,
matching how they are settled.

| `SubstituteMethod` | When to use |
|---|---|
| `LinearInterpolation` | Short gaps (≤ 3 intervals) with surrounding data |
| `PriorPeriodAverage` | Same time-slot from prior reference week (§17 Abs. 2) |
| `ZeroFill` | Documented plant shutdown — affirmative zero only |
| `LastValueCarryForward` | Conservative fallback |

---

## 📉 Resampling

```rust
use metering::{ResampleConfig, calendar, resample};

// Down-sample 15-min RLM data to Berlin calendar months (MaBiS § 13 StromNZV)
let monthly = resample(&intervals, &ResampleConfig::to_monthly());
for bucket in &monthly {
    println!("{}: {} kWh ({} of {} intervals, {:.1} %)",
        calendar::local_month(bucket.from),
        bucket.total_kwh,
        bucket.interval_count,
        bucket.expected_count,   // 2 972 for March 2026, not 2 976
        bucket.coverage_pct());
}
```

Buckets are half-open `[from, to)` UTC instants over **Berlin calendar
periods**; `expected_count` comes from the bucket's real duration, so DST is
handled without the caller knowing it happened. `ResampleConfig` carries both the
source and target resolution as typed `IntervalResolution`s.

---

## ☀️ Virtual meters (§42b EnWG GGV — Solarpaket I)

Both GGV variants compute the tenant's **net grid draw after PV allocation**,
satisfying §42b Abs. 5 EnWG sentence 2: the allocated PV energy can never exceed
the tenant's actual consumption in any 15-minute interval. This is enforced by the
`Pos()` = `max(0, x)` operator per the BDEW "Anwendungshilfe Berechnungsformeln
Solarpaket 1" (v1.0, 25.01.2024).

### Formula overview

**Constant allocation** (BDEW Beispiel 1 — UTILTS CCI+ZG6):
```text
net_grid_draw_i[t] = max(0, tenant_consumption_i[t] - fraction_i × plant_generation[t])
```

**Proportional allocation** (BDEW Beispiel 3 — variable ratio):
```text
ratio_i[t]         = tenant_consumption_i[t] / Σ all_tenant_consumption_j[t]
                     (0 if denominator = 0 — zero-division protected)
net_grid_draw_i[t] = max(0, tenant_consumption_i[t] - ratio_i[t] × plant_generation[t])
```

### Constant allocation (Beispiel 1 — UTILTS CCI+ZG6)

```rust
use metering::{AggregationRule, compute_virtual_meter};
use rust_decimal::dec;

// Tenant receives 10 % of plant generation; result = net grid draw
let rule = AggregationRule::GgvConstantAllocation {
    plant_melo_id: "MELO_PLANT".into(),
    tenant_melo_id: "MELO_T2".into(),
    fraction: dec!(0.10),
};
let net_grid_draw = compute_virtual_meter(&rule, &sources)?;
// Each interval: max(0, tenant_consumption - 0.10 × plant_generation)
// Examples:
//   gen=10, consumption=5   → max(0, 5 - 1)   = 4  (1 kWh covered by PV)
//   gen=10, consumption=0.5 → max(0, 0.5 - 1) = 0  (§42b cap: no negative draw)
```

### Proportional allocation (Beispiel 3 — variable ratio)

```rust
let rule = AggregationRule::GgvProportionalAllocation {
    plant_melo_id: "MELO_PLANT".into(),
    tenant_melo_id: "MELO_T2".into(),
    all_tenant_melo_ids: vec!["MELO_T2".into(), "MELO_T3".into()],
};
let net_grid_draw = compute_virtual_meter(&rule, &sources)?;
// ratio = T2_consumption / (T2 + T3); net = max(0, T2 - ratio × generation)
// zero-division protected: if all consumptions are 0 → net = 0
```

`sources` is a `HashMap<String, Vec<MeterInterval>>` mapping MaLo / MeLo ID to its
sorted series. Only timestamps present in **all** required series are emitted
(intersection semantics) — a gap in any source propagates rather than silently
producing a wrong total.

### Energy balance check (plant feed-in)

The residual plant feed-in (grid export from PV) equals:
```text
plant_feedin[t] = plant_generation[t] - Σ (tenant_consumption_i[t] - net_grid_draw_i[t])
               = plant_generation[t] - Σ pv_allocated_i[t]
```

Available rules: `Sum`, `Residual` (§42a EEG), `PvSelfConsumption`,
`GgvConstantAllocation`, `GgvProportionalAllocation` (§42b EnWG Solarpaket I).

---

## 🔐 BSI TR-03109 SMGW lifecycle

```rust
let session = SmgwSession { device_id, status, certificates, cls_channels, .. };

// Certificate expiry (BSI TR-03109-4 §6.3 — renew ≥ 30 days before expiry)
let expiring = session.expiring_certificates(today, 30);

// §14a CLS channel compliance
assert!(session.has_section_14a_cls());

// Communication fault detection (triggers § 60 Abs. 2 MsbG substitute)
if session.is_communication_fault(2) { // 2h threshold
    // create Sonderablesung reading order
}
```

---

## 🧮 Billing period aggregation

```rust
use metering::{AggregationConfig, aggregate};

let agg = aggregate(&intervals, &AggregationConfig::rlm_strom());
println!("kWh total:    {}", agg.arbeitsmenge_kwh);
println!("Spitzenlast:  {:?} kW", agg.spitzenleistung_kw);
if let Some(split) = &agg.ht_nt {
    println!("kWh HT:       {}", split.ht_kwh);
    println!("kWh NT:       {}", split.nt_kwh);
}
```

### `AggregationConfig` presets

| Preset | Messtyp | Detail |
|---|---|---|
| `rlm_strom()` | RLM | Spitzenleistung § 12 StromNZV, no tariff split |
| `slp_strom()` | SLP | Arbeitsmenge only |
| `rlm_zweitarif()` | RLM | Spitzenleistung + HT/NT split |
| `gas()` | Gas | Single total (m³ → kWh_Hs, no tariff split) |

The HT window is a `TariffWindow`, so **hour and weekday are both read in
Europe/Berlin local time** — `.with_ht_window(TariffWindow::MON_SAT_0600_2200)`
swaps it. Reading the hour locally but the weekday in UTC (as an earlier version
did) misclassifies the hours either side of local midnight at the week
boundaries. For the general §14a EnWG case use `zaehlzeit::Zaehlzeitdefinition`.

---

## 🏷️ SLP/RLM/iMSys classification

```rust
pub fn classify_messtyp(
    malo: &MaloId,
    jahresverbrauch_kwh: Decimal,
    sparte: Sparte,
) -> Messtyp
```

| `Messtyp` | Criteria | Basis |
|---|---|---|
| `Slp` | < 100 MWh/a (Strom) or < 1.500 MWh/a (Gas) | § 2 MsbG |
| `Rlm` | ≥ 100 MWh/a (Strom) or ≥ 1.500 MWh/a (Gas) | § 12 StromNZV |
| `Imsys` | Pflichteinbau iMSys (§41a EnWG) | §41a EnWG |

---

## 🔢 OBIS medium (value group A)

`ObisCode::is_heat`, `is_water` and `is_heat_cost_allocator` follow the
DLMS/COSEM Blue Book media list that OMS Spec Vol. 2 adopts:

| A | Medium | Predicate | Constant |
|---|---|---|---|
| 1 | Electricity | `is_electricity()` | `STROM_BEZUG_TOTAL`, … |
| 4 | Heizkostenverteiler | `is_heat_cost_allocator()` | — |
| 5 | Cooling | `is_heat()` | `KAELTE_ENERGY` |
| 6 | Heat | `is_heat()` | `WAERME_ENERGY` |
| 7 | Gas | `is_gas()` | `GAS_VOLUME_M3` |
| 8 | Cold water | `is_water()` | `WASSER_KALT_VOLUME` |
| 9 | Hot water | `is_water()` | `WASSER_WARM_VOLUME` |

**A = 8 is water, not heat.** An HCA (A = 4) reports dimensionless
*Verbrauchseinheiten* and carries no Eichfrist — HeizkostenV §5 Abs. 1 Satz 3
admits it as an apportionment device precisely because it measures no unit.

A test asserts every named constant satisfies the predicate its name implies —
`WAERME_ENERGY` was `8-0:1.0.0` through 0.14, so a heat register reported
`is_water()` and inherited water's daily default resolution instead of hourly.

---

## ⚖️ Mehr-/Mindermengensaldo

```rust
pub fn compute_imbalance(
    actual_kwh: Decimal,
    contracted_kwh: Decimal,
) -> ImbalanceSaldo
```

`ImbalanceSaldo` carries `mehr_kwh`, `minder_kwh` and the signed
`delta_kwh = actual − contracted`, plus `is_mehr()` / `is_minder()`.

Consuming **above** the profile is a **Minder**menge — the NB supplied the
shortfall and invoices it (LF owes NB). Consuming **below** it is a
**Mehr**menge, which the NB vergütet (NB owes LF).
Basis: **§ 13 StromNZV**.

---

## 📊 Hampel-filter quality scoring

`score_intervals` runs a Hampel filter over a time-ordered slice of intervals
and returns a `QualityReport` (gaps, outliers, zero runs, coverage, grade):

```rust
use metering::{QualityConfig, Sparte, score_intervals};

let report = score_intervals(&intervals, QualityConfig::for_sparte(Sparte::Strom));
println!("Grade:    {:?}", report.grade);
println!("Coverage: {:.1} %", report.coverage_pct);
println!("Outliers: {}", report.outlier_intervals.len());
```

### Media-aware thresholds

| `Sparte` | `max_zero_run_allowed` | `min_sigma` |
|---|---|---|
| `Strom` | 2 | 0.0 |
| `Gas` | 48 | 0.01 |
| `Waerme` | 720 | 0.05 |
| `Wasser` | 720 | 0.001 |

`min_sigma` guards **MAD implosion**: across a flat window the median absolute
deviation is 0, so `t × sigma` is 0 and every nonzero value scores as an outlier.
`hampel_filter_with_floor` exposes the same guard as a primitive.

### `QualityGrade`

| Grade | Meaning | Suggested downstream effect |
|---|---|---|
| `A` | Clean — within threshold | Billing proceeds normally |
| `B` | Slightly suspicious (≤ 1 gap/outlier, coverage ≥ 99 %) | Logged; billing proceeds |
| `C` | Likely outlier (≤ 3 gaps/outliers, coverage ≥ 95 %) | Manual review / quality warning event |
| `F` | Severe outlier / substituted | `blocks_billing()` → operator alert |

Low-level primitives are exported too: `hampel_filter` (raw outlier indices),
`hampel_filter_with_floor`, and `score_intervals_f64` / `score_intervals_raw`
for callers that already hold `f64` samples (e.g. straight out of a database).

---

## 🔤 String forms

`Sparte`, `MeasurementUnit`, `QualityFlag` and `QualityGrade` each have **one**
stable code that `as_str()`, `Display`, `FromStr` and the `serde` tag all agree
on, so a value written to a log, a CLI argument or a database column reads back
as itself:

```rust
use metering::{QualityFlag, Sparte, IntervalResolution};

assert_eq!(QualityFlag::Substituted.as_str(), "SUBSTITUTED");
assert_eq!("substituted".parse::<QualityFlag>().unwrap(), QualityFlag::Substituted);

// IntervalResolution uses ISO 8601 durations, so Custom(n) round-trips too.
assert_eq!(IntervalResolution::QuarterHour.to_string(), "PT15M");
assert_eq!("PT900S".parse::<IntervalResolution>().unwrap(), IntervalResolution::QuarterHour);
```

A test walks `Sparte::ALL`, `QualityFlag::ALL`, `MeasurementUnit::ALL` and
`LoadProfile::ALL` asserting `from_str(x.as_str()) == x` and that all codes are
distinct, so a new variant cannot ship with a half-wired mapping. Unrecognised
input is a parse **error**, never a silent `Unknown` — the latter is a statement
about the measurement, the former about the message.

Every `FromStr` in the crate returns the same `ParseError`, so a decoder needs
one `?` chain rather than three `map_err` calls:

```rust
use metering::{ObisCode, ParseError, QualityFlag, Sparte};

fn decode(sparte: &str, quality: &str, obis: &str)
    -> Result<(Sparte, QualityFlag, ObisCode), ParseError>
{
    Ok((sparte.parse()?, quality.parse()?, obis.parse()?))
}

let err = decode("STROM", "MEASURED", "nope").unwrap_err();
assert_eq!(err.type_name(), "ObisCode"); // which column was bad
```

---

## 🔓 Enum exhaustiveness

**Domain enums are exhaustive; only error enums are `#[non_exhaustive]`.** A
deliberate choice, and the opposite of the usual library default.

`#[non_exhaustive]` buys the *library* freedom to add variants without a major
version and charges the *consumer* a wildcard arm. That wildcard is where the
cost lands: when a new `Messtyp`, `SubstituteMethod` or `QualityFlag` appears, a
consumer mapping this vocabulary onto their own storage codes wants their build
to break so a human decides what it means. With a wildcard they get a silent
fallback instead — a reading filed under the wrong Messtyp, a substitute value
attributed to the wrong method. For a crate whose output ends up on an invoice,
a compile error at upgrade time is much the cheaper failure.

Error enums are the opposite case, so `VirtualMeterError` *is* `#[non_exhaustive]`:
a consumer that wildcards an unfamiliar error still reports a failure, which is
correct.

The consequence, stated plainly: **adding a variant to a domain enum is a
breaking change here** and will be released as one. Exhaustive `match` and
iteration over `ALL` are supported patterns.

---

## 🔒 Serde representation stability

With `serde` enabled, the emitted representation — enum tags and struct field
names — is **part of the public API and covered by semver**. Persisting these
values means relying on a wire format, so a rename is a breaking change and will
be released as one.

[`tests/serde_representation.rs`](tests/serde_representation.rs) pins every tag
literally, so the commitment is mechanical rather than a promise:

```rust
assert_eq!(to_string(&QualityFlag::Measured)?,  "\"MEASURED\"");
assert_eq!(to_string(&Sparte::Waerme)?,         "\"WAERME\"");
assert_eq!(to_string(&ObisCode::WAERME_ENERGY)?, "\"6-0:1.0.0*255\"");
```

Adding a **new** variant is not breaking for writers and may ship in a minor
release; it is breaking for older readers — the usual open-enum trade-off. If you
would rather not couple your storage to this crate at all, define your own codes
with an exhaustive `match`, so a new upstream variant breaks your build rather
than your data.

---

## 🎛️ Feature flags

| Flag | Default | Effect |
|---|---|---|
| `serde` | ❌ | Derive `Serialize`/`Deserialize` on all public types (also enables `rust_decimal/serde`) |

---

## 🧪 Testing

```bash
cargo test --all-features
```

The suite covers: gas conversion (DVGW G 685, incl. the published NB rounding
examples), aggregation (RLM/SLP/Gas), Messtyp classification, imbalance
arithmetic, the V01–V11 validation engine (DST transitions, V07 ambiguity,
plant-capacity ceiling, order-independence), § 60 Abs. 2 MsbG substitute methods and forecast confidence
bounds, resampling, §42b EnWG GGV virtual meters (Beispiel 1 constant +
Beispiel 3 proportional, Pos() cap, zero-division guard), §42a Residuallast,
BSI TR-03109 SMGW + CLS lifecycle, Zählzeitdefinition resolution, MsbG §29/§45
rollout classification, BDEW 2025 profile Dynamisierung, measurement series
provenance, register + ObisCode — plus property tests and three integration
suites:

| Suite | Covers |
|---|---|
| [`berlin_calendar.rs`](tests/berlin_calendar.rs) | Every DST transition 2025–2027, day tiling across a full year, 92/100-interval completeness |
| [`serde_representation.rs`](tests/serde_representation.rs) | Every enum tag and struct field name, pinned literally |
| [`regulatory_showcase.rs`](tests/regulatory_showcase.rs) | End-to-end regulatory scenarios with the exact 2026 DST dates |
| [`readme_samples.rs`](tests/readme_samples.rs) | Every snippet in this file, so the README cannot drift from the code |

### 🛠️ Dev tasks

A [`justfile`](justfile) wraps the common commands ([just](https://just.systems)):

```bash
just            # list all recipes
just ci         # fmt-check + lint + purity + test + doc + package
just test       # cargo test --all-features
just lint       # clippy with -D warnings, default + all features
just purity     # assert no clock read, no I/O, no unsafe in src/
just msrv       # compile on the pinned MSRV
just doc-open   # build the docs and open them
```

Or plain cargo:

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
cargo doc --no-deps --all-features
```

---

## 📚 Regulatory basis

- **§3, § 12 StromNZV** — SLP/RLM classification thresholds
- **§ 12 StromNZV** — Spitzenleistung definition for RLM
- **§ 60 Abs. 2 MsbG** — Ersatzwertbildung + Jahresprognose (substitute values + annual forecast)
- **§ 60 Abs. 6 MsbG** — 3-year provenance retention (`MeasurementSeries`, `ProvenanceEntry`)
- **§ 13 StromNZV** — Mehr-/Mindermengensaldo
- **§25 Nr. 4 MessEV / DVGW G 685** — Gas Brennwertkorrektur
- **§41a EnWG** — 15-Minuten-Lastgang and iMSys Pflichteinbau
- **§42a/§42b EEG** — Residuallast / GGV community solar virtual meters
- **§42c EnWG** — Energy-Sharing metering eligibility
- **§14a EnWG** — Steuerbare Verbrauchseinrichtungen (CLS channels, Zählzeitdefinition)
- **§29 / §45 MsbG** — iMSys Pflichteinbaufälle and the Rollout-Fahrplan quotas
- **HeizkostenV §9 Abs. 2** — Warmwasser-Wärmemenge Zahlenwertgleichungen
- **BDEW Anwendungshilfe SLP Strom (17.03.2025)** — H25/G25/L25/P25/S25 profiles and Dynamisierung rounding
- **BDEW Anwendungshilfe Berechnungsformeln Solarpaket 1 (v1.0, 25.01.2024)** — GGV allocation examples
- **BSI TR-03109** — Smart Meter Gateway lifecycle and certificates

> ℹ️ This crate implements arithmetic and classification rules derived from the
> sources above. It is **not legal advice** and carries no warranty of regulatory
> compliance — verify against the current Gesetzestext for your use case.

---

## 🤝 Contributing

Issues and pull requests are welcome at
[github.com/hupe1980/metering](https://github.com/hupe1980/metering).

Please make sure `just ci` passes before opening a PR. New regulatory rules should
cite their **Rechtsgrundlage** in a doc comment and come with a test derived from a
published example wherever one exists.

---

## 📄 License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in this crate by you, as defined in the Apache-2.0 license, shall be
dual licensed as above, without any additional terms or conditions.
