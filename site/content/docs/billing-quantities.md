+++
title = "Billing quantities"
description = "Arbeitsmenge, Spitzenleistung and the two figures a network invoice is actually built from: the Benutzungsstundenzahl that picks the tariff band, and the Blindmehrarbeit beyond the Freigrenze."
weight = 4
+++

A Netznutzungsabrechnung for an RLM Entnahmestelle rests on four numbers, and
all four are quantities rather than prices. This crate computes them; what they
cost is the Preisblatt's business.

| Quantity | Where it comes from | Basis |
|---|---|---|
| **Arbeitsmenge** (kWh) | the sum of the billable intervals | § 17 Abs. 2 StromNEV |
| **Jahreshöchstleistung** (kW) | the highest average power over any one interval | § 17 Abs. 2 StromNEV |
| **Benutzungsstundenzahl** (h) | Arbeitsmenge ÷ Jahreshöchstleistung | § 17 Abs. 1 StromNEV |
| **Blindmehrarbeit** (kvarh) | the reactive energy beyond the Freigrenze | the Netzbetreiber's Preisblatt |

## Arbeit and Leistung

```rust
use metering::{AggregationConfig, MeterInterval, aggregate};
use rust_decimal::dec;
use time::{Duration, macros::datetime};

// A flat 4 kW draw for one day.
let day: Vec<MeterInterval> = (0..96)
    .map(|i| MeterInterval::quarter_hour(
        datetime!(2026-06-01 0:00 UTC) + Duration::minutes(15 * i),
        dec!(1),
    ))
    .collect();

let period = aggregate(&day, &AggregationConfig::rlm());
assert_eq!(period.arbeitsmenge, dec!(96));
assert_eq!(period.spitzenleistung_kw, Some(dec!(4)));      // 1 kWh per quarter-hour
assert_eq!(period.spitzenleistung_at, Some(datetime!(2026-06-01 0:00 UTC)));
```

The peak is reported **with the interval it was first reached in** — the
Leistungspreis is the most disputed line on an RLM invoice, and "48 kW" does not
answer *when*.

## One resolution, or none

An average power over an hour is not comparable with one over a quarter-hour:
the hour has already averaged away the peak the quarter-hour would show. A
maximum taken across a series that mixes the two is a Spitzenleistung of
nothing.

```rust
# use metering::{AggregationConfig, MeterInterval, aggregate};
# use rust_decimal::dec;
# use time::macros::datetime;
let quarter = MeterInterval::quarter_hour(datetime!(2026-06-01 0:00 UTC), dec!(1));
let hour = MeterInterval::hour(datetime!(2026-06-01 0:15 UTC), dec!(2));

let mixed = aggregate(&[quarter, hour], &AggregationConfig::rlm());
assert!(!mixed.uniform_resolution);
assert_eq!(mixed.spitzenleistung_kw, Some(dec!(4)));
```

The crate does not guess which resolution was meant, and it does not drop the
answer either. `uniform_resolution` is `false`, which makes the peak an upper
bound the caller can qualify, refuse, or resample away.

## Benutzungsstundenzahl

§ 17 Abs. 1 StromNEV makes the Netzentgelt depend on *"der jeweiligen
Benutzungsstundenzahl der Entnahmestelle"*, and Anlage 4 zu § 17 Abs. 2 builds
the Gleichzeitigkeitsgrad on the Jahresbenutzungsdauer, its two straight lines
meeting *"durch die Jahresbenutzungsdauer 2 500 Stunden"* and reaching 1 at
8 760 Stunden. It is the figure a price sheet's two tariff bands are separated
by — and it is a ratio of two quantities, so it lives here:

```rust
# use metering::{AggregationConfig, MeterInterval, aggregate};
# use rust_decimal::dec;
# use time::{Duration, macros::datetime};
# let day: Vec<MeterInterval> = (0..96).map(|i| MeterInterval::quarter_hour(
#     datetime!(2026-06-01 0:00 UTC) + Duration::minutes(15 * i), dec!(1))).collect();
let period = aggregate(&day, &AggregationConfig::rlm());

// A flat load uses every hour of its own period.
assert_eq!(period.benutzungsdauer_h(), Some(dec!(24)));
```

`None` when there is no peak to divide by. The 2 500 h threshold is stated for a
**year**; over a month the same arithmetic answers a different question.

## Blindmehrarbeit

A Netznutzer draws real energy (kWh) and reactive energy (kvarh). The reactive
part performs no work but loads the network, so the Netzbetreiber grants a
Freigrenze proportional to the Wirkarbeit and charges only the excess:

```text
Blindmehrarbeit = max(0, Blindarbeit − ratio × Wirkarbeit)
```

**The ratio is the Netzbetreiber's.** No national rule fixes it, and published
Preisblätter state it two ways: as *50 % der Wirkarbeit* (a `cos φ` of about
0,894), or as `cos φ = 0,9`, which is the slightly stricter 0,4843. Both are
offered, neither is presumed:

```rust
use metering::reactive::{ReactiveLimit, blindmehrarbeit};
use rust_decimal::dec;

let balance = blindmehrarbeit(dec!(100000), dec!(62000), ReactiveLimit::half());
assert_eq!(balance.freigrenze_kvarh, dec!(50000.0));
assert_eq!(balance.blindmehrarbeit_kvarh, dec!(12000.0));

// The same registers under cos φ = 0,9 admit less, so more is charged.
let strict = blindmehrarbeit(dec!(100000), dec!(62000), ReactiveLimit::cos_phi_0_9());
assert_eq!(strict.blindmehrarbeit_kvarh, dec!(13570.0000));

// What is left of the allowance — the figure a compensation is sized against.
let compensated = blindmehrarbeit(dec!(100000), dec!(20000), ReactiveLimit::half());
assert_eq!(compensated.headroom_kvarh(), dec!(30000.0));
```

The conversion `ratio = tan(arccos(cos φ)) = √(1 − cos²φ) ÷ cos φ` is *stated*
rather than computed: a square root has no exact decimal, and no float touches a
number that multiplies a billed quantity. `RATIO_COS_PHI_0_9` is documented as
the four-place rounding it is.

One product and one difference, so the balance reconstructs digit for digit from
the two register totals it was given.

## Import, export, and what has neither

A bidirectional Zählpunkt delivers a Bezug *and* an Einspeisung series for the
same quarter-hour. `sum_by_direction` reads the direction off OBIS value group C
and returns three buckets, not two:

```rust
use metering::{MeterInterval, aggregation::sum_by_direction};
use rust_decimal::dec;
use time::macros::datetime;

let iv = |code: &str, kwh| MeterInterval::quarter_hour(datetime!(2026-06-01 12:00 UTC), kwh)
    .with_obis(code.parse().unwrap());

let balance = sum_by_direction(&[iv("1-0:1.8.0", dec!(9)), iv("1-0:2.8.0", dec!(4))]);
assert_eq!(balance.net(), dec!(5));
assert_eq!(balance.total(), dec!(13));
```

The third bucket, `undirected`, holds everything whose code has no direction to
read — a reactive register, a gas volume, an interval with no code at all — so
`import + export + undirected` is always the plain sum of the input and no
energy disappears between the call and the result.
