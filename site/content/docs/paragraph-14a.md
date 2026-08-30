+++
title = "§ 14a steering"
description = "The two powers a netzorientierte Steuerung turns on: the netzwirksamer Leistungsbezug it reduces, and the Mindestleistung it may not reduce past."
weight = 8
+++

BNetzA **BK6-22-300** (27.11.2023, in force 01.01.2024) lets a Netzbetreiber cut
the *netzwirksamer Leistungsbezug* of steuerbare Verbrauchseinrichtungen in an
overloaded Netzbereich, and guarantees the operator a floor while it does. Two
numbers decide everything, and both are powers in kW rather than money:

| Quantity | Source | Function |
|---|---|---|
| netzwirksamer Leistungsbezug | Anlage 1 Ziff. 2.3 | `netzwirksamer_leistungsbezug` |
| `P_min,14a`, Direktansteuerung | Anlage 1 Ziff. 4.5.1 | `mindestleistung_direktansteuerung` |
| `P_min,14a`, Steuerung mittels EMS | Anlage 1 Ziff. 4.5.2 | `mindestleistung_ems` |

The Netzentgelt *modules* that reward participation are a different Festlegung
(BK8-22/010-A); Modul 3's tariff windows live in
[tariff registers](@/docs/tariff-registers.md).

## One of the two is exactly citable, and the other is not

`P_min,14a` is printed in the Festlegung as a formula with its own
Gleichzeitigkeitsfaktor table. It is reproduced verbatim.

The netzwirksamer Leistungsbezug is only **defined** there. Ziff. 2.3 says which
share it is —

> derjenige Anteil der über den Netzanschluss aus einem
> Elektrizitätsverteilernetz der allgemeinen Versorgung entnommenen Leistung,
> der zeitgleich durch eine oder mehrere steuerbare Verbrauchseinrichtungen
> verursacht wird

— and stops. When local generation covers part of the load, *which* part of the
remaining grid draw the steuVE caused is an apportionment the text does not
perform. VDE FNN's *Bewertung der Mindestleistung* (V1.0, April 2025) says in as
many words that the calculation is **not its subject** and points on to
*Netzbetrieb mit Flexibilitäten* Kap. 4.1.2, which is not freely citable.

So the apportionment is a `Verursachungsregel` the caller picks, each with its
assumption written out — the same treatment G 685's final rounding and
VDE-AR-N 4400's thresholds get. What the crate guarantees is the arithmetic, not
conformance with a document it cannot quote.

```rust
use metering::para14a::{Verursachungsregel, netzwirksamer_leistungsbezug};
use rust_decimal::dec;

// A house drawing 10 kW from the grid: a 6 kW wallbox, 8 kW of other load,
// 4 kW of PV running.

// The PV serves the uncontrollable load first, so all 6 kW of the wallbox is
// still grid draw. Conservative: a guard on it fires early, never late.
assert_eq!(
    netzwirksamer_leistungsbezug(dec!(10), dec!(6), None, Verursachungsregel::SteuVeZuletzt),
    Some(dec!(6)),
);

// Pro rata, the wallbox is 6 of 14 kW of load and carries 10 × 6/14.
let anteilig = netzwirksamer_leistungsbezug(
    dec!(10), dec!(6), Some(dec!(8)), Verursachungsregel::Anteilig,
).unwrap();
assert!(anteilig < dec!(4.3));

// The pro-rata rule will not invent the rest of the installation.
assert_eq!(
    netzwirksamer_leistungsbezug(dec!(10), dec!(6), None, Verursachungsregel::Anteilig),
    None,
);
```

Ziff. 4.7 makes a separate Zählpunkt for the steuVE optional, so a
sub-measurement often does not exist. The conservative substitute is the
device's Netzanschlussleistung — assume it draws its rated power, which can only
overstate the steuVE share. That is a caller's choice, so pass
`measured.unwrap_or(nennleistung)` and keep the convention visible at the call
site.

## The floor: two branches and two traps

Ziff. 4.5.2, verbatim:

```text
Sofern Anlagen im Sinne der Ziffern 2.4.1.b sowie 2.4.1.c (jeweils i.V.m.
Ziffer 2.4.2), mit einer Netzanschlussleistung über 11 kW Bestandteil der
Steuerung nach Ziffer 4.4.b sind, gilt:

  P_min,14a = Max(0,4 x P_Summe WP; 0,4 x P_Summe Klima) + (n_steuVE − 1) x GZF x 4,2 kW

Ansonsten gilt:

  P_min,14a = 4,2 kW + (n_steuVE − 1) x GZF x 4,2 kW
```

**The first term of the upper branch is a maximum of two group sums, not
`0,4 ×` everything.** `P_Summe WP` sums Fallgruppe b and `P_Summe Klima` sums
Fallgruppe c, and the larger of the two scaled sums wins. Adding them instead
overstates the floor on any installation carrying both a heat pump and room
cooling — and a floor that is too high silently denies the Netzbetreiber
reduction headroom it is entitled to.

**`n_steuVE` counts all of them**, Ladepunkte and Speicher included, not only
the scaled ones: *"Anzahl aller steuerbaren Verbrauchseinrichtungen, die nach
Ziffer 4.4.b angesteuert werden"*.

```rust
use metering::para14a::{Para14aConfig, SteuVe, SteuVeFallgruppe as F, mindestleistung_ems};
use rust_decimal::dec;

let cfg = Para14aConfig::default();

// Three ordinary steuVE: 4,2 + (3−1) × 0,75 × 4,2 = 10,5 kW.
let plain = [
    SteuVe::new(F::Ladepunkt, dec!(11)),
    SteuVe::new(F::Waermepumpe, dec!(9)),
    SteuVe::new(F::Stromspeicher, dec!(10)),
];
assert_eq!(mindestleistung_ems(&plain, &cfg), Some(dec!(10.500)));

// A 20 kW heat pump instead: Max(0,4 × 20; 0,4 × 0) = 8 replaces the 4,2.
let scaled = [
    SteuVe::new(F::Ladepunkt, dec!(11)),
    SteuVe::new(F::Waermepumpe, dec!(20)),
    SteuVe::new(F::Stromspeicher, dec!(10)),
];
assert_eq!(mindestleistung_ems(&scaled, &cfg), Some(dec!(14.300)));
```

The Gleichzeitigkeitsfaktoren are the published table:

| `n_steuVE` | 2 | 3 | 4 | 5 | 6 | 7 | 8 | ≥ 9 |
|---|---|---|---|---|---|---|---|---|
| GZF | 0,8 | 0,75 | 0,7 | 0,65 | 0,6 | 0,55 | 0,5 | 0,45 |

`gleichzeitigkeitsfaktor(n)` returns `None` below two: the table starts there,
and at `n = 1` the term it multiplies is zero anyway.

## Which devices scale, and which do not

Ziff. 4.5.1 Satz 2 and Ziff. 4.5.2 Satz 3 both name *"Ziffern 2.4.1.b sowie
2.4.1.c"* — Wärmepumpe and Raumkühlung — and no others. So a 50 kW Ladepunkt
keeps the flat 4,2 kW floor while a 20 kW heat pump scales to 8 kW, and the
threshold is strict: exactly 11 kW is not *"über 11 kW"*.

```rust
use metering::para14a::{Para14aConfig, SteuVe, SteuVeFallgruppe as F, mindestleistung_direktansteuerung};
use rust_decimal::dec;

let cfg = Para14aConfig::default();
let wallbox = SteuVe::new(F::Ladepunkt, dec!(22));
let heat_pump = SteuVe::new(F::Waermepumpe, dec!(20));

assert_eq!(mindestleistung_direktansteuerung(&wallbox, &cfg), Some(dec!(4.2)));
assert_eq!(mindestleistung_direktansteuerung(&heat_pump, &cfg), Some(dec!(8.0)));
```

## A device below 4,2 kW is not a steuVE

Ziff. 2.4.1 admits the four Fallgruppen only *"mit einer Netzanschlussleistung
von mehr als 4,2 Kilowatt (kW)"*, and the bound is strict. Both functions return
`None` for a smaller device, and `mindestleistung_ems` returns `None` for a
**set** containing one.

That last refusal is the point. `n_steuVE` counts steuVE, so a stray 3 kW entry
would raise the floor for every other device in the set —
`4,2 + (2−1) × 0,80 × 4,2 = 7,56 kW` where the right answer is `4,2 kW` — and
quietly cost the Netzbetreiber reduction headroom it is entitled to. A silently
wrong answer to *"how much may I reduce?"* is worse than no answer, so the
function declines. `SteuVe::is_steuerbar()` says why, and where Ziff. 2.4.2
groups several Anlagen behind one Netzanschluss it is the **group's** sum that
has to clear the threshold, so group first.

## Group before you count

Ziff. 2.4.2: where several Anlagen of Fallgruppe b or c sit behind one
Netzanschluss, what matters is whether **their sum** exceeds 4,2 kW, and in that
case *"werden diese gruppierten Anlagen als eine steuerbare
Verbrauchseinrichtung behandelt"*.

So grouping changes both the branch and the count. Three ungrouped 5 kW heat
pumps are three steuVE, none over 11 kW, and take the flat branch; the same
three grouped are one steuVE of 15 kW and take the scaled one. `SteuVe` takes
the figure you hand it, so do the grouping first.

## The parameters are presumptions, not constants

The Festlegung fixes 4,2 kW and 11 kW, and *presumes* 0,4 and the GZF table
appropriate — *"Bis zum Inkrafttreten einer anderweitigen Empfehlung wird die
Angemessenheit vermutet"*. `Para14aConfig` carries the scalars so the arithmetic
survives the day one of them moves.
