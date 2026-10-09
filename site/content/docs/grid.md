+++
title = "Grid: § 14a, Ausfallarbeit, rollout"
description = "The § 14a EnWG Mindestleistung with the Ziff. 2.4.2 grouping and the netzwirksamer Leistungsbezug; Redispatch 2.0 Ausfallarbeit including the fluctuating Spitzabrechnung; the § 29 MsbG rollout duty."
weight = 10
aliases = ["/docs/paragraph-14a/", "/docs/redispatch/"]
[extra]
group = "Specialised"
+++

## § 14a EnWG — netzorientierte Steuerung

BNetzA **BK6-22-300** (27.11.2023, in force 01.01.2024): a Netzbetreiber may
reduce the netzwirksamer Leistungsbezug of steuerbare Verbrauchseinrichtungen
(steuVE) down to a floor, `P_min,14a`, both in kW. Modul 3's tariff windows are
under [Billing quantities](@/docs/billing-quantities.md#modul-3).

### Which devices are one steuVE

Anlage 1 Ziff. 2.4.2:

> Abweichend von Ziffer 2.4.1. ist in den Fallgruppen der Ziffern 2.4.1.b.
> und 2.4.1.c. beim Vorhandensein mehrerer Anlagen hinter einem
> Netzanschluss jeweils maßgeblich, ob die Summe der Netzanschlussleistungen
> aller Anlagen insgesamt 4,2 kW je Fallgruppe überschreitet.

All Wärmepumpen behind one Netzanschluss are **one** steuVE of their summed
power, all Raumkühlung another; a Ladepunkt or Stromspeicher is its own when it
alone exceeds 4,2 kW. `steuerbare_verbrauchseinrichtungen` groups;
`mindestleistung_ems` groups, then applies Ziff. 4.5.2's
Gleichzeitigkeitsfaktor.

```rust
use metering::grid::para14a::{Anlage, Para14aConfig, SteuVeFallgruppe as F, mindestleistung_ems};
use rust_decimal::dec;

let cfg = Para14aConfig::VERMUTUNG;

// Three 5 kW heat pumps are one 15 kW steuVE: scaled, 0,4 × 15 = 6 kW.
let three_heat_pumps = [Anlage::new(F::Waermepumpe, dec!(5)); 3];
assert_eq!(mindestleistung_ems(&three_heat_pumps, &cfg), Some(dec!(6.0)));

// Two 3 kW heat pumps are one 6 kW steuVE below 11 kW: the flat 4,2 kW.
let two_small = [Anlage::new(F::Waermepumpe, dec!(3)); 2];
assert_eq!(mindestleistung_ems(&two_small, &cfg), Some(dec!(4.2)));
```

`Para14aConfig::VERMUTUNG` holds 0,4 and the Gleichzeitigkeitsfaktor table,
presumptions *"bis zum Inkrafttreten einer anderweitigen Empfehlung"*; both are
settable.

### The netzwirksamer Leistungsbezug

Ziff. 2.3 defines the steuVE's share of the draw but not how to split a draw
local generation partly covers, so you choose a `Verursachungsregel`:
`SteuVeZuletzt` (steuVE served last — conservative) or `Anteilig` (pro rata,
which needs the rest of the installation's draw).

```rust
use metering::grid::para14a::{Verursachungsregel, netzwirksamer_leistungsbezug};
use rust_decimal::dec;

// 10 kW at the Netzanschluss, 6 kW of it steuVE.
assert_eq!(
    netzwirksamer_leistungsbezug(dec!(10), dec!(6), None, Verursachungsregel::SteuVeZuletzt),
    Some(dec!(6)),
);
assert_eq!(
    netzwirksamer_leistungsbezug(dec!(10), dec!(6), None, Verursachungsregel::Anteilig),
    None,
);
```

Without a sub-measurement (optional per Ziff. 4.7), the device's
Netzanschlussleistung is the conservative substitute.

## Redispatch 2.0 — Ausfallarbeit

BNetzA **BK6-23-241** (Beschluss 07.05.2026), Anlage *BilAReM* Kap. 3,
applicable from 01.10.2026. Per quarter-hour and Technische Ressource, signed:
*"bei negativem Redispatch ist die Ausfallarbeit positiv, bei positivem
Redispatch ist die Ausfallarbeit negativ (Mehrarbeit)"*.

| Abrechnungsvariante | Function |
|---|---|
| Spitzabrechnung, fluctuating (Wind Kap. 3.2.2.1, Solar 3.2.4.1) | `spitz_fluktuierend_kwh` |
| vereinfachte Spitzabrechnung (3.2.2.2, 3.2.4.2) | the same formula, weather data from a provider or a reference plant |
| Spitzabrechnung, nicht-fluktuierend (3.3.1) | `spitz_nicht_fluktuierend_kwh` |
| Pauschal-Abrechnung (3.2.2.3, 3.2.4.3, 3.3.2) | `pauschal_wind_kwh`, `pauschal_solar_kwh`, `pauschal_nicht_fluktuierend_kwh` |

`leistungslimitierung_kw` gives `P_lim` for the Aufforderungs- and
Duldungsfall; `p0_kw` the last fully measured quarter-hour before the measure.

### The fluctuating Spitzabrechnung

Theoretical power: wind `KF × P_theo` (the power curve at the measured wind
speed, `P_theo` your input, corrected by the four reference quarter-hours);
solar `P_VZ,ist ÷ G_VZ × G_i`, bounded by the inverter. Both are capped at the
Nennleistung (a larger product is *"nicht plausibel"*), bounded by any
marktbedingte Anpassung or Nichtbeanspruchbarkeit, and cut once to
`AUSFALLARBEIT_DP`.

```rust
use metering::grid::ausfallarbeit::{Fluktuierend, spitz_fluktuierend_kwh};
use rust_decimal::dec;

// A 3 MW turbine: the curve says 2 000 kW, the reference quarter-hours ran at
// 90 % of theirs (KF = 0,9), and the measure capped it at 600 kW.
let wind = Fluktuierend::Wind { p_theo_kw: dec!(2000), p_vz_ist_kw: dec!(1800), p_vz_theo_kw: dec!(2000) };
assert_eq!(
    spitz_fluktuierend_kwh(wind, dec!(3000), None, None, dec!(600)),
    Some(dec!(300.000)), // (0,9 × 2 000 − 600) × ¼ h
);
```

The Pauschal-Abrechnung ends on 31.12.2028 (`PAUSCHAL_BESTANDSSCHUTZ_ENDE`);
the Beschluss finds it *"nicht geeignet, die Ausfallarbeit von
Redispatch-Maßnahmen mit diesen Anlagen ausreichend genau zu bestimmen"*. The
solar Pauschal's Anlagenfaktor table (`anlagenfaktor`) is in UTC+1 all year.

## Rollout — § 29 and § 45 MsbG

`classify_rollout_obligation` returns every § 29 Abs. 1 ground that applies
(cumulative, strict thresholds): consumption above 6 000 kWh (mean of the last
three years, § 30 Abs. 4, or the Netzbetreiber's forecast), a § 14a agreement,
a plant above 7 kW. The Steuerungseinrichtung belongs to Nr. 2 as a whole;
Abs. 5 lifts it — only it — for a permanent 0 % feed-in declared in Textform.

```rust
use metering::grid::rollout::{FeedInWaiver, Jahresverbrauch, Messstelle, RolloutObligation, classify_rollout_obligation};
use rust_decimal::dec;

let roof = Messstelle::new(Jahresverbrauch::Prognose(dec!(2000))).installierte_leistung(dec!(12));
let a = classify_rollout_obligation(&roof);
assert_eq!(a.primary(), RolloutObligation::PflichtGeneration);
assert!(a.quota_conditional);

let waived = classify_rollout_obligation(&roof.einspeiseverzicht(FeedInWaiver::GRANTED));
assert!(waived.imsys_required());
assert!(!waived.steuerungseinrichtung_required);
```

Nr. 2b depends on the Messstellenbetreiber's portfolio, so it is reported as
`quota_conditional`, not decided. Everything else owes a moderne
Messeinrichtung by 31.12.2032 (`MME_DEADLINE`).
