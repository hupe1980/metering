+++
title = "Design constraints"
description = "Determinism, exact decimals, one canonical string per value, serde stability and exhaustive domain enums — the invariants the library holds to."
weight = 15
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

## What "exact" means here

**No float, and one rounding at most.**

The first half is absolute: a metered quantity is a `Decimal` from the wire to
the invoice and never passes through `f64`. The second half is the part worth
stating, because "exact decimal" is routinely read as "never rounds", and that
is not true of any decimal type. `Decimal` carries 28–29 significant digits, and
a division whose quotient does not terminate is rounded to that width. `2 ÷ 3`
is `0.666…7` here as anywhere.

Addition, subtraction and multiplication of quantities at realistic scales do
not round at all, which is why the **conservation laws hold exactly**:

| Law | Where |
|---|---|
| a register split reconstructs its Arbeitsmenge | `split_energy` |
| a filled series covers its grid, slot for slot | `fill_gaps` |
| a consumption splits into a credited and a drawn part | `compute_community_allocation` |
| a resampling preserves the energy it buckets | `resample` |
| a Lastgang sums to the difference of its outer Zählerstände | `to_lastgang` |
| a session's total survives being spread across slots | `split_session` |
| `Σ allocated + residual = total`, for every key | `allocate` |

`tests/quantity_invariants.rs` asserts each of them over generated input.

### Division is where a choice has to be made

Two ways, and which one applies is a rule rather than a case-by-case decision:

- **Cut to a documented number of places** when the quotient is a value someone
  stores, prints or settles on — or when an identity depends on it.
  `ALLOCATION_DP` (6), `SUBSTITUTE_DP` (6), `SigLinDe::H_VALUE_DP` (6),
  `KUNDENWERT_DP` (4), `FORECAST_DP` (3), `BENUTZUNGSDAUER_DP` (2),
  `IMBALANCE_PCT_DP` (2). A share carrying twenty-seven decimal places is not a
  quantity, and it breaks the subtraction that follows it.
- **Leave it at full width** when it is an intermediate nothing downstream can
  distinguish. `allocation_temperature` feeds only `h_value`, which crosses into
  `f64` at once; cutting it would be a rule the Leitfaden does not state, bought
  for no benefit.

Where the rounding rule is the *market's* rather than this crate's it is a
parameter with a documented default instead — `G685Rounding` is the case where
published Netzbetreiber practice demonstrably disagrees with itself.

### Multiply before you divide

`a ÷ b × c` and `a × c ÷ b` are the same number in arithmetic and not in a
`Decimal`: the first rounds the quotient to 28 significant digits and then
scales the error up by `c`. Every share in this crate multiplies first. Three
equal tenants against 9 kWh of generation exhaust it exactly that way, and come
three millionths short the other.

The same reasoning gives an interval's average power as `kWh × 3600 ÷ s` rather
than `kWh ÷ (s ÷ 3600)` — one rounding instead of two, and identical for every
interval length the market actually uses.

### The consequence that surprises people

A quantity that has been cut is homogeneous only to its last reported place.
Doubling every reading doubles a Jahresprognose to within a year's worth of
`FORECAST_DP`, not exactly, because `round(2x)` and `2·round(x)` differ at a
rounding boundary.

That is a deliberate trade. The projection is computed from the **cut** daily
average, not from a 28-digit one nobody can see, so an operator who recomputes
`daily_average_kwh × year_days` gets the number the report states. MessEV
§ 25 Nr. 7 asks that *die verwendeten Werte* be fit for the purpose; a value
that cannot be re-derived from the figures beside it is not.

The distinction is easy to overstate in either direction. The
Allokationstemperatur has exact *weights* — eighths — and an inexact division
by 15. `UnitScale` keeps the *defining identities* exact — 3.6 GJ is 1 000 kWh
to the digit — and rounds everything else once.

## The market's vocabulary, not a parallel one

Where EDI@Energy publishes a code list for something this crate models, the
crate **is** that list. An invented enum has to be mapped at the market
boundary, the mapping is never quite one-to-one, and the meaning changes on the
way out.

| Crate | List | Segment |
|---|---|---|
| `SubstitutionReason` | 28 Gründe der Ersatzwertbildung | `STS+Z40` |
| `SubstituteMethod::market_code` | 13 Ersatzwertbildungsverfahren | `STS+Z32` |
| `QualityFlag::market_code` | the Mengen-Qualifier | `QTY` |

Each variant carries three labels because they answer three questions:
`code()` for the wire, `as_str()` for a database column (`Z81` says nothing to
a reader, and a code starting with `Z` sorts badly), `description()` for a
person — the published German title, verbatim.

Where the market has no code, the answer is `None` rather than the nearest
match. See [Ersatzwertbildung](@/docs/substitute-values.md).

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
ISO 8601 duration — external standards no refactor here can rename. The
identifiers travel as their digits.

**Instants are RFC 3339 and dates ISO 8601**, in a human-readable format:
`"2026-06-01T12:00:00Z"`, `"2026-06-01"`. That is the spelling a `TIMESTAMPTZ`
cast, a JSON Schema `format: date-time` and a log viewer all understand.

In a **binary** format they keep `time`'s own nine-integer tuple, which packs
tightly and which no schema language would recognise anyway. `MeterInterval`
carries two instants and is the hottest type in the crate, so a twenty-byte
string per boundary is a poor trade there. `serde` is asked which kind of format
it is and the answer decides.

### Quantities are exact decimal strings

A `Decimal` travels as `"12.345"`, the characters its `Display` writes, in every
format. There is no readable/binary split here because there is nothing to
trade: `"0.25"` is five postcard bytes against the sixteen a packed mantissa
would take, and it is the one form a human, a `NUMERIC` column and a JSON Schema
all read.

The representation is written on each field rather than inherited from
`rust_decimal`'s `serde` features, because **Cargo features are additive and
global to a build graph**. `serde-str` enabled here moves every `Decimal` in the
consumer's workspace from `deserialize_any` to `deserialize_str`, so a JSON
number stops being accepted in crates that never named `metering` and their
tests pass alone but fail in a workspace run. In the other direction,
`serde-float` set by *any* crate in that graph decides how these quantities
serialise — as `f64`, in a library whose claim is exact arithmetic.

Per-field costs an attribute and buys a wire format identical under every
feature combination anyone can select. Two source scans fail if a field forgets;
a third fails if the manifest reaches for a `rust_decimal/serde*` feature.

Reading asks for a **string**: a JSON number is a type error rather than a
silent trip through `f64`, and more digits than a `Decimal` holds are refused
rather than rounded away.

### The hot types survive a non-self-describing format

`MeterInterval`, `ObisCode`, `MeterReading` and the identifiers round-trip
through bincode and postcard; the internally tagged `AggregationRule` and
`AllocationKey` deliberately do not.

Asking for a string is what makes the first half hold: `deserialize_any` is the
one question a format without a self-describing wire cannot answer, and it is
what an inherited `Decimal` impl asks. A test holds both halves of the
trade-off.

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
- A `MeterInterval`'s flow direction comes from value group C of its OBIS code
  (`direction()`), never from a field beside it. `resample` and
  `split_session` bucket through one `DayBoundary::bucket_bounds`, so two
  callers cannot disagree about which slot a kWh belongs to; `allocate` is the
  one implementation of the `Pos()` cap and the residual.

### …and where it cannot be removed, make it reportable

`MeasurementPoint` states direction twice on purpose: `EnergyFlow` is master
data about the point's *purpose* and also distinguishes storage from load and
marks a four-quadrant meter `Bidirectional`, none of which an OBIS code says.
The duplication is therefore load-bearing, and silently preferring one of the
two would be the same failure in a different place. `direction()` believes the
metered code, and `direction_conflict()` returns `Some((measured, declared))`
so the contradiction is something you can log or assert on.

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
- A validation report says **which rules ran**. Four are opt-in and two more
  can be switched off, so a clean result means "the rules that ran found
  nothing" — and `ValidationConfig::disabled_rules()` and
  `ValidationResult::evaluated` make the difference between that and "nothing
  is wrong" a fact rather than an assumption.
- `GasConversionParams` has no `Default`. A Brennwert and a Zustandszahl are
  operator data, and a typical value for either is a silent percentage error on
  a billed quantity.
- `ObisCode::as_lastgang()` returns `None` for a tariff register rather than
  inventing `1-0:1.29.1`, a code the market does not define.
- `Dynamization::factor()` returns `None` outside day 1..=366. The polynomial is
  a quartic *fitted* to a year: at day 400 the 1999 function has already fallen
  through zero, so an off-by-one would otherwise scale every value it touches by
  a plausible-looking number.
- `BillingPeriod::uniform_resolution` is `false` where a series mixes interval
  lengths, so a Spitzenleistung taken across them is an upper bound the caller
  can qualify rather than a measurement it cannot.
- `SubstituteMethod::market_code()` returns `None` where the market has no code —
  a held value in Strom, a zero fill anywhere — instead of the nearest-looking
  one.

The same rule shapes the constructors. `MeterInterval::measured` is named for
the claim it makes; there is no `new` that would have to pick a `QualityFlag`,
and `Unknown` — the `Default` — is reachable only by asking for it.
