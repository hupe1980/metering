# Changelog

All notable changes to `metering` are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); the crate follows
semver, with the `serde` representation explicitly in scope (see the crate docs).

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
