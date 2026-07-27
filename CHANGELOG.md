# Changelog

All notable changes to `metering` are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); the crate follows
semver, with the `serde` representation explicitly in scope (see the crate docs).

## [0.16.0] — 2026-07-27

One theme: **a value must have exactly one string.** Two spellings of one value
is not a cosmetic problem — it produces two database keys, two map entries and
two "distinct" rows that mean the same thing, with no error raised anywhere.
Reported from production, where a correction failed to supersede the reading it
corrected and the billed total was overstated by the correction amount.

A second theme emerged while verifying the first against the authoritative
source — the EDI@Energy *Codeliste der OBIS-Kennzahlen und Medien* v2.4b.
Several `ObisCode` predicates were written from the IEC value-group *names*
rather than from how the German market *assigns* them, and the test suite
never exercised the difference.

### Fixed — OBIS value-group semantics

- **Direction was read from the wrong value group, so the Lastgang was neither
  import nor export.** `is_import()` required `C = 1 && D = 8`, but EDI@Energy
  §2.1 defines direction by C alone — "+ Bezug des Kunden aus dem Netz (z. B.
  `1-b:1.x.y`)", with `x`/`y` explicitly free. `1-0:1.29.0` — the Lastgang,
  the code MSCONS PID 13018 carries and the one a `MeterInterval` normally
  holds — therefore reported `is_import() == false`, and so did
  `MeterInterval::is_import_energy()`, `MeasurementPoint::is_bezug()` and
  `MeterRegister::is_import()`. Direction is now `A = 1 && C ∈ {1, 2}`, across
  every Messart.

- **`D = 9` was modelled as a reverse-direction flag.** In the German market
  D is a *Messart*: 6 = Maximum, 8 = Zählerstand (Zeitintegral 1), 9 = Vorschub
  (Zeitintegral 2), 29 = Lastgang (Zeitintegral 5). The constants
  `STROM_REACTIVE_INDUCTIVE_EXPORT` / `..._CAPACITIVE_EXPORT` (`1-0:3.9.0`,
  `1-0:4.9.0`) were labelled "export direction" but denote *Blindarbeit
  positiv/negativ, Vorschub* — a different quantity, not a direction.

- **`is_demand()` claimed `D = 29` is maximum demand.** D = 29 is the Lastgang,
  an energy quantity in kWh per interval; the maximum is D = 6, a power in kW.
  Conflating them bills a 15-minute energy quantity as a Leistungspreis basis.
  Replaced by `is_lastgang()`, `is_maximum()`, `is_zaehlerstand()` and
  `is_vorschub()`, which name what each Messart is.

- **`is_reactive()` missed the four quadrant registers and claimed gas volume.**
  It tested `C ∈ {3, 4}` with no medium guard. Blindleistung is C = 3…8 —
  positiv, negativ and Q I…Q IV — so a quadrant register was read as active
  energy, putting kvarh into a kWh column. Without the medium guard,
  `GAS_VOLUME_M3` (`7-0:3.0.0`, C = 3) reported `is_reactive() == true`.
  Now `A = 1 && C ∈ 3..=8`.

- **`tariff_register()` reported the Fehlerregister as tariff 63.** EDI@Energy
  §2.2 lists "63 Fehlerregister" alongside the tariffs; it counts faults and is
  not a billable quantity. It now returns `None`, with `is_fehlerregister()` to
  distinguish it from the total register, and `default_resolution()` returns
  `None` for it rather than a 15-minute series.

- **Two power-quality OBIS codes were documented as L1 when they are the
  all-phase average.** Per-phase channels are 3x (L1), 5x (L2), 7x (L3); the 1x
  codes are the average across phases. `power_quality.rs` listed `1-0:12.7.0`
  as "Voltage L1" (it is the average — a different number on an unbalanced
  three-phase load) and `1-0:11.7.0` as "Current L1". The correct codes are
  `1-0:32.7.0` and `1-0:31.7.0`; the table now lists per-phase and average rows
  separately, and the missing L2/L3 current rows were added.

### Changed — OBIS constants renamed to what they denote

| Removed | Replacement | Why |
|---|---|---|
| `STROM_DEMAND_INTERVAL` | `STROM_BEZUG_LASTGANG` | `1-0:1.29.0` is a Lastgang in kWh, not a demand in kW |
| — | `STROM_BEZUG_MAXIMUM` | `1-0:1.6.0`, the actual Spitzenleistung register |
| — | `STROM_BEZUG_VORSCHUB` | `1-0:1.9.0`, Zeitintegral 2 |
| — | `STROM_EINSPEISUNG_LASTGANG` | `1-0:2.29.0` |
| `STROM_REACTIVE_INDUCTIVE` | `STROM_BLINDARBEIT_POSITIV` | C = 3 is "Blindleistung positiv"; inductive/capacitive is a quadrant property |
| `STROM_REACTIVE_CAPACITIVE` | `STROM_BLINDARBEIT_NEGATIV` | C = 4 is "Blindleistung negativ" |
| `STROM_REACTIVE_INDUCTIVE_EXPORT` | `STROM_BLINDARBEIT_Q1`…`Q4` | the old pair were Vorschub registers mislabelled as export |
| `STROM_REACTIVE_CAPACITIVE_EXPORT` | (as above) | |

`ObisCode::TARIFF_FEHLERREGISTER` (63) was added alongside `STORAGE_UNUSED`.

### Fixed

- **`ObisCode` was not string-stable: `FromStr` → `Display` was not the
  identity.** Parsing defaulted the storage group F to 255 and `Display` always
  printed it, so `"1-0:1.8.0"` — the spelling MSCONS carries and people type —
  came back out as `"1-0:1.8.0*255"`. A reading written through one path and a
  correction written through another produced different keys for one channel,
  so the correction superseded nothing and both rows survived resolution.

  `Display` now writes the **reduced form**, omitting `*F` when F is 255 ("not
  applicable", IEC 62056-6-1 Annex A). A storage group that carries information
  is never elided: `1-0:1.8.0*1` keeps its suffix and stays a distinct code.

- **`ObisCode` could not be deserialised by any non-borrowing deserialiser.**
  `serde(try_from = "&str")` required the deserialiser to hand out a borrowed
  `&str`, so `serde_json::from_reader`, bincode, postcard and MessagePack all
  failed with `invalid type: string "1-0:1.8.0*255", expected a borrowed string`
  regardless of what the payload said — streaming a file of intervals was simply
  impossible. Replaced with a visitor-based `Deserialize`, and a `Serialize` that
  writes through `collect_str` without an intermediate `String`.

- **The OBIS parser accepted signed value groups.** `u8::from_str` accepts a
  leading `+`, so `+1-0:1.8.0*+255` parsed and then rendered under the canonical
  spelling — a second spelling entering through the front door. Value groups are
  now ASCII digits or nothing.

### Changed

- **BREAKING — `ObisCode`'s string and `serde` form is now `"1-0:1.8.0"`**, not
  `"1-0:1.8.0*255"`. Both spellings still *parse*, so archives written under
  either form still read; only what the crate *writes* changed. See "Migrating
  stored OBIS codes" below.

- **BREAKING — `IntervalResolution`'s `serde` form is now its ISO 8601 duration**
  — `"PT15M"`, `"P1D"`, `"PT300S"` — instead of the derived Rust variant names
  `"QuarterHour"`, `"Day"`, `{"Custom":300}`. This is the same defect one type
  over: the value had a canonical string *and* a parallel serde encoding, and
  the serde one was a variant rename away from silently invalidating stored
  data. ISO 8601 is an external standard, so the new form cannot be renamed by
  any refactor here. `Display` / `FromStr` are unchanged.

- `ObisCode` now derives `PartialOrd` + `Ord`, ordering lexicographically by
  value group (A, then B, … then F) — it is routinely used as a sort and merge
  key.

- The OBIS parser now tolerates surrounding whitespace and leading zeros, so
  `"  01-00:01.08.00 "` from an EDIFACT segment normalises onto `"1-0:1.8.0"`.

### Added

- **`ObisCode::normalize(&str) -> Result<String, ParseError>`** — canonicalise a
  raw string without building a value first, for consumers holding a database
  column, a CSV cell or an MSCONS segment. Idempotent.
- **`ObisCode::to_full_string()`** and the `{:#}` alternate format — the explicit
  six-group form `1-0:1.8.0*255`, for systems that demand it. Parses back to the
  same value.
- `ObisCode::STORAGE_UNUSED` (255) and `ObisCode::has_unused_storage()`.
- `ObisCode::MAX_LEN` (23) — the longest string a code can render as, for sizing
  a fixed-width column. The canonical form is at most 19.
- `ObisCode`'s `Display` now honours width, fill and alignment (`{:>13}`). It
  wrote straight to the formatter before, so padding was silently dropped and
  codes would not line up in a table. Rendering stays allocation-free.
- Gas constants `GAS_NORMVOLUMEN_UMGEWERTET` (`7-0:13.2.0`), `GAS_ZUSTANDSZAHL`
  (`7-0:52.0.22`) and `GAS_BRENNWERT_MONATSMITTEL` (`7-0:54.0.22`), plus a
  warning on `GAS_VOLUME_M3` and in `conversion`: `gas_m3_to_kwh_hs` expects a
  **Betriebsvolumen**. Passing an already-converted Normvolumen applies the
  Zustandszahl twice and overstates the energy by a few percent, silently.
- `tests/string_canonicalisation.rs` — stability, totality, idempotence and
  injectivity under `proptest`, for the string form and the `serde` form, over
  both `ObisCode` and `IntervalResolution`.
- An `obis::mako_semantics_tests` module pinning every value-group rule above
  against the EDI@Energy Codeliste, and README/rustdoc sections stating the
  three rules that are read wrong most often: direction is C alone, D is a
  Messart, E = 63 is the Fehlerregister.

### Migrating stored OBIS codes

Only rows written by this crate need rewriting, and only if you store the string
rather than the six groups. The change is a suffix strip:

```sql
UPDATE readings
   SET obis_code = left(obis_code, length(obis_code) - 4)
 WHERE obis_code LIKE '%*255';
```

Run it over every table that keys on the code, then rebuild any index or
materialised view derived from it. If you enforce the canonical shape with a
check constraint, update it to reject a `*255` suffix rather than require one.
Rows that already hold the short spelling are already canonical, and rows with a
real storage group (`*0`, `*1`, …) must be left alone — the `WHERE` clause above
does that.

For `IntervalResolution`, map the old tags with
`'QuarterHour' → 'PT15M'`, `'HalfHour' → 'PT30M'`, `'Hour' → 'PT1H'`,
`'Day' → 'P1D'`, `'Month' → 'P1M'`, `'Year' → 'P1Y'`, and `{"Custom":n}` →
`'PT{n}S'`.

## [0.15.0] — 2026-07-27

A deliberate hard cut. Several fixes could not be made compatibly without
leaving the wrong behaviour reachable, so they were made breaking instead.
Every change below is mechanical to apply; the table at the end maps each old
call to its replacement.

### Added

- **`calendar` module** — Europe/Berlin calendar arithmetic, the piece every
  consumer was otherwise reimplementing: `local_day`, `local_month`,
  `local_year`, `day_start_utc`, `day_end_utc`, `month_start_utc`,
  `month_end_utc`, `year_start_utc`, `year_end_utc`, `day_length`,
  `month_length`, `year_length`, `day_kind` (`DayKind::{Normal, ShortDay,
  LongDay}`), `intervals_in_day`, `intervals_in_month`, `intervals_in_year`,
  `intervals_between`, `days_between`, `shift_back_one_year`, `days_in_month`,
  `days_in_year`, `is_leap_year`, `to_berlin`, `berlin`. Rules come from the IANA
  tz database, so historical transitions are correct.
- **`error::ParseError`** — one error type for every `FromStr` in the crate,
  replacing `ObisParseError`, `IntervalResolutionParseError`, `ParseCodeError`
  and `LoadProfile`'s `type Err = String`. Opaque, with `type_name()`,
  `input()` and `expected_values()` accessors, so added context is never
  breaking. A decoder can now parse several types in one `?` chain.
- `LoadProfile::{ALL, CODES}`.
- `MeterInterval::berlin_day()` — the correct grouping key for daily aggregation.
- `Display` + `FromStr` for `Sparte`, `MeasurementUnit`, `QualityFlag` and
  `IntervalResolution`, plus `QualityFlag::as_str` (previously absent) and
  `ALL` / `CODES` constants. `IntervalResolution` uses ISO 8601 durations
  (`PT15M`, `P1D`, `PT300S`), so `Custom(n)` round-trips.
- `IntervalResolution::{fixed_seconds, nominal_seconds, is_fixed, is_calendar, to_iso8601}`.
- `AggregationConfig::{with_ht_window, default}`, `ResampleConfig::{new, default}`,
  `MeasurementSeries::{with_melo_id, with_resolution, worst_quality,
  has_unbillable_intervals, record_event_with_note}`.
- `ObisCode::{KAELTE_ENERGY, WASSER_KALT_VOLUME, WASSER_WARM_VOLUME}`.
- **V11 `ValidationRuleId::UnorderedSeries`** — `validate_intervals` is now
  order-independent (see Fixed) and reports an out-of-order input as its own
  warning-level finding.
- `tests/berlin_calendar.rs` and `tests/serde_representation.rs`; a `purity`
  check (justfile recipe + CI job) asserting no clock read, I/O or `unsafe`
  reaches non-comment source.

### Fixed

- **`resample()` bucketed `Day`/`Month`/`Year` on UTC boundaries.** A German
  settlement period starts at 00:00 Europe/Berlin — 23:00 UTC the previous day in
  winter — so the first hour of every day and every month was booked into the
  preceding period. This made every §13 StromNZV monthly Mehr-/Mindermengensaldo
  wrong, not only at the DST transitions. Buckets are now Berlin calendar periods.
- **`expected_count` assumed a fixed day length.** It now follows the bucket's
  real duration: 92 quarter-hours on the spring-forward day, 100 on the fall-back
  day. Previously a complete autumn day of 100 intervals was indistinguishable
  from 96, so a genuine four-interval gap was masked every October.
- **`IntervalResolution::as_seconds()` returned wrong values** for `Month`
  (30 days), `Year` (365 days) and `Day` (24 h, wrong twice a year). Replaced by
  `fixed_seconds() -> Option<u32>`, which returns `None` for all three rather
  than guessing, and `nominal_seconds()` for sizing only.
- **`HtHours::is_ht` mixed timezones** — the hour was read in Europe/Berlin but
  the weekday in UTC, so the hours either side of local midnight were classified
  against the wrong day at week boundaries. `AggregationConfig` now holds a
  `TariffWindow`, which reads both locally.
- **`ObisCode::WAERME_ENERGY` was `8-0:1.0.0` — medium 8 is cold water, not
  heat.** A heat register therefore reported `is_water() == true`,
  `is_heat() == false`, and inherited water's daily default resolution instead of
  hourly. It is now `6-0:1.0.0`, and a test asserts every named constant
  satisfies the predicate its name implies.
- **The annual projection always scaled to 365 days**, understating a leap-year
  Jahresprognose by one day (0.27 %). It now uses the target year's real length,
  and groups daily sums by Berlin calendar day.
- **`observed_days` was computed as `(last_to - first_from).whole_days()`**,
  which truncates: fourteen calendar days spanning the spring transition are 335
  hours, so integer division reported thirteen. The daily average — and the
  projection built on it — came out **7.7 % too high** for any window containing
  a spring transition. Now counted in Berlin calendar days via
  `calendar::days_between`. The same truncation affected the seasonal factor's
  `period_days`.
- **`compute_seasonal_factor` shifted the comparison window back by a fixed 365
  days**, which drifts by a day across a leap year and lands an hour off across a
  DST transition — so "the same two weeks of March" silently compared different
  windows. It now shifts by a calendar year (`calendar::shift_back_one_year`,
  29 February → 28 February), and divides the prior year's total by that year's
  real length rather than 365.
- **`IntervalLengthClass::seconds()` carried the same approximation bug as the
  old `as_seconds()`** — `Daily => 86_400`, `Monthly => 86_400 * 30`. The type
  was a near-duplicate of `IntervalResolution`, so it was removed rather than
  patched; the bug is gone by construction.
- **`validate_intervals` silently required pre-sorted input** — the one function
  most likely to be handed messy data. Unsorted input produced a cascade of
  spurious V01 gap and V02 overlap errors, which are billing-blocking, on data
  that was merely shuffled. The adjacency rules now evaluate in timestamp order
  regardless of input order, `interval_index` still refers to the caller's slice,
  and the disorder itself is reported once as V11.
- **`LoadProfile::Custom` did not round-trip**: `as_str()` emitted `"CUSTOM"` but
  `parse("CUSTOM")` returned `None`, so a stored `Custom` profile failed to read
  back. Caught by the new `ALL`/`CODES` consistency test.
- **`MeasurementSeries::new`, `record_event` and the SMGW fault checks read the
  system clock**, making construction non-deterministic — two series built from
  identical inputs never compared equal, so no storage layer could write a
  round-trip test. All timestamps are now parameters; the crate contains no clock
  read at all, enforced by CI.
- `MeasurementUnit`'s `serde` tag (`KILO_WATT_HOUR`) disagreed with its
  `as_str()` code (`KWH`). There is now exactly one string per value.

### Changed (breaking)

| 0.14 | 0.15 |
|---|---|
| `aggregate(&ivs, config)` | `aggregate(&ivs, &config)` |
| `AggregationConfig { ht_hours: HtHours { .. } }` | `AggregationConfig { ht_window: TariffWindow { .. } }` |
| `MeterInterval { obis_code: Some("1-0:1.8.0".into()) }` | `MeterInterval { obis_code: Some("1-0:1.8.0".parse()?) }` |
| `iv.parsed_obis_code()` | `iv.obis_code` |
| `res.as_seconds()` | `res.fixed_seconds()` (`Option`) / `calendar::day_length` |
| `IntervalResolution::from_seconds(86_400)` → `Day` | → `Custom(86_400)`; a calendar day is not 86 400 s |
| `ResampleConfig { source_interval_seconds: 900 }` | `ResampleConfig { source_resolution: IntervalResolution::QuarterHour }` |
| `MeasurementSeries::new(malo, obis, ivs, src)` | `MeasurementSeries::new(malo, obis, ivs, src, ingested_at)` |
| `series.worst_quality` (field) | `series.worst_quality()` (method) |
| `series.recompute_quality()` | removed — the value is always derived |
| `series.record_event(kind, actor)` | `series.record_event(kind, actor, occurred_at)` |
| `gw.hours_since_last_contact()` | `gw.hours_since_last_contact(now)` |
| `gw.is_communication_fault(hours)` | `gw.is_communication_fault(now, hours)` |
| `IntervalResolution`'s `Display` (`"900s (15-Minuten)"`) | ISO 8601 (`"PT15M"`); `label()` still gives the German name |
| `MeasurementUnit` serde tag `KILO_WATT_HOUR` / `CUBIC_METRE` | `KWH` / `M3` |
| `ObisCode::WAERME_ENERGY` = `8-0:1.0.0*255` | `6-0:1.0.0*255` |
| `IntervalLengthClass` | removed — use `IntervalResolution` |
| `detect_interval_length() -> Option<IntervalLengthClass>` | `-> Option<IntervalResolution>` |
| `DeliveryEvidenceInput::interval_class` | `::resolution`, typed `Option<IntervalResolution>` |
| `ObisParseError`, `IntervalResolutionParseError`, `ParseCodeError` | all replaced by `ParseError` |
| `ObisParseError(pub String)` field access | `err.input()` |
| `ParseCodeError { type_name, input, expected }` fields | `err.type_name()`, `err.input()`, `err.expected_values()` |
| `<LoadProfile as FromStr>::Err = String` | `= ParseError` |
| `ValidationRuleId` | gained `UnorderedSeries` (V11) — breaking for exhaustive matches |

Callers that previously grouped by `iv.from.date()` should move to
`iv.berlin_day()`; those that assumed 96 intervals per day should call
`calendar::intervals_in_day(day, IntervalResolution::QuarterHour)`.

`MeasurementSeries` gained `PartialEq`/`Eq` (as did `ProvenanceEntry`), so
storage round-trip tests can assert equality directly. No `Default` impl was
added: `MeasurementSource` has no meaningful default, and inventing one would put
a fabricated provenance record into every test fixture. Use `new(...)` — now
total and deterministic — with the `with_*` builders.

### Enum exhaustiveness policy

Domain enums stay **exhaustive**; only error enums are `#[non_exhaustive]`
(currently `VirtualMeterError`). The reasoning is in the crate docs and the
README: a consumer mapping this crate's vocabulary onto their own storage codes
should get a build break when a new `Messtyp` or `SubstituteMethod` appears, not
a silent wildcard that files the reading under the wrong one.

The trade-off, stated so it is not a surprise: **adding a variant to a domain
enum is a breaking change here** and will be released as one.

## [0.14.0]

Initial published version.
