+++
title = "Virtual meters"
description = "§ 42b EnWG Gemeinschaftliche Gebäudeversorgung: constant and proportional PV allocation, and the two caps that apply."
weight = 11
+++

A virtual meter derives one series from others.

| Rule | Formula |
|---|---|
| `Sum` | `Σ sources[i][t]` |
| `Residual` | `total[t] − Σ subtract[i][t]` |
| `PvSelfConsumption` | `grid[t] − generation[t]` |
| `GgvConstantAllocation` | `max(0, consumption[t] − fraction × generation[t])` |
| `GgvProportionalAllocation` | `max(0, consumption[t] − ratio[t] × generation[t])` |

`PvSelfConsumption` returns one **signed** series — positive is draw, negative
is feed-in. The three quantities a prosumer report wants follow from it without
further metering: `grid_draw = max(0, r)`, `grid_feed_in = max(0, −r)`,
`self_consumption = load − grid_draw`.

## Two caps, from two places

§ 42b Abs. 5 EnWG caps the **pool**:

> … wobei die rechnerisch aufteilbare Strommenge begrenzt ist auf die
> Strommenge, die innerhalb eines 15-Minuten-Zeitintervalls in der Solaranlage
> erzeugt oder von allen teilnehmenden Letztverbrauchern verbraucht wird, je
> nachdem welche dieser Strommengen geringer ist.

The **per-tenant** cap — the `max(0, …)`, so no tenant is credited more PV than
they drew — is the `Pos()` operator of the BDEW *Anwendungshilfe Solarpaket 1*
(v1.0, 25.01.2024), not that sentence. Both bounds together are what the module
enforces.

## The allocated amount, not just the net

`compute_virtual_meter` returns the tenant's **net grid draw**, which is what
the Marktlokation is billed for. A § 42b GGV or § 42c Energy-Sharing settlement
also needs the **allocated** energy — the share of community PV credited to the
tenant — and recovering that from the net meant re-reading and re-projecting the
tenant's own consumption series purely to subtract it back out, *and* reproducing
the `Pos()` cap in the caller, where a change to the cap would never reach it.

`compute_ggv_allocation` returns all of it:

```rust
use metering::{AggregationRule, MeterInterval, QualityFlag, compute_ggv_allocation};
use rust_decimal::dec;
use std::collections::HashMap;
use time::macros::datetime;

let iv = |kwh| vec![MeterInterval {
    from: datetime!(2026-06-01 12:00 UTC),
    to:   datetime!(2026-06-01 12:15 UTC),
    value: kwh, quality: QualityFlag::Measured, obis_code: None,
}];
let mut sources = HashMap::new();
sources.insert("PLANT".to_owned(), iv(dec!(10)));  // 10 kWh generated
sources.insert("T1".to_owned(),    iv(dec!(1)));   //  1 kWh drawn

let rule = AggregationRule::GgvConstantAllocation {
    plant_melo_id:  "PLANT".to_owned(),
    tenant_melo_id: "T1".to_owned(),
    fraction:       dec!(0.5),
};
let out = compute_ggv_allocation(&rule, &sources)?;

assert_eq!(out[0].share,         dec!(5.0));  // nominal half of the plant
assert_eq!(out[0].allocated,     dec!(1));    // ...capped at what they drew
assert_eq!(out[0].net_grid_draw, dec!(0));
assert!(out[0].capped());
assert_eq!(out[0].surplus_to_grid(), dec!(4.0));
# Ok::<(), metering::VirtualMeterError>(())
```

`capped()` is the one an operator wants: it says this tenant's share was limited
by their own consumption and the remainder fed the grid, which is the whole
economics of § 42b Abs. 5. It is a **method**, derived from `share` and
`allocated`, not a stored flag that could contradict the two numbers beside it.

The identity `consumption == allocated + net_grid_draw` holds in every interval,
exactly — all three are `Decimal`. And `compute_virtual_meter` projects from
this result rather than recomputing, so the `Pos()` cap has one implementation
and the two entry points cannot drift.

## One discriminator, one spelling

`AggregationRule` is **internally tagged** on a `kind` field carrying
`VirtualMeterKind`'s own code:

```json
{ "kind": "GGV_CONSTANT_ALLOCATION", "plant_melo_id": "…", "fraction": "0.1" }
```

The derived form was externally tagged with the Rust variant names, so one
discriminator had two spellings *and* two positions — a JSON key here, a JSON
value on `VirtualMeterKind`. Storing a rule as `jsonb` then meant a separate
`rule_type` column, because a key cannot be indexed or queried as a value, and a
recursive JSON path into the payload, because its depth depended on the variant.

Internal tagging needs a self-describing format, so this type will not
round-trip through bincode or postcard. That is the right way round: a rule is
configuration, stored once per delivery point in a queryable document, and the
hot types a binary format is chosen for — `MeterInterval`, `ObisCode` — are
untouched. Both halves are pinned by a test; see
[design](@/docs/design.md#the-hot-types-survive-a-non-self-describing-format).

## Intersection semantics

Only timestamps present in **all** required source series appear in the output.
A gap in any one source propagates rather than silently producing a wrong total
— which for a GGV allocation would mean crediting a tenant against generation
that was never measured.

Source series must share a timestamp grid; resample first if they do not.

## The whole community at once

`compute_ggv_allocation` answers for one tenant, which is the shape a persisted
`virtual_meter_configs` row has. `compute_community_allocation` answers for the
community, which is the shape a settlement, a dashboard and an optimiser want —
and it is not the same computation run N times:

- the proportional denominator is formed once instead of once per tenant, and
  the source index once instead of N times, so a year of quarter-hours across a
  twenty-flat building stops being quadratic in the tenant count;
- the **surplus** that fed the grid is a number, not something to reconstruct by
  subtracting N results from the generation;
- and the § 42b Abs. 5 pool ceiling becomes computable at all, because it is
  defined over the whole participant set.

```rust
use metering::{AllocationKey, compute_community_allocation};

let key = AllocationKey::Proportional {
    participants: vec!["T1".to_owned(), "T2".to_owned()],
};
let out = compute_community_allocation("PLANT", &key, &sources)?;
let interval = &out[0];

assert_eq!(interval.pool_cap, interval.generation.min(interval.total_consumption));
assert!(interval.total_allocated() <= interval.pool_cap);
assert_eq!(interval.generation, interval.total_allocated() + interval.surplus_to_grid);
```

### The pool cap is a theorem, not a step

§ 42b Abs. 5 caps *"die rechnerisch aufteilbare Strommenge … auf die Strommenge,
die innerhalb eines 15-Minuten-Zeitintervalls in der Solaranlage erzeugt oder
von allen teilnehmenden Letztverbrauchern verbraucht wird, je nachdem welche
dieser Strommengen geringer ist."* — the pool, not the individual share.

The function does not clamp to that figure, because it does not have to: with
fractions summing to at most 1, the per-participant `Pos()` cap already implies
it. `Σ min(cᵢ, shareᵢ) ≤ Σ cᵢ` and `Σ shareᵢ ≤ generation`, so the allocated
total never exceeds either limb. Clamping a second time would be a rule the
statute does not contain, applied to a quantity that already satisfies it.

`pool_cap` reports the ceiling so a caller can assert against it, and
`tests/allocation_invariants.rs` holds the inequality under proptest — an
argument about a billed quantity is worth a proof obligation.

### A share is a quantity, so it has a scale

The proportional key divides, and a `Decimal` quotient carries up to 28
significant digits. `ALLOCATION_DP` cuts the derived share to **six** decimal
places, toward zero, for two reasons.

It makes the share a number somebody can write down: no invoice, no MSCONS field
and no settlement system has a place for 0.333…3 kWh to twenty-seven places.

And it is what makes `consumption = allocated + net_grid_draw` hold exactly.
An uncut proportional share leaves `consumption − allocated` needing more
significant digits than a `Decimal` has, and the identity misses by a few
1e-27 kWh.

Cutting *toward zero* is not stylistic: truncating can only lower a share, so
`Σ allocated ≤ Σ share ≤ generation` survives it and the ceiling above stays a
theorem. The consumption itself is never cut — it is the caller's measurement —
so a participant whose whole draw is covered is credited exactly what they drew
and nets exactly zero, at whatever scale their meter delivered.

## § 42c Energy Sharing uses the same arithmetic

Energy Sharing has **no published allocation formula**. § 42c Abs. 3 Nr. 2
requires the *contract* to state

> einen Aufteilungsschlüssel, aus dem sich der Umfang des Rechts zur Nutzung der
> Elektrizität ergibt

so the key is an input rather than a constant, and both `AllocationKey` variants
express one. The per-participant `min(consumption, share)` carries over as
physics rather than law — nobody can be credited energy they did not draw.

What does **not** carry over is `pool_cap`: § 42c contains no counterpart to
§ 42b Abs. 5, so under a sharing contract that field is an observation, not a
ceiling. Eligibility — which delivery points can produce quarter-hour values at
all — is [`sharing`](@/docs/regulatory-basis.md), a separate question from the
allocation.

## One arithmetic underneath

`compute_community_allocation` is `allocation::allocate` applied once per
quarter-hour: the generation is the pool, each participant's own draw is the
ceiling, and the `AllocationKey` decides only how a weight becomes a share.
`compute_ggv_allocation` applies the same cut and the same cap for one tenant.

That matters beyond tidiness. The `Pos()` operator, the `ALLOCATION_DP` cut and
the residual are defined in exactly one place, so a § 42b settlement, a § 42c
contract key and a rule this crate has never heard of — a per-session split
behind a charge-point Übergabestelle, say — cannot disagree about them.

Any key the statutes do not publish is expressible directly:
[Sessions and allocation](@/docs/sessions-and-allocation.md).
