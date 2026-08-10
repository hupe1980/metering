+++
title = "Design constraints"
description = "Determinism, exact decimals, one canonical string per value, serde stability and exhaustive domain enums — the invariants the library holds to."
weight = 11
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
