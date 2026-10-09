+++
title = "Power quality"
description = "EN 50160 as the statistical test it is — per ISO week and per phase — and the VDE-AR-N 4100 Unsymmetrieleistung of a customer installation."
weight = 11
[extra]
group = "Specialised"
+++

## EN 50160

Every limit is a **share of 10-minute means over a window**, not a threshold
on one sample.

| Parameter | Limit | Share | Window |
|---|---|---|---|
| Supply voltage | `Un ± 10 %` | 95 % | each week, each phase |
| Supply voltage | `Un + 10 % / − 15 %` | 100 % | each week, each phase |
| Frequency | `50 Hz ± 1 %` | 99,5 % | one year |
| THD of voltage | `≤ 8 %` | 95 % | each week |

The standard is paywalled, so nothing is quoted: the figures are
`En50160Limits::LOW_VOLTAGE` (`Un = 230 V`), each settable; other voltage levels
are other values of the type. `assess_en50160` reports **per ISO week** (Monday
00:00 Berlin, 167–169 hours) **and per phase**, never pooled.

```rust
use metering::grid::power_quality::{En50160Limits, En50160Verdict, PowerQualityInterval, assess_en50160};
use rust_decimal::dec;
use time::{Duration, macros::datetime};

// ISO week 23 of 2026: Monday 1 June 00:00 CEST is Sunday 22:00 UTC.
let start = datetime!(2026-05-31 22:00 UTC);
let mut week: Vec<PowerQualityInterval> = (0..1008)
    .map(|i| {
        let from = start + Duration::minutes(i * 10);
        PowerQualityInterval {
            voltage_l1_v: Some(dec!(231)),
            ..PowerQualityInterval::empty(from, from + Duration::minutes(10))
        }
    })
    .collect();
week[500].voltage_l1_v = Some(dec!(260)); // one sample over +10 %

let report = assess_en50160(&week, &En50160Limits::LOW_VOLTAGE);
let l1 = &report.weeks[0].phases[0];
assert_eq!(l1.voltage_band.compliant(), Some(true)); // 1 in 1 008 is within 5 %
assert_eq!(l1.voltage_absolute.compliant(), Some(false)); // +10 % admits none
assert_eq!(report.verdict(), En50160Verdict::NonCompliant);
```

- `LimitOutcome::compliant()` is `None` for an unmeasured parameter; the
  verdict is `Unknown` when a week is incomplete.
- `worst` is the sample furthest beyond the **bound**, not from nominal.
- `PowerQualityInterval::voltage_out_of_range` and friends are single-sample
  triage, not a conformance test.

**Not assessed:** voltage unbalance (`u₂ = U₂ / U₁` needs phase angles), flicker,
dips, swells, interruptions, harmonics by order.

## Unsymmetrieleistung — VDE-AR-N 4100

VDE-AR-N 4100 Abschnitt 5.5.2 limits a customer installation's
Unsymmetrieleistung to **4,6 kVA** — VDE FNN *Symmetrischer Anschluss und
Betrieb in Kundenanlagen*: *"zur Einhaltung dieser Symmetriegrenze der
Versorgungsspannung wurde bei einem Außenleiterstrom von 20 A ein
Leistungsgrenzwert von 4,6 kVA festgelegt"*.

```rust
use metering::grid::power_quality::{Phase, PhaseApparentPower};
use rust_decimal::dec;

// A 22 kVA wallbox charging single-phase at 7,2 kVA breaches it.
let single = PhaseApparentPower::single_phase(Phase::L1, dec!(7.2));
assert!(!single.within_limit(None));
assert_eq!(single.excess_kva(None), dec!(2.6));

// Three 4,6 kVA units, one per Außenleiter: balanced.
let spread = PhaseApparentPower::default()
    .plus(Phase::L1, dec!(4.6))
    .and_then(|p| p.plus(Phase::L2, dec!(4.6)))
    .and_then(|p| p.plus(Phase::L3, dec!(4.6)))
    .unwrap();
assert_eq!(spread.unbalance_kva(), dec!(0.0));
```

- **kVA, not kW.**
- **Device classes, not the grid meter**: the rule applies *"nur für Geräte die
  elektrische Energie einspeisen oder speichern können, also
  Erzeugungsanlagen, Speicher, Ladeeinrichtungen für Elektrofahrzeuge"*. Sum
  those per Außenleiter; a symmetric device contributes nothing
  (`PhaseApparentPower::symmetric`).
- 4,6 kVA is a parameter (`UNSYMMETRIE_LIMIT_KVA`); max − min is the arithmetic
  of the Hinweis's worked examples.
