+++
title = "Design constraints"
description = "Determinism, exact decimals, one canonical string per value, serde stability and exhaustive domain enums — the invariants the library holds to."
weight = 12
+++

## Determinism

**No function reads the system clock, the filesystem or the network.** Every
input is an argument, so equal inputs give equal outputs and any result can be
replayed or cached. Where an instant is needed, it is a parameter.

A clock read is ambient state in the same family as I/O: it makes construction
non-deterministic, so two values built from identical inputs are never equal and
no storage layer can write a round-trip test. Callers holding a clock pass
`OffsetDateTime::now_utc()`; callers replaying an archive pass the archived
instant and get the archived value back. CI enforces this with a grep over
non-test code.

## Where `f64` is and is not

| Kind | Type | Examples |
|---|---|---|
| Metered quantity | `Decimal` | `value`, `arbeitsmenge`, `spitzenleistung_kw`, register splits, gas conversion, GGV allocations, substitute values |
| Statistic / diagnostic | `f64` | `coverage_pct`, the Hampel MAD and threshold, the forecast prediction interval, EN 50160 shares |

The two meet in exactly one place: the V04 outlier rule converts values to `f64`
to run the Hampel filter. That comparison decides whether to *flag* an interval;
it never alters one. Nothing a float touches is written back into a quantity.

## One value, one string

Every type with a string form has exactly one, because the alternative fails
silently: a value with two spellings produces two database keys and two
"distinct" rows that mean the same thing, with no error anywhere.

```rust
use metering::ObisCode;

// Whichever spelling arrived, one key comes out.
assert_eq!(ObisCode::normalize("1-0:1.8.0*255")?, "1-0:1.8.0");
assert_eq!(ObisCode::normalize("  1-0:01.8.0 ")?, "1-0:1.8.0");
// ...but a real storage group is information, never elided.
assert_eq!(ObisCode::normalize("1-0:1.8.0*1")?,   "1-0:1.8.0*1");
# Ok::<(), metering::ParseError>(())
```

Parsing is lenient where writing is not. `s.parse()?.to_string() == s` holds for
every canonical `s`, and a proptest suite holds stability, totality, idempotence
and injectivity.

**Every coded enum carries the whole contract**, not a favoured few: `ALL`,
`CODES`, `as_str`, `Display`, `FromStr`, and a `serde` tag that *is* the
`as_str` code. That last equality is the load-bearing one — it means a database
`CHECK` constraint generated from `CODES` cannot drift from what the crate
writes.

`tests/code_contract.rs` asserts all six for every one of them, and adding an
enum without adding it there is the only way out. A `Debug` rendering is not a
contract: a rename would go on writing rows, spelled differently, with nothing
anywhere failing.

A **code** and a **description** are different things, and the types with both
keep them apart: `Holiday::as_str()` is `BUSS_UND_BETTAG` and `name()` is
*"Buß- und Bettag"*; `RegisterUnit::as_str()` is `KILO_WATT_HOUR` and `symbol()`
is `kWh`. Parsing also takes a few **input aliases** — `WÄRME`, `DE-BY`, `ÜNB` —
which are never written back, and are deliberately absent from `CODES`.

The harder half of the rule is the other direction: **one meaning, one value.**
Two distinct values that mean the same thing are the same failure seen from the
inside — two keys, two map entries, one concept. `IntervalResolution::Custom`
carries an opaque `CustomSeconds` that refuses every length which already has a
name, so `Custom(900)` cannot exist beside `QuarterHour`: the property is
enforced by construction rather than by convention.

## Serde representation stability

With the `serde` feature, enum tags and field names are **part of the public API
and covered by semver**. A test pins every tag literally, so the commitment is
mechanical rather than a promise.

`ObisCode` serialises as its IEC 62056 string and `IntervalResolution` as its
ISO 8601 duration — external standards no refactor here can rename.

## Enum exhaustiveness

**Domain enums are exhaustive; only error enums are `#[non_exhaustive]`.**

When a new `Messtyp` or `SubstituteMethod` appears, a consumer mapping this
crate's vocabulary onto their own storage codes wants their build to break, so a
human decides what the new variant means. With a wildcard they get a silent
fallback instead — a reading filed under the wrong Messtyp, a substitute value
attributed to the wrong method. For a crate whose output ends up on an invoice,
a compile error at upgrade time is the cheaper failure by a wide margin.

An error enum is the opposite case: a consumer that wildcards an unfamiliar
error still reports a failure, so there is nothing to protect.

Adding a variant to a domain enum is therefore a breaking change here, and is
released as one.

## No second copy of a fact

A field that restates something already derivable is a field that can contradict
it, and nothing downstream can tell which one is right.

- A register's unit comes from its OBIS code (`ObisCode::register_unit`), never
  from a stored `unit` column — a register tagged `kWh` with code `1-0:3.8.0` is
  kvarh however the column is set.
- `MeterExchangeEvent` carries `exchange_at` and derives `exchange_date()` from
  it; a hand-filled date is free to say 14 June where the instant says the 15th.
- `MeasurementSeries::worst_quality()` is computed on demand, not cached, so a
  direct edit to `intervals` cannot leave it stale.
- `SubstituteEntry::method` records the method that **ran**, not the one that
  was asked for, because a fallback that reports the request puts a claim in the
  audit trail the number does not support.
- `GgvInterval::capped()` is a method over `share` and `allocated`, not a
  stored flag that could contradict the two numbers beside it — and
  `compute_virtual_meter` projects from the allocation rather than recomputing
  the § 42b Abs. 5 cap, so the two entry points cannot drift.

## Unknown is not good

A recurring rule, learned the hard way in several places: where a quantity
cannot be determined, the API says so rather than returning a benign-looking
default.

- `ResampledBucket::coverage_pct()` and `is_complete()` return `Option` — a
  bucket whose expected count is underivable is *unknown*, not 100 % complete.
- `to_lastgang` emits no interval for a span it cannot difference honestly.
- `consumption_between` returns the `Anomaly` rather than clamping a backwards
  register to zero.
- Coverage measured against a *declared* period, not the extent of whatever
  data happened to arrive.
- `DynamicSlpProfile::value_at` returns `None` when a profile needs dynamizing
  and no Dynamisierungsfunktion was supplied, rather than handing back an
  entdynamisiert value as though it were a real one.
- A validation report says **which rules ran**. Four are opt-in, so a clean
  result means "the rules that ran found nothing" — and
  `ValidationConfig::disabled_rules()` and `ValidationResult::evaluated` make
  the difference between that and "nothing is wrong" a fact rather than an
  assumption.
- `ObisCode::as_lastgang()` returns `None` for a tariff register rather than
  inventing `1-0:1.29.1`, a code the market does not define.
