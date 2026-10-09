+++
title = "Allocation"
description = "The UTILTS Berechnungsformel, the § 42b community allocation, § 42c Energy Sharing eligibility, and charging sessions placed on the settlement grid."
weight = 12
aliases = ["/docs/virtual-meters/", "/docs/sessions-and-allocation/"]
[extra]
group = "Specialised"
+++

## The Berechnungsformel

A Marktlokation that is not one Messlokation is computed by a formula the
Netzbetreiber transmits (UTILTS AHB 1.1, MIG 1.1e, PID 25001).
`allocation::formula::Formula` is that vocabulary and its evaluator; parsing
the message is the EDIFACT layer's job. Each Rechenschritt is one of:

| `Step` | Operators | Value per interval |
|---|---|---|
| `Sum` | Z69 Addition / Z70 Subtraktion | `Σ ±operand` |
| `Quotient` | one Z81 Dividend, one Z80 Divisor | `dividend ÷ divisor`, cut toward zero to `FORMULA_QUOTIENT_DP` |
| `Product` | Z82 Faktor | `Π operand` |
| `Positive` | one Z83 Positivwert | `max(0, step)` |

An operand is another step or a Messlokation, whose Z87 Energieflussrichtung
selects the **register** (never a sign) and whose loss and split factors apply
first. A zero divisor gives 0 for that interval, reported — BDEW AWH
*Beispiele von Berechnungsformeln für das Solarpaket 1*: *"Ist die Energiemenge
einer Marktlokation zugeordneten Messlokation = 0, so ist auch der Verbrauch der
Marktlokation auf 0 zu setzen."* `Formula::new` refuses a missing or
self-referencing step, a cycle, an empty step, and an unreferenced step.

```rust
use metering::allocation::formula::{Formula, SplitFactor};
use metering::{DayBoundary, Direction, MeloId, MeterInterval, QualityFlag, Resolution, Series};
use rust_decimal::dec;
use time::macros::datetime;

let plant: MeloId = "DE0001234567890000000000000000001".parse().unwrap();
let tenant: MeloId = "DE0001234567890000000000000000002".parse().unwrap();
let one = |kwh| {
    let iv = MeterInterval::quarter_hour(datetime!(2026-06-01 12:00 UTC), kwh, QualityFlag::Measured).unwrap();
    Series::new(Resolution::QUARTER_HOUR, DayBoundary::Strom, vec![iv]).unwrap()
};
let (generation, consumption) = (one(dec!(100)), one(dec!(20)));

// AWH Beispiel 1: MaLo Verbrauch = Pos(MeLo2 Verbrauch − 10 % MeLo1 Erzeugung).
let formula = Formula::constant_share(tenant, plant, SplitFactor::new(dec!(0.1)).unwrap()).unwrap();
let out = formula
    .eval(|melo, direction| match (*melo == plant, direction) {
        (true, Direction::Export) => Some(&generation),
        (false, Direction::Import) => Some(&consumption),
        _ => None,
    })
    .unwrap();
assert_eq!(out.series.as_slice()[0].value(), dec!(10.0));
```

## § 42b — Gemeinschaftliche Gebäudeversorgung

`community::allocate` divides one plant's generation across the participants,
per interval:

```text
pool        = max(0, generation)
share_i     = q_i × pool                          q_i from the key
allocated_i = min(max(0, consumption_i), share_i)  the Pos() cap
residual    = pool − Σ allocated_i                exactly
```

| `AllocationKey` | `q_i` | Source |
|---|---|---|
| `Constant` | the agreed `SplitFactor` | UTILTS ZG6, AWH Beispiel 1 |
| `Proportional` | `c_i ÷ Σ c`, cut | AWH Beispiel 3 |
| `EqualShares` | `1 ÷ n`, cut | § 42b Abs. 5 Satz 3 EnWG |
| `Cascading` | relative weights, unused share re-offered | none — contractual arithmetic |

The cut is the formula quotient's, so shares sum to at most 1 and
`Σ allocated ≤ pool`. The **residual** is the generation fed to the grid. A
tenant's figure equals what the Berechnungsformel computes for that tenant. A
negative consumption is clamped to zero for its interval and reported.

```rust
use metering::allocation::community::{AllocationKey, allocate};
use metering::{DayBoundary, MeloId, MeterInterval, QualityFlag, Resolution, Series};
use rust_decimal::dec;
use time::macros::datetime;

let one = |kwh| {
    let iv = MeterInterval::quarter_hour(datetime!(2026-06-01 12:00 UTC), kwh, QualityFlag::Measured).unwrap();
    Series::new(Resolution::QUARTER_HOUR, DayBoundary::Strom, vec![iv]).unwrap()
};
let t1: MeloId = "DE0001234567890000000000000000001".parse().unwrap();
let t2: MeloId = "DE0001234567890000000000000000002".parse().unwrap();
let (plant, a, b) = (one(dec!(10)), one(dec!(1)), one(dec!(3)));

let rows = allocate(&plant, &[(t1, &a), (t2, &b)], &AllocationKey::Proportional).unwrap();
let row = &rows[0];
assert_eq!(row.allocated(), dec!(4)); // both capped by their own draw
assert_eq!(row.residual, dec!(6)); // fed the grid
assert_eq!(row.allocated() + row.residual, row.pool());
```

## § 42c — Energy Sharing eligibility

Abs. 1 admits a point only when consumption and generation are measured by
*"Zählerstandsgangmessung nach § 2 Satz 1 Nummer 27 des
Messstellenbetriebsgesetzes oder durch eine viertelstündliche registrierende
Leistungsmessung"* — so an RLM meter qualifies, and an iMSys not configured for
Zählerstandsgangmessung does not. Dates are under [Regulatory
basis](@/docs/regulatory-basis.md).

| Question | Function | From |
|---|---|---|
| Can the point produce quarter-hour values? | `assess_capability` (refuses contradictory input) | master data |
| Is it producing them? | `assess_delivery` (billable quarter-hours against the window's real length) | the observed series |
| Both | `combine_readiness` → `SharingReadiness` | — |

*Capable but not delivering* is a configuration order, not a rollout. No
Aufteilungsschlüssel is published for § 42c; the keys above are arithmetic.

## Sessions on the grid

`split_session` places register readings over a span (a Charge Detail Record,
a device log) on the grid; `merge_sessions` adds sessions on one grid.
`Σ slot energy = last reading − first reading`, exactly. A slot bracketed by
readings is `Measured`; one split pro rata is `Estimated`. Slots come from
`DayBoundary::bucket`, so the autumn's repeated hour is there.

```rust
use metering::allocation::session::{MeterSample, SessionSplitConfig, split_session};
use metering::QualityFlag;
use rust_decimal::dec;
use time::macros::datetime;

// Meter start 12:07, clock-aligned values at 12:15 and 12:30, stop 12:37.
let samples = [
    MeterSample::new(datetime!(2026-06-01 12:07 UTC), dec!(998)),
    MeterSample::new(datetime!(2026-06-01 12:15 UTC), dec!(1000)),
    MeterSample::new(datetime!(2026-06-01 12:30 UTC), dec!(1006)),
    MeterSample::new(datetime!(2026-06-01 12:37 UTC), dec!(1008)),
];
let series = split_session(&samples, &SessionSplitConfig::quarter_hourly()).unwrap();
assert_eq!(series.as_slice()[1].value(), dec!(6));
assert!(series.iter().all(|s| s.quality() == QualityFlag::Measured));
assert_eq!(series.billable_total(), Some(dec!(10)));
```
