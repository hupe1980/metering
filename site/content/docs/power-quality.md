+++
title = "Power quality"
description = "EN 50160 is a statistical standard over a week of 10-minute means, not a per-interval threshold — and what that changes."
weight = 11
+++

It is tempting to write `voltage > 253 V → non-compliant`. EN 50160 says no such
thing. Every one of its limits is a **share of 10-minute mean values over an
observation window**:

| Parameter | Limit | Share | Window |
|---|---|---|---|
| Supply voltage | `Un ± 10 %` | 95 % | one week |
| Supply voltage | `Un + 10 % / − 15 %` | 100 % | one week |
| Frequency | `50 Hz ± 1 %` | 99.5 % | one year |
| Frequency | `50 Hz + 4 % / − 6 %` | 100 % | one year |
| THD of voltage | `≤ 8 %` | 95 % | one week |

A week of 10-minute means is 1 008 samples, and **up to 50 of them may sit
outside `Un ± 10 %` with the supply still conforming**. A single interval above
253 V is not a breach, and reporting it as one produces alarms that are
individually true and collectively meaningless.

## The assessment

```rust
use metering::power_quality::{En50160Limits, assess_en50160};
# use metering::power_quality::PowerQualityInterval;
# use rust_decimal::dec;
# use time::{Duration, macros::datetime};
# let mut series: Vec<PowerQualityInterval> = (0..1008).map(|i| {
#     let from = datetime!(2026-06-01 0:00 UTC) + Duration::minutes(i * 10);
#     PowerQualityInterval { voltage_l1_v: Some(dec!(231)),
#         ..PowerQualityInterval::empty(from, from + Duration::minutes(10)) }
# }).collect();
# series[500].voltage_l1_v = Some(dec!(260));
let report = assess_en50160(&series, &En50160Limits::LOW_VOLTAGE);

assert!(report.is_conclusive());             // a full week was supplied
assert!(report.voltage_band.compliant);      // 1 in 1 008 is inside the 5 % allowance
assert!(!report.voltage_absolute.compliant); // ...but +10 % admits no exceptions
```

`compliant()` and `is_conclusive()` are separate questions. A verdict over three
hours of data is not an EN 50160 statement, and the report says so rather than
quietly claiming conformance.

Each phase counts as its own sample — the limits apply per phase, so a
three-phase week is 3 024 voltage samples. A parameter nobody measured is
excluded from the verdict rather than counted as a pass.

## Per-interval predicates are triage, not conformance

`voltage_out_of_range`, `frequency_out_of_range` and friends remain, and are
useful for spotting a sample worth looking at. They are documented as
indicators, not as an EN 50160 verdict.

## Not assessed, deliberately

- **Voltage unbalance.** EN 50160 limits the negative-sequence ratio
  `u₂ = U₂/U₁` to 2 % for 95 % of a week. Computing `U₂` needs the phase
  *angles*, and a meter reporting three RMS magnitudes has not supplied them.
  The magnitude-only approximations in circulation answer a different question.
- **Flicker (`Plt`), dips, swells, interruptions and harmonics by order.** These
  need waveform-level measurement, not interval means.

The limits are data, not constants: `En50160Limits::LOW_VOLTAGE` carries the
230 V figures, and medium or high voltage is a different value of the same type.

## Unsymmetrie is a different question, and this one *is* answered

The voltage unbalance above needs phase angles and is refused. **Load**
unbalance does not: VDE-AR-N 4100 Abschnitt 5.5.2 limits the
Unsymmetrieleistung of the devices in a customer installation to **4,6 kVA**,
and that is computable from three magnitudes.

VDE FNN's *Symmetrischer Anschluss und Betrieb in Kundenanlagen* explains where
the number comes from: EN 50160 caps the Unsymmetrie der Versorgungsspannung at
2 %, and *"zur Einhaltung dieser Symmetriegrenze der Versorgungsspannung wurde
bei einem Außenleiterstrom von 20 A ein Leistungsgrenzwert von 4,6 kVA
festgelegt"*. So the two limits are the same limit twice.

```rust
use metering::power_quality::{Phase, PhaseApparentPower};
use rust_decimal::dec;

// A 22 kVA wallbox charging single-phase at 7,2 kVA needs a Symmetrieeinrichtung.
let single = PhaseApparentPower::single_phase(Phase::L1, dec!(7.2));
assert_eq!(single.unbalance_kva(), dec!(7.2));
assert!(!single.within_limit(None));
assert_eq!(single.excess_kva(None), dec!(2.6));

// Three 4,6 kVA units, one per Außenleiter: 13,8 kVA installed, balanced.
let spread = PhaseApparentPower::default()
    .plus(Phase::L1, dec!(4.6))
    .plus(Phase::L2, dec!(4.6))
    .plus(Phase::L3, dec!(4.6));
assert_eq!(spread.unbalance_kva(), dec!(0.0));
```

### kVA, not kW — and that is not a detail

The rule is stated in **Scheinleistung**. An inverter running at cos φ < 1,
which VDE-AR-N 4105 requires it to be able to do for
Blindleistungsbereitstellung, moves more kVA than kW: a guard written on active
power passes installations that breach the rule, and the error grows exactly
when the grid asked for reactive support.

### It is not what the grid meter sees

Abschnitt 5.5.2 applies *"nur für Geräte die elektrische Energie einspeisen oder
speichern können, also Erzeugungsanlagen, Speicher, Ladeeinrichtungen für
Elektrofahrzeuge"*. Ordinary household load is outside it, so a `MeterInterval`
— an energy accumulated at the Netzanschluss — cannot answer the question
however many phases it carried. Sum those three device classes per Außenleiter
and pass that. A three-phase symmetric device contributes nothing whatever its
size, which is `PhaseApparentPower::symmetric`.

VDE-AR-N 4100 is a paywalled Anwendungsregel, so the 4,6 kVA limit is a
parameter with a documented default — FNN itself notes it *"soll im Rahmen einer
FNN-Studie untersucht werden"* — and the max-minus-min reading is the arithmetic
the Hinweis's worked examples demonstrate rather than a quoted formula.
