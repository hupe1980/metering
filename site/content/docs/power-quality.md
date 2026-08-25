+++
title = "Power quality"
description = "EN 50160 is a statistical standard over a week of 10-minute means, not a per-interval threshold — and what that changes."
weight = 9
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
