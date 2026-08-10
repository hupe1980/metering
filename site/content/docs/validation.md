+++
title = "Validation and quality"
description = "The order-independent rule engine V01–V12, why the outlier test is a Hampel identifier, and the A/B/C/F grade layered over it."
weight = 4
+++

## The rules

| Rule | ID | Severity | What it catches |
|---|---|---|---|
| Gap | V01 | Error | A missing interval, head and tail included |
| Overlap | V02 | Error | Two intervals covering the same instant |
| Negative energy | V03 | Error | A value below zero on a single-direction meter |
| Statistical outlier | V04 | Warning | A value far from its neighbours, by a robust Hampel test |
| Zero run | V05 | Warning | A run of zeros suggesting a stuck meter |
| Interval length | V06 | Warning | An interval that is not the expected length |
| Collapsed DST hour | V07 | Error | The repeated 02:00–03:00 hour is missing one of its two passes |
| Future timestamp | V08 | Warning | An interval starting after the reference instant |
| Non-billable quality | V09 | Error | `Faulty` or `Unknown` |
| Unordered series | V11 | Warning | Input was not ascending by `from` |
| Implausible power | V12 | Error | Average power above the plant's physical capacity |

**V10 is retired, not recycled.** It was a "register rollover" rule comparing
consecutive interval values for a drop over 50 000 kWh — but a `MeterInterval`
carries interval energy, not a cumulative Zählerstand, so for it to fire a single
quarter-hour would have had to carry 50 MWh (200 MW of average load). Rollover
detection lives in [`reading`](@/docs/readings.md). The number stays unused so a
stored `V10` finding cannot be reinterpreted.

## Order independence

Adjacency rules are evaluated in timestamp order whatever order the caller
supplies, so a shuffled series cannot produce spurious gaps or overlaps. The
disorder itself is reported once as V11, and every `interval_index` still points
into the caller's slice.

## Declare the period, or gaps at the edges are invisible

```rust
use metering::{ValidationConfig, validate_intervals};
# use metering::{MeterInterval, QualityFlag};
# use rust_decimal::dec;
use time::macros::datetime;
# let delivered = vec![MeterInterval {
#     from: datetime!(2026-06-01 0:00 UTC), to: datetime!(2026-06-01 0:15 UTC),
#     value: dec!(2.0), quality: QualityFlag::Measured, obis_code: None }];

// Without a period the data defines its own extent — a truncated delivery is clean.
assert!(validate_intervals(&delivered, &ValidationConfig::default()).is_clean());

// With one, the missing tail is an Error.
let cfg = ValidationConfig::default()
    .over_period(datetime!(2026-06-01 0:00 UTC), datetime!(2026-06-01 2:00 UTC));
assert!(validate_intervals(&delivered, &cfg).has_errors());
```

A month whose last week never arrived validates clean without a declared period.
That is the failure mode that matters most at billing time.

## V07 looks at the repeated hour, not the day

When CEST ends, local 02:00–03:00 happens **twice** — once at UTC+2, once at
UTC+1 — and the two passes occupy a two-hour UTC window either side of the
transition. A series converted from local time without carrying the offset keeps
only one of them, and an hour of energy vanishes.

V07 looks only at that window. An earlier version compared the whole day's
covered duration against 25 hours, which could not tell a collapsed hour from an
ordinary gap: *any* two missing quarter-hours anywhere on a fall-back day
produced a confident report that "the repeated hour was collapsed" — untrue, and
it sent the reader looking in the wrong place.

A gap at midday is now a V01 gap and nothing else. A genuinely collapsed hour is
caught even on a day that is otherwise complete.

`calendar::dst_transition_utc(day)` exposes the anchor if you need to bucket the
repeated hour yourself.

## V04 is robust, and that matters

V04 uses a **Hampel identifier**: a value is an outlier when it deviates from
its local *median* by more than `t × 1.4826 × MAD`. Median and MAD both have a
50 % breakdown point, so up to half a window can be corrupt without moving the
threshold meant to catch it.

The rule this replaced compared each value against the **mean of the whole
series**. The mean includes the spike, so a run of bad values raises its own
threshold and hides itself — and a global mean has no notion of the daily shape,
so quiet hours were judged against a threshold set by busy ones.

### The zero-MAD edge

When more than half a window holds the same value the MAD is exactly zero, and
the test degenerates to "differs from the median at all". On a flat-profile
medium that flags the first genuine draw after a quiet spell, which is what
`outlier_min_sigma` exists to soften. `QualityConfig::for_sparte` sets it per
medium.

## Grading

`score_intervals` runs the validation engine and condenses the findings into one
letter, for callers who must decide "bill or review" and cannot read a list.

| Grade | Condition |
|---|---|
| `A` | no findings, coverage adequate |
| `B` | findings, none blocking — bill it and note it |
| `C` | at most three blocking findings — somebody has to look |
| `F` | more than three blocking findings, or an empty series |

The B/C line is **severity**, not a count: twenty spike warnings still bill, one
gap does not.

The grader does not compute its own statistics. It used to — there were three
scorers with three copies of gap detection, zero-run counting and coverage, each
subtly different from the validation engine's — and a series could grade `A`
while validation reported errors on it. There is now one implementation of each
rule, and a test asserts the two can never disagree.
