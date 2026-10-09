+++
title = "Gas, units and load profiles"
description = "m³ to kWh_Hs per DVGW G 685, the Zustandszahl, unit normalisation that refuses to guess, and the standard load profiles: BDEW SLP Strom and SigLinDe SLP Gas."
weight = 8
[extra]
group = "Tasks"
+++

## m³ → kWh_Hs

`kWh_Hs = V_Betrieb [m³] × Hs [kWh/m³] × Zustandszahl` (DVGW G 685). Billing a
derived value rests on § 25 Nr. 4 and Nr. 7 MessEV — see the [regulatory
basis](@/docs/regulatory-basis.md#almost-nothing-here-is-a-measured-value).

```rust
use metering::gas::conversion::{GasConversionParams, gas_m3_to_kwh_hs, normalize_to_kwh};
use rust_decimal::dec;

// 100 m³ × 10.55 kWh/m³ × 0.9764, exactly.
assert_eq!(gas_m3_to_kwh_hs(dec!(100), dec!(10.55), dec!(0.9764)), Some(dec!(1030.102000)));

// Through the unit-aware path: a volume needs both parameters, and has no default.
let params = GasConversionParams::new(dec!(10.55), dec!(0.98));
assert_eq!(normalize_to_kwh(dec!(100), "m3", Some(&params), None).unwrap(), dec!(1033.900));
assert!(normalize_to_kwh(dec!(100), "m3", None, None).is_err());
```

**Pass a Betriebsvolumen** (OBIS `7-0:3.0.0`). A Normvolumen (`7-0:13.2.0`) is
already state-converted; use `GasConversionParams::already_converted(hs)`,
which fixes the Zustandszahl at 1. `GasConversionParams` has no `Default`.

### The Zustandszahl

From the Netzbetreiber's Höhenzonen table, or computed from the DVGW G 685-3
inputs: air pressure (`p_amb = 1016 − 0,12 × H`), Effektivdruck,
Abrechnungstemperatur (15 °C) and Kompressibilitätszahl (1 below one bar).
Normzustand per DIN 1343: 273,15 K, 1013,25 mbar.

```rust
use metering::gas::conversion::{
    G685Rounding, ZustandszahlParams, gas_m3_to_kwh_hs_rounded, hoehenzonen_luftdruck_mbar, zustandszahl,
};
use metering::precision::G685_STRATEGY;
use rust_decimal::dec;

// A household connection 253 m above sea level, 22 mbar Effektivdruck.
let params = ZustandszahlParams::below_one_bar(hoehenzonen_luftdruck_mbar(dec!(253)).unwrap(), dec!(22)).unwrap();
let z = zustandszahl(&params).unwrap();
assert_eq!(z.round_dp_with_strategy(4, G685_STRATEGY), dec!(0.9427));

let kwh = gas_m3_to_kwh_hs_rounded(dec!(1874), dec!(11.316), z, G685Rounding::PUBLISHED_PRACTICE).unwrap();
assert_eq!(kwh.round_dp_with_strategy(2, G685_STRATEGY), dec!(19991.07));
```

**The final rounding is a setting**: Merkblätter diverge between whole kWh and
two decimals. `G685Rounding::PUBLISHED_PRACTICE` rounds the Zustandszahl to four
and the Brennwert to three places and leaves the result unrounded.

## Unit normalisation

`normalize_to_kwh(value, unit, gas, interval_secs)`: an energy unit is
rescaled exactly, a power needs its averaging interval, a volume the gas
parameters; an unknown unit is an error.

```rust
use metering::gas::conversion::normalize_to_kwh;
use rust_decimal::dec;

assert_eq!(normalize_to_kwh(dec!(3.6), "GJ", None, None).unwrap(), dec!(1000));
assert_eq!(normalize_to_kwh(dec!(48), "kW", None, Some(900)).unwrap(), dec!(12));
assert!(normalize_to_kwh(dec!(1), "furlong", None, None).is_err());
```

## SLP Strom

The BDEW profiles are **value tables** the operator loads — the 2025 revision
(H25, G25, L25, P25, S25) or the 1999 VDEW profiles; the crate ships none. BDEW
*Hinweise zu den aktualisierten Standardlastprofilen Strom* (17.03.2025):
*"Jedem Netzbetreiber steht es weiterhin frei, bei der Bilanzierung auf die
aktualisierten Profile aus dem Jahr 2025, die alten Profile aus dem Jahr 1999,
eigene Profile oder eine Mischung der verschiedenen Optionen zurückzugreifen."*

`DynamicSlpProfile::value_at(instant, &calendar)` resolves the Berlin
quarter-hour, month, day type in the Land and Dynamisierung, and answers `None`
when a table is missing.

- **Day type** from `SlpCalendar`: 24 and 31 December take the Saturday
  profile unless a Sunday (VDEW definition; `eves_as_saturday(false)`).
- **Dynamisierung**: `Dynamization::BDEW`, the H0/H25 quartic in exact
  `Decimal`, refused outside day 1..=366.

```rust
use metering::slp::strom::{DynamicSlpProfile, Dynamization, LoadProfile, SlpDayType};
use metering::time::holiday::{Bundesland, SlpCalendar};
use rust_decimal::dec;
use time::macros::datetime;

let mut h25 = DynamicSlpProfile::new(LoadProfile::H25).dynamization(Dynamization::BDEW);
h25.insert(6, SlpDayType::SonnFeiertag, vec![dec!(100); 96]).unwrap();

// Fronleichnam 2026 is a holiday in Bavaria, a Werktag in Berlin.
let noon = datetime!(2026-06-04 10:00 UTC);
assert!(h25.value_at(noon, &SlpCalendar::new(Bundesland::By)).is_some());
assert!(h25.value_at(noon, &SlpCalendar::new(Bundesland::Be)).is_none()); // Werktag table not loaded
```

## SLP Gas — SigLinDe

One value per Gastag from temperature — Leitfaden *Abwicklung von
Standardlastprofilen Gas* (KoV XV, Stand 27.03.2026, Anlage 6):

```text
h(ϑ)  = A / (1 + (B / (ϑ − ϑ₀))^C) + D  +  max{ mH·ϑ + bH ; mW·ϑ + bW }
Q(D)  = Kundenwert · h(ϑ_allok) · F_WT
ϑ_allok = (ϑ_D + 0,5·ϑ_D−1 + 0,25·ϑ_D−2 + 0,125·ϑ_D−3) / 1,875
```

```rust
use metering::slp::gas::{SigLinDe, allocation_temperature, gas_daily_quantity};
use rust_decimal::dec;

// The Leitfaden's worked example: −0,2399 °C.
let theta = allocation_temperature(dec!(-2.0), dec!(0.5), dec!(3.4), dec!(3.6)).unwrap();
assert_eq!(theta, dec!(-0.23991));

let h = SigLinDe::DE_HEF34.h_value(dec!(4)).unwrap();
let q = gas_daily_quantity(dec!(60.3423), h, dec!(1)).unwrap();
assert!(q > dec!(90));
```

`h_value` evaluates the sigmoid in `f64` (the [one float
crossing](@/docs/design.md#exactness)) and returns an unrounded `Decimal`, as
Anlage 5 requires. `SigLinDe::DE_HEF34` is embedded to verify against the
printed numbers; operators load the sets they balance with. The
allocation-temperature weights use four decimals, rounded *mathematisch* (half
to even).
