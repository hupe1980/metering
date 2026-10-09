+++
title = "EEG and EnFG"
description = "MiSpeL (BNetzA 618-25-02, adopted 01.10.2026) for storage and charge points, the EEG § 51/§ 51a negative-price quarter-hours, and EnFG § 46 Zeitgleichheit."
weight = 13
[extra]
group = "Specialised"
+++

| Module | Rule | What leaves it |
|---|---|---|
| `eeg::mispel` | MiSpeL Anlage 1 and 2 | the Abgrenzungs- and Pauschaloption quantities, formula by formula |
| `eeg::negative_prices` | EEG § 51, § 51a | zero-value quarter-hours, the extension of the Vergütungszeitraum, the § 51 Abs. 3 energy |
| `eeg::zeitgleichheit` | EnFG § 46 Abs. 3 and 5 | the per-interval coincident quantity, the worst-case estimate |

**Quantities only.** Where a rule turns on the sign of a price (AW¼ > 0,
SP¼ ≥ 0, a negative spot price) you pass **which** quarter-hours pass, never
the price. Inputs are quarter-hourly series on one grid; a mismatch is refused
with the first instant where the grids part.

## MiSpeL

*Marktintegration von Speichern und Ladepunkten* (BNetzA, 01.10.2026, Az.
618-25-02): a storage system or bidirectional charge point mixing grid and
plant electricity keeps its Umlageprivilegien (§ 21 EnFG) and Marktprämie
(§ 19 EEG) for the share these formulas attribute to it.

| Anlage | Module | Scope | Formulas | Period |
|---|---|---|---|---|
| 1, Abgrenzungsoption | `mispel::abgrenzung` | Z1 at the connection, Z2 before the storage or charge point; cases A1–A11 | (1)–(40) | **calendar month**: `Abgrenzungsoption::monat`; `abgrenzung::jahr` sums months |
| 2, Pauschaloption | `mispel::pauschal` | PV ≤ 30 kWp behind one meter; cases P1–P5; 500 kWh/kWp (P1) and P1 plus an Indifferenzbereich (P4) replace Z2 | (P1)–(P22) | **calendar year** or Rumpfjahr |

Per quarter-hour the Speichervorrang and gates, per month the sums, then every
`MIN`/`MAX` clamp. Every numbered intermediate is in the result as `fN`.

```rust
use metering::eeg::mispel::pauschal::{Pauschalbasis, Pauschalgrenzen};
use rust_decimal::dec;

// Anlage 2, p. 13: 8 kWp of PV and a 10 kWh storage.
let g = Pauschalgrenzen::new(Pauschalbasis::Stromspeicher { kapazitaet_kwh: dec!(10) }, dec!(8)).unwrap();
assert_eq!((g.p1, g.p3, g.p4), (dec!(4000), dec!(320), dec!(4320)));
```

**Your input:** the Formelsatz (binding for a calendar year) and whether the EU
state-aid approval of § 19 Abs. 3c EEG, from which the Pauschaloption applies,
is granted. **Out:** the money formulas (41)–(44) of case A9 and the § 22 EnFG
heat-pump quantity of the A7 and P5 footnotes. **No rounding** — the Festlegung
states none; its tables are rounded for display.

## EEG § 51 and § 51a — negative prices

§ 51 Abs. 1: *"Für Zeiträume, in denen der Spotmarktpreis negativ ist,
verringert sich der anzulegende Wert auf null."* § 51a extends the
Vergütungszeitraum by those quarter-hours (year of commissioning plus 19),
rounded **up** to whole days; for a Solaranlage halved and rounded up to
Volllastviertelstunden, spent month by month against the statutory table.

```rust
use metering::eeg::negative_prices::{extension_days, solar_extension, volllastviertelstunden};
use time::macros::date;

assert_eq!(extension_days(97), 2); // up, never to nearest
assert_eq!(volllastviertelstunden(1001), 501);

// 500 Volllastviertelstunden from an end on 15 April: the rest of April's
// budget, then May's.
let e = solar_extension(date!(2045 - 04 - 15), volllastviertelstunden(1000)).unwrap();
assert_eq!(e.end, date!(2045 - 05 - 31));
```

`negative_span_energy` gives the § 51 Abs. 3 energy per uninterrupted span (the
5 % reduction is money). Whether § 51 applies to a plant is your fact.

## EnFG § 46 — Zeitgleichheit

Abs. 5: *"höchstens bis zu der Höhe der tatsächlichen Netzentnahme, bezogen auf
jedes 15-Minuten-Intervall"* — the minimum per quarter-hour, summed after:
`Σ min(a, b)`. MiSpeL's Speichervorrang is the same function.

```rust
use metering::eeg::zeitgleichheit::{worst_case_estimate, zeitgleichheit};
use metering::prelude::*;
use time::{Duration, macros::datetime};

let t = datetime!(2026-06-01 0:00 UTC);
let series = |values: [Decimal; 2]| {
    let slots = values
        .iter()
        .enumerate()
        .map(|(i, v)| MeterInterval::quarter_hour(t + Duration::minutes(15 * i as i64), *v, QualityFlag::Measured))
        .collect::<Result<Vec<_>, _>>().unwrap();
    Series::new(Resolution::QUARTER_HOUR, DayBoundary::Strom, slots).unwrap()
};
let z = zeitgleichheit(&series([dec!(5), dec!(1)]), &series([dec!(2), dec!(4)])).unwrap();
assert_eq!(z.total, dec!(3)); // min(5, 2) + min(1, 4), not min(6, 6)

// Abs. 3 Satz 4: maximum draw × the full hours of the Berlin calendar year.
let e = worst_case_estimate(dec!(2.5), 2028).unwrap();
assert_eq!(e.hours, 8_784);
```
