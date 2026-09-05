+++
title = "Sessions and allocation"
description = "Placing a charging session or a device log on the metering grid without losing a kWh, and splitting one pool across many claims with the residual reported."
weight = 13
+++

Two operations sit either side of the settlement grid, and both are the same
promise in different clothes: **nothing is created, and nothing is lost.**

| Direction | Function | Identity |
|---|---|---|
| One total → many slots | `split_session` | `Σ slot = session total` |
| One slot → many claims | `allocate` | `Σ allocated + residual = total` |

Neither is a rounding pass. Where a division does not terminate, the quotient
is cut once, to `ALLOCATION_DP` (six places), and the arithmetic is arranged so
that the cut cannot break the identity.

---

## Sessions → Lastgang

Some energy is measured as a **total over a span** rather than per slot. A
charge point reports one Charge Detail Record for a session that ran 17:42 to
21:09; a submetered heat pump reports what it drew between two visits; a device
log reports a day. The market settles on slots, so before that energy can be
allocated, balanced or invoiced it has to be placed on the grid.

```rust
use metering::session::{MeterSample, SessionSplitConfig, split_session};
use metering::QualityFlag;
use rust_decimal::dec;
use time::macros::datetime;

// The charge point sent a clock-aligned reading at 12:15 and at 12:30.
let samples = [
    MeterSample::new(datetime!(2026-06-01 12:15 UTC), dec!(1000)),
    MeterSample::new(datetime!(2026-06-01 12:30 UTC), dec!(1006)),
];

let slots = split_session(
    datetime!(2026-06-01 12:07 UTC),
    datetime!(2026-06-01 12:37 UTC),
    dec!(10),                      // the CDR total, kWh
    &samples,
    &SessionSplitConfig::quarter_hourly(),
)?;

// The 12:15–12:30 slot is bounded by two readings, so it is measured.
assert_eq!(slots[1].value, dec!(6));
assert_eq!(slots[1].quality, QualityFlag::Measured);

// The two ends had to be pro-rated across a boundary, and say so.
assert_eq!(slots[0].quality, QualityFlag::Estimated);
assert_eq!(slots[2].quality, QualityFlag::Estimated);

// And the total survives, to the digit.
assert_eq!(slots.iter().map(|s| s.value).sum::<rust_decimal::Decimal>(), dec!(10));
# Ok::<(), metering::session::SessionError>(())
```

### Measured and pro rata are different things

| Basis | Where it comes from | Quality |
|---|---|---|
| **Metered** | the difference of two register readings on this slot's own boundaries | `Measured` |
| **Pro rata** | a span straddling a boundary, divided by wall-clock time | `Estimated` |

The distinction is not cosmetic. A supplier settling a § 42b allocation, a
Bilanzkreis reconciling a Summenzeitreihe and a customer disputing a peak all
need to know which quarter-hours were *measured* and which were inferred from a
constant-power assumption the session almost certainly did not obey. OCPP's
clock-aligned meter values exist precisely so the first row can be filled in.

`MeasurementSource` records which kind of record a series came from —
`ChargeDetailRecord`, `ClockAlignedMeterValue` or `DeviceLog` — so the
provenance survives alongside the per-slot flag.

### Why the total cannot drift

Not by summing the slots and correcting the last one. Each slot is the
**difference of two adjacent cumulatives**, so the series telescopes to
`cum(end) − cum(start)` = `total − 0`, whatever rounding the cumulative itself
needed. Truncation toward zero keeps that cumulative monotone, so no slot comes
back negative.

### The DST arithmetic is not re-derived

The grid comes from `DayBoundary::bucket_bounds`, the same function `resample`
buckets with. A session running through the October Sunday sees the repeated
hour; one running through the March Sunday sees the skipped one; a daily or
Gastag grid gets 23, 24 or 25 hours from the calendar rather than a flat
86 400 s.

```rust
use metering::session::{SessionSplitConfig, split_session};
use metering::calendar;
use rust_decimal::dec;
use time::macros::date;

let long_day = date!(2026 - 10 - 25);
let slots = split_session(
    calendar::day_start_utc(long_day),
    calendar::day_end_utc(long_day),
    dec!(100),
    &[],
    &SessionSplitConfig::quarter_hourly(),
)?;

assert_eq!(slots.len(), 100, "the autumn day has 100 quarter-hours");
assert_eq!(slots.iter().map(|s| s.value).sum::<rust_decimal::Decimal>(), dec!(100));
# Ok::<(), metering::session::SessionError>(())
```

### What it refuses

A register reading that runs backwards, a sample outside the span, and a total
the samples contradict are errors — each is a contradiction in the input, and
spreading it across the grid would hide it.

Pro rata is not a **profile**. If a better shape is known, express it by
supplying more `MeterSample`s.

### Adding sessions up

A Übergabestelle has many sessions behind it, and what a Bilanzkreis settles is
their sum. `merge_sessions` adds series that already share a grid:

```rust
use metering::session::{SessionSplitConfig, merge_sessions, split_session};
use rust_decimal::dec;
use time::macros::datetime;

let cfg = SessionSplitConfig::quarter_hourly();

// Two cars, overlapping in the 12:15 slot and nowhere else.
let a = split_session(
    datetime!(2026-06-01 12:00 UTC), datetime!(2026-06-01 12:30 UTC),
    dec!(8), &[], &cfg,
)?;
let b = split_session(
    datetime!(2026-06-01 12:15 UTC), datetime!(2026-06-01 12:45 UTC),
    dec!(4), &[], &cfg,
)?;

let merged = merge_sessions(&[a, b]);

assert_eq!(merged.len(), 3, "a union of the slots, not an intersection");
assert_eq!(merged[1].value, dec!(6), "both cars charging");
assert_eq!(merged.iter().map(|s| s.value).sum::<rust_decimal::Decimal>(), dec!(12));
# Ok::<(), metering::session::SessionError>(())
```

**Union, not intersection — which is why it is a different function.**
`compute_virtual_meter` with `AggregationRule::Sum` keeps only the timestamps
present in *all* its sources, because a missing source there means the total
would be wrong. A charge point that was idle contributes no intervals, and that
absence *is* zero energy. Choosing the wrong one silently produces a plausible
number, so it is not a flag.

Intervals are grouped by `(from, to, obis_code)`, so a bidirectional point's
import and export do not collapse into one total; each slot carries the worst
quality among its contributors. Only slots something touched appear — filling an
idle hour with zeros is `fill_gaps`, which records what it invented.

---

## One pool, many claims

`allocate` is the other direction, and it is the single arithmetic behind every
allocation in the crate: `compute_community_allocation` is this function applied
once per quarter-hour, and `compute_ggv_allocation` applies the same cut and the
same cap for one tenant.

```rust
use metering::allocation::{AllocationBasis, AllocationPart, allocate};
use rust_decimal::dec;

// 12 kWh arrived at the Übergabestelle in this quarter-hour, and three
// sessions ran behind it. Each is capped at what its own meter recorded.
let row = allocate(
    dec!(12),
    vec![
        AllocationPart::new("S1", dec!(6)).capped_at(dec!(6)),
        AllocationPart::new("S2", dec!(3)).capped_at(dec!(3)),
        AllocationPart::new("S3", dec!(3)).capped_at(dec!(1)), // cable pulled early
    ],
    AllocationBasis::Proportional,
)?;

assert_eq!(row.part("S3").unwrap().share, dec!(3));
assert_eq!(row.part("S3").unwrap().allocated, dec!(1));
assert!(row.part("S3").unwrap().capped());

assert_eq!(row.residual, dec!(2), "two kWh nobody claimed");
assert_eq!(row.allocated() + row.residual, row.total);
# Ok::<(), metering::allocation::AllocationError>(())
```

### Two bases, one cap

| Basis | Share | Used by |
|---|---|---|
| `Fraction` | `weight × total` — weights are absolute, must be positive and sum to at most 1 | § 42b constant key (UTILTS `CCI+ZG6`) |
| `Proportional` | `(weight ÷ Σ weight) × total` — only ratios matter | § 42b proportional key (UTILTS `Z74`) |

`allocated = min(capacity, share)` in both, never below zero — the `Pos()`
operator of the BDEW Anwendungshilfe, where the ceiling is the participant's own
consumption. Under § 42c the *Aufteilungsschlüssel* is a contractual input
(Abs. 3 Nr. 2 publishes no formula), and both bases express one.

### The residual is a quantity, not a rounding error

Nothing redistributes it: no largest-remainder pass, no "give the rest to the
biggest". Under § 42b the residual is the generation that fed the public grid.
Turning it into a correction on somebody's invoice would credit them energy they
did not receive.

### A negative weight is refused

Under `Proportional`, a negative weight does not merely take nothing — it
shrinks the denominator and so **inflates** every other part's share. That is a
silent over-allocation, which is the exact failure this module exists to make
impossible, so it is an error rather than something absorbed.

`validate_key` runs the same checks without dividing anything, so a persisted
allocation rule can be rejected when it is *stored* rather than a month later in
a settlement run.

---

## Directional balance

A bidirectional charge point, a battery or a V2G session delivers import *and*
export for the same quarter-hour. The market already keeps the two apart, in
value group C of the OBIS code, so `metering` reads the direction off the code
rather than carrying a second, separately-mutable copy of it on `MeterInterval`.

```rust
use metering::{Direction, MeterInterval, QualityFlag, aggregation::sum_by_direction};
use rust_decimal::dec;
use time::macros::datetime;

let iv = |code: &str, kwh| MeterInterval {
    from: datetime!(2026-06-01 12:00 UTC),
    to:   datetime!(2026-06-01 12:15 UTC),
    value: kwh,
    quality: QualityFlag::Measured,
    obis_code: Some(code.parse().unwrap()),
};

let grid = [iv("1-0:1.8.0", dec!(9)), iv("1-0:2.8.0", dec!(4))];
let allocated = [
    iv("1-0:1.8.0", dec!(5)), iv("1-0:1.8.0", dec!(4)),
    iv("1-0:2.8.0", dec!(4)),
];

let measured = sum_by_direction(&grid);
let split = sum_by_direction(&allocated);

assert_eq!(measured.import - split.import, dec!(0));
assert_eq!(measured.export - split.export, dec!(0));
assert_eq!(measured.net(), dec!(5));
assert_eq!(iv("1-0:2.8.0", dec!(1)).direction(), Some(Direction::Export));
```

Three buckets, not two: an interval whose code has no direction — a reactive
register, a gas volume, or no code at all — lands in `undirected` rather than
being dropped, so `import + export + undirected` is always the plain sum of the
input. A **signed** interval is deliberately not offered: a negative kWh here
means a Korrekturenergiemenge (EDI@Energy *Codeliste* v2.5c §2.1), and
overloading the sign would make those two indistinguishable.

Unlike `aggregate`, this counts every interval, billable or not. It answers a
*physical* question — an allocation that drops a `Faulty` quarter-hour has still
lost that energy — where `aggregate` answers *"can this period be invoiced"*.

### Direction on a measurement point

`MeasurementPoint` states direction twice, and that redundancy cannot be
removed: `EnergyFlow` is master data about the point's *purpose*, and it also
distinguishes storage from load and marks a four-quadrant meter
`Bidirectional`, none of which any OBIS code says. So the crate applies its
other answer for a fact stated twice — make the disagreement **reportable**.

```rust
use metering::{Direction, EnergyFlow, ObisCode};

// A point whose OBIS code counts Bezug while its master data says Generation.
mp.obis_code = ObisCode::STROM_BEZUG_TOTAL;
mp.energy_flow = EnergyFlow::Generation;

// The metered code wins — it is what was measured...
assert_eq!(mp.direction(), Some(Direction::Import));
// ...and the contradiction is a fact you can log or assert on.
assert_eq!(mp.direction_conflict(), Some((Direction::Import, Direction::Export)));

mp.energy_flow = EnergyFlow::Consumption;
assert_eq!(mp.direction_conflict(), None);
```

Where the code carries no direction — a gas volume, a Zustandszahl — the master
data decides. Where neither does, the answer is `None` rather than a guess.
