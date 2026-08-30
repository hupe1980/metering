+++
title = "The whole pipeline"
description = "One Liefertag from the Zählerstandsgang the gateway delivered through to the §14a register split — a runnable, self-asserting example."
weight = 15
+++

The guides explain each stage on its own. This is all of them, on one day:

```text
Zählerstandsgang ─► Lastgang ─► validate ─► Ersatzwerte ─► Abrechnung
                    (reading)   (validation)  (substitute)   (aggregation
                                                              + zaehlzeit)
```

```bash
cargo run --example pipeline
```

The source is [`examples/pipeline.rs`](https://github.com/hupe1980/metering/blob/main/examples/pipeline.rs).
It asserts its own invariants, so CI runs it as a test — an example that only
prints is a demo; one that asserts is documentation you cannot silently break.

## The day is deliberately awkward

**25 October 2026** is the autumn DST transition: 25 hours long, **100**
quarter-hours. Two defects are planted in the readings:

- a six-digit register that **wraps** past 999 999 partway through the day, and
- a corrupt reading that makes the two spans touching it un-differenceable.

Every stage resolves the day through the calendar rather than assuming a length,
so none of them needs telling that this day is not 96 intervals.

## What it prints

```text
Liefertag 2026-10-25 — 25 h, 100 quarter-hours

1. Zählerstandsgang: 101 readings
2. Lastgang:  98 intervals, 1 rollover(s) reconstructed, 2 anomaly/-ies refused
     ! register decreased, but reconstructing a wrap implies an implausible consumption …
     ! the forward difference exceeds the plausible maximum …
3. Validation: 5 finding(s), 1 blocking
     ✗ V01 gap of 2 interval(s) … — Ersatzwerte required
4. Ersatzwerte: 2 of 100 intervals substituted (98.0 % measured)
     + 2026-10-25 12:45 LinearInterpolation (2 reference value(s))
     + 2026-10-25 13:00 LinearInterpolation (2 reference value(s))

5. Abrechnung
     Arbeitsmenge     22.72 kWh
     Spitzenleistung  1.80 kW at 2026-10-25 6:00 +01:00
     Coverage         100.00 %

6. Zählzeitregister
     HT        5.40 kWh
     NT        2.56 kWh
     ST       14.76 kWh

7. Grade B — 4 findings, 100.00 % coverage
```

Read that as a chain of decisions:

- The **rollover is reconstructed** because a register width was configured; the
  **corrupt reading is refused** because reconstructing *it* as a wrap would
  imply more energy than a 30 kW connection can pass in a quarter-hour. One
  becomes a value, the other becomes a hole.
- The hole surfaces as an ordinary **V01 gap**, not as a special
  "reading anomaly" concept — so Ersatzwertbildung closes it with the same
  machinery it closes any gap, and the audit trail records
  `PlausibilityCheckFailed` as the reason.
- Coverage reaches **100 %** only after filling, and is measured against the
  *declared* period rather than the extent of whatever arrived.
- The register split **reconstructs the Arbeitsmenge exactly** — an invariant
  the example asserts.

## The invariants it checks

```rust
assert_eq!(intervals.len(), 100);                        // the 25-hour day, exactly
assert!((period.coverage_pct - 100.0).abs() < 1e-9);     // nothing left unfilled
assert_eq!(registers.values().sum::<Decimal>(), period.arbeitsmenge);
assert!(!registers.contains_key(&None));                 // the fallback covers everything
assert_eq!(nt_intervals, 36);                            // 8 h of NT plus the repeated hour
```

The last one is the crate's whole thesis in a line. A Niedertarif band of
22:00–06:00 is eight hours, which on an ordinary day is 32 quarter-hours. On
this day the repeated 02:00–03:00 falls inside it, so the answer is **36**. A
fixed 96-interval assumption gets both the day and the band wrong.
