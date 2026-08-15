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

## Intersection semantics

Only timestamps present in **all** required source series appear in the output.
A gap in any one source propagates rather than silently producing a wrong total
— which for a GGV allocation would mean crediting a tenant against generation
that was never measured.

Source series must share a timestamp grid; resample first if they do not.
