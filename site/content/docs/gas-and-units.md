+++
title = "Gas conversion and units"
description = "m³ to kWh_Hs under MessEG and DVGW G 685, the SigLinDe gas SLP arithmetic, the 06:00 Gastag, and unit normalisation that refuses to guess."
weight = 8
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

## The gas SLP — SigLinDe, published in full

Unlike the 2025 electricity profiles, whose value tables are licensed, the gas
SLP procedure is published in full: the BDEW/VKU/GEODE Leitfaden *"Abwicklung
von Standardlastprofilen Gas"* — current edition **KoV XV, Stand 27.03.2026**,
coefficients in Anlage 6 — prints the profile function, the temperature
weighting, the weekday factors and every coefficient set. `metering::gas_slp`
implements that arithmetic:

```text
f_sigmoid(ϑ) = A / (1 + (B / (ϑ − ϑ₀))^C) + D          ϑ₀ = 40 °C
f_linear(ϑ)  = max{ mH·ϑ + bH ;  mW·ϑ + bW }
h(ϑ)         = f_sigmoid(ϑ) + f_linear(ϑ)

Q(D) = KW · h(ϑ_D) · F_WT
```

```rust
use metering::gas_slp::{SigLinDe, allocation_temperature};
use metering::{gas_daily_quantity, kundenwert};
use rust_decimal::dec;

// The temperature entering h is a geometric series over four days — the
// heat stored in buildings — and the division is exact (weights are eighths).
let theta = allocation_temperature(dec!(5.0), dec!(2.5), dec!(2.5), dec!(5.0));
assert_eq!(theta, dec!(4));

// DE_HEF34 is the published single-family-home reference set, normalised so
// h(8 °C) = 1.00000 — which the test suite reproduces from the printed row.
let h = SigLinDe::DE_HEF34.h_value(theta);
let q = gas_daily_quantity(dec!(60.3423), h, dec!(1));
assert!(q > dec!(90));
# let _ = kundenwert(dec!(1), dec!(1));
```

The **Kundenwert** — the customer's consumption on a day where `h = 1` — comes
from a metered reference period as `KW = Q / Σ(h·F)`, weekday factors must sum
to exactly 7.0000 for the standard week, and a gesetzlicher Feiertag takes the
Sunday factor, nationwide by default and per-Land on request — all as the
Leitfaden specifies. The pure-sigmoid form — zero linear parts — is both the
pre-2015 TUM generation and how HKO, the Kochgasprofil, is published today.

### The fifteen profile types

The Leitfaden publishes **fifteen** gas profile types in two variants each
(`33` and `34`, differing in how much of the demand the linear part carries):
`HEF`, `HMF`, `HKO` for households, eleven Gewerbe sector types, and `GHD`.

`GHD` is the *Summenlastprofil Gewerbe, Handel, Dienstleistung* — the EDI@Energy
*Codeliste TUM- und BDEW-SLP Gas* v1.1 §6.3 lists it under the TUM codes `HD3`
and `HD4`, and `GHD` is its BDEW/SigLinDe short code, formed as `G` + the TUM
stem like `GMF` from `MF`. Its coefficients and weekday factors are a weighted
mean across the sector types, and a delivery point takes it when it fits none of
them.

```rust
use metering::LoadProfile;

let ghd = LoadProfile::parse("GHD").expect("a real profile");
assert!(ghd.is_gas() && ghd.is_commercial());
assert!(ghd.is_gas_aggregate(), "the only aggregate of the fifteen");
assert_eq!(LoadProfile::ALL.iter().filter(|p| p.is_gas()).count(), 15);
```

## The Gastag runs 06:00 to 06:00

Gas is balanced on **gas days**, not calendar days: a Gastag runs from 06:00
local to 06:00 the next morning. Summing a gas Lastgang over the calendar day
books the 00:00–06:00 draw into the wrong Bilanzierungstag — six hours, every
day. `calendar::gas_day_start_utc`, `gas_day_end_utc` and `local_gas_day` own
the boundary.

One consequence worth knowing: the clocks change at 02:00/03:00 local, *before*
the 06:00 boundary — so the 23- or 25-hour Gastag is the one named after the
**Saturday**, not the transition Sunday.

The boundary is not something you have to re-derive at each call site.
`DayBoundary::Gastag` moves a whole daily, monthly or yearly grid onto it:

```rust
use metering::{MeterInterval, QualityFlag, ResampleConfig, calendar};
use rust_decimal::dec;
use time::{Duration, macros::date};

// Two whole Gastage of hourly gas intervals.
let start = calendar::gas_day_start_utc(date!(2026 - 01 - 15));
let series: Vec<MeterInterval> = (0..48).map(|i| MeterInterval {
    from:  start + Duration::hours(i),
    to:    start + Duration::hours(i + 1),
    value: dec!(1),
    quality: QualityFlag::Measured,
    obis_code: None,
}).collect();

let gas_days = metering::resample(&series, &ResampleConfig::to_gas_daily());
assert_eq!(gas_days.len(), 2);
assert_eq!(gas_days[0].total, dec!(24));
assert_eq!(gas_days[0].is_complete(), Some(true));

// The same data on calendar days is three partial buckets, the first holding
// only 18:00–24:00.
let calendar_days = metering::resample(
    &series,
    &ResampleConfig::new(metering::IntervalResolution::Hour, metering::IntervalResolution::Day),
);
assert_eq!(calendar_days.len(), 3);
assert!(calendar_days[0].has_missing_data());
```

`FillGapsConfig::on` takes the same boundary, so Ersatzwertbildung on a gas SLP
walks Gastage rather than Liefertage.

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
