+++
title = "Gas conversion and units"
description = "m³ to kWh_Hs under MessEG and DVGW G 685, why the Betriebsvolumen matters, and unit normalisation that refuses to guess."
weight = 7
+++

## The formula and why it is lawful

```text
kWh_Hs = V_m3 × Hs_kWh_per_m3 × Zustandszahl
```

A gas meter registers m³. The kWh billed is **derived, never measured**, so the
conversion rests on the Eichrecht exceptions to § 33 MessEG:

- **§ 33 Abs. 1 MessEG** — a value for a Messgröße may only be used if it was
  determined with a Messgerät.
- **§ 25 Nr. 4 MessEV** — permits Brennwert values *"wenn sie nach den
  anerkannten Regeln der Technik ermittelt worden sind"*.
- **§ 25 Nr. 7 MessEV** — permits a value formed as a *"Produkt"* of measured
  values, which `V × Z × Hs` is.
- **DVGW G 685** — the anerkannte Regel der Technik § 25 Nr. 4 refers to.

## Pass a Betriebsvolumen, not a Normvolumen

`7-0:13.2.0` (Normvolumen umgewertet) and `7-0:3.2.0` (Normvolumen gemessen)
have **already** been state-converted by the Mengenumwerter. Feeding one in
applies the Zustandszahl a second time and overstates the energy by the
Zustandszahl's deviation from 1 — a few percent, silently, on a billed quantity.

For an already-converted volume, pass `Decimal::ONE` as the Zustandszahl.

## G 685 rounding is a configuration choice

```rust
use metering::{G685FinalRounding, G685Rounding, gas_m3_to_kwh_hs_rounded};
use rust_decimal::dec;

// The published eneregio worked example: 895 m³ × 11.369 × 0.9543 → 9 710 kWh.
let kwh = gas_m3_to_kwh_hs_rounded(
    dec!(895), dec!(11.369), dec!(0.9543),
    G685Rounding { final_rounding: G685FinalRounding::WholeKwh, ..G685Rounding::default() },
);
assert_eq!(kwh, dec!(9710));
```

Input rounding is consistent across published Netzbetreiber Merkblätter —
Zustandszahl to four decimal places, Abrechnungsbrennwert to three. The **final**
rounding demonstrably diverges (both whole-kWh and two-decimal results appear in
published Merkblätter) and the normative text is not freely citable, so it is a
setting rather than a hard-coded claim.

## Unit normalisation refuses to guess

```rust
use metering::{GasConversionParams, normalize_to_kwh};
use rust_decimal::dec;

let gas = GasConversionParams { hs_kwh_per_m3: dec!(10.55), zustandszahl: dec!(0.98) };
assert_eq!(normalize_to_kwh(dec!(100), "m3", Some(&gas), None)?, dec!(1033.900));
assert_eq!(normalize_to_kwh(dec!(3.6), "GJ", None, None)?, dec!(1000)); // exactly
assert_eq!(normalize_to_kwh(dec!(48), "kW", None, Some(900))?, dec!(12));

// An unknown unit is an error, not a silent pass-through as kWh.
assert!(normalize_to_kwh(dec!(1), "furlong", None, None).is_err());
# Ok::<(), metering::conversion::ConversionError>(())
```

`MeasurementUnit::parse_scaled` accepts device symbols (kWh, Wh, MWh, GJ, MJ,
m³, litres) and the UN/ECE Rec 20 codes UTILMD and EN 16931 use — where the
codes are *not* the symbols: gigajoule is `GV`, megajoule is `3B`, cubic metre
is `MTQ`.

Each factor is kept as an exact rational rather than a decimal. 1 GJ is
2500/9 kWh, so 3.6 GJ is exactly 1000 kWh with no residue — multiplying before
dividing rounds once, at the end, instead of once per reading.

## Warm water under HeizkostenV § 9 Abs. 2

```text
Q [kWh/a] = 2.5 × V [m³] × (t_w [°C] − 10)
```

A *Zahlenwertgleichung* — not dimensionally consistent, so 2.5 carries no unit.
§ 9 Abs. 2 Satz 3 Nr. 1 defines it as covering the Erzeugeraufwandszahl, the
specific heat capacity of water, storage and circulation losses, and metering
effort. Because the Erzeugeraufwandszahl is inside the constant, **Q is
generator-input heat, not delivered useful heat**.

This equation is the fallback admitted only where measuring with a Wärmezähler
would take *"unzumutbar hohen Aufwand"*. The floor-area variant
(`32 × A_Wohn`) is narrower still: it needs neither the heat quantity nor the
volume to be measurable, and its constant covers a *different* bundle of losses.
