+++
title = "Design constraints"
description = "The rules every module follows: purity, exact arithmetic with one float boundary and a named rounding for every cut, determinism, one string per code, the serde representation, and the non_exhaustive policy."
weight = 14
[extra]
group = "Reference"
+++

The rules every module follows. Guides link here rather than restate them.

## Purity

**No I/O, no async, no clock, no environment.** Where an instant matters — the
`now` of a validation — it is an argument. Clippy's `disallowed-methods`
(`clippy.toml`, run by `just purity`) refuses the clock, `std::env` and
`std::fs` in the library. Runtime dependencies are `rust_decimal`, `thiserror`,
`time` and `time-tz`, whose `db` feature embeds the IANA database; `serde` is
optional.

## Exactness

**Every quantity is a `Decimal`**, from input to result. There is one float
crossing: the SigLinDe sigmoid raises to a non-integer power, so
`SigLinDe::h_value` evaluates it in `f64` and returns a `Decimal`. A source
scan refuses an `f64 ↔ Decimal` conversion anywhere else.

### One cut, named

Sums, differences and products do not round, so conservation holds to the
digit: a register split reconstructs its Arbeitsmenge, a Lastgang sums to the
difference of its outer readings, `Σ allocated + residual = pool`.

A quotient that does not terminate is cut **once, where it is formed**, to a
named `*_DP` constant with a named `*_STRATEGY` from `metering::precision`; a
bare `round_dp(` in `src/` fails a source scan. The default mode is
kaufmännisch wherever no source states one:

| Strategy | Constants | Why |
|---|---|---|
| **half away from zero** (kaufmännisch, DIN 1333) | `PERCENT_DP` 2, `BENUTZUNGSDAUER_DP` 2, `FORECAST_DP` 3, `SEASONAL_FACTOR_DP` 4, `FORECAST_ACCURACY_DP` 4, `AUSFALLARBEIT_DP` 3, `SUBSTITUTE_DP` 6 | no source states a mode |
| half away from zero | `DYNAMIZATION_DP` 4, `DYNAMIZED_VALUE_DP` 3 | BDEW AWH SLP Strom 2025 states the places, not a mode |
| half away from zero | `G685_ZUSTANDSZAHL_DP` 4, `G685_BRENNWERT_DP` 3 | published Netzbetreiber G 685 Merkblätter |
| **half to even** (*mathematisch*) | `KUNDENWERT_DP` 4, `ALLOCATION_TEMPERATURE_WEIGHT_DP` 4 | Leitfaden SLP Gas, Anlage 5, defines *mathematisch* as half to even |
| **toward zero** | `FORMULA_QUOTIENT_DP` 6 | a Berechnungsformel quotient — and the § 42b share — so the shares of one denominator sum to at most 1 |
| **toward zero** | `ALLOCATION_DP` 6 | a session's pro-rata cumulative, so no slot comes back negative |

The SLP-Gas h-value is not rounded (Anlage 5: no rounding after the
computation). Where the rounding is the market's choice — the G 685 final
amount — it is a parameter.

**Multiply before you divide.** `a × c ÷ b` rounds once; `a ÷ b × c` scales the
quotient's error by `c`. A source scan rejects a division by a simple term
followed by a multiplication.

## Determinism

Equal inputs give equal outputs, independent of input order (proptests shuffle
and draw ties on purpose). Where a value cannot be determined the answer is
`None` or an error, never a benign default: no `Default` for a Brennwert, no
Spitzenleistung across a coarse grid, no market code where the market has none,
no dynamisation factor for day 400.

## One string per code

Where EDI@Energy publishes a code list, the crate **is** that list —
`SubstitutionReason` is `STS+Z40`, `Method::market_code` is `STS+Z32` per
Sparte, `QualityFlag::market_code` is the `QTY` qualifier. Every coded enum has
`ALL`, `CODES`, `as_str`, `Display`, `FromStr`, and a `serde` tag that **is**
the `as_str` code, so a database `CHECK` generated from `CODES` cannot drift. A
code (`BUSS_UND_BETTAG`) and a description (*Buß- und Bettag*) stay apart;
parsing accepts input aliases that are never written back.

Every value with a string form has one canonical spelling:

```rust
use metering::{ObisCode, Resolution};

let a: ObisCode = "1-0:1.8.0*255".parse().unwrap();
let b: ObisCode = " 1-0:01.8.0 ".parse().unwrap();
assert_eq!(a, b);
assert_eq!(a.to_string(), "1-0:1.8.0");
assert_eq!(format!("{a:#}"), "1-0:1.8.0*255");

assert_eq!(Resolution::from_seconds(900), Some(Resolution::QUARTER_HOUR));
assert_eq!(Resolution::QUARTER_HOUR.to_string().parse::<Resolution>(), Ok(Resolution::QUARTER_HOUR));
```

## Serde

The `serde` feature enables `serde` and `time/serde`, and nothing else.

**Which types.** The coded enums, the identifiers (`MaloId`, `MeloId`,
`BdewCode`, `Eic`, `ObisCode`), `Resolution` and `Period`, the inputs
(`MeterInterval`, `Series`, `MeterReading`, `Formula`, `Zaehlzeitdefinition`,
the gas and § 14a parameters), and the results of billing, validation, power
quality, rollout, community allocation and the forecast (`BillingPeriod`,
`Report`, `En50160Report`, …). Not: the error types; the configurations
`Rules`, `Policy` and `LastgangConfig`; and the results of `substitute`,
`to_lastgang`, `split_energy`, the § 42c assessment, `eeg` and `heat` — record
the fields you need from those.

**The representation is part of the API and covered by semver**; a test pins
every tag literally.

| Format | Instant | Date | Quantity |
|---|---|---|---|
| JSON, YAML, TOML | `"2026-06-01T12:00:00Z"` (RFC 3339) | `"2026-06-01"` (ISO 8601) | `"12.345"` |
| bincode, postcard, MessagePack | `time`'s compact tuple | `time`'s compact form | `"12.345"` |

A quantity is the exact decimal string in every format; a JSON number is a
type error, and excess digits are refused rather than rounded. The
representation is set per field, not through a `rust_decimal` feature, which
would change every `Decimal` in your build graph. `ObisCode` travels as its IEC
62056 string, `Resolution` as its ISO 8601 duration.

## `#[non_exhaustive]`

| Non-exhaustive | Exhaustive (closed by definition) |
|---|---|
| errors; market and statutory code lists (`SubstitutionReason`, `QualityFlag`, `Sparte`, `Holiday`, `Bundesland`, `Rule`, …); findings, configurations and result structs | `Direction`, `DayBoundary`, `DayKind`, `Resolution`, `Quarter`, `SlpDayType`, the Berechnungsformel's operators and steps, the Redispatch directions and cases |

Structs with private fields are built through validating constructors —
`MeterInterval::new`, `Series::new`, `Formula::new` — so an invalid value
cannot be expressed.
