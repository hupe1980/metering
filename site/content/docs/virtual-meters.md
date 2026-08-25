+++
title = "Virtual meters"
description = "§ 42b EnWG Gemeinschaftliche Gebäudeversorgung: constant and proportional PV allocation, and the two caps that apply."
weight = 10
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

> die rechnerisch aufteilbare Strommenge [ist] begrenzt […] auf die Strommenge,
> die innerhalb eines 15-Minuten-Zeitintervalls in der Solaranlage erzeugt oder
> von allen teilnehmenden Letztverbrauchern verbraucht wird, je nachdem welche
> dieser Strommengen geringer ist.

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
untouched.

## Intersection semantics

Only timestamps present in **all** required source series appear in the output.
A gap in any one source propagates rather than silently producing a wrong total
— which for a GGV allocation would mean crediting a tenant against generation
that was never measured.

Source series must share a timestamp grid; resample first if they do not.
