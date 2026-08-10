+++
title = "Readings and intervals"
description = "Zählerstandsgang to Lastgang: differencing meter readings, reconstructing register wraps safely, and why the conversion never invents a value."
weight = 3
+++

A meter counts upwards. `MeterInterval` is the energy *in* a period — a derived
quantity — and what a register actually holds is a cumulative **Zählerstand**.

Since **6 June 2025** the differencing is the Messstellenbetreiber's job by
regulation. BNetzA **BK6-24-174** is titled *"Datenübermittlung ZSG"*:

```text
Smart-Meter-Gateway ──Zählerstandsgang──► MSB ──Lastgang──► NB, Lieferant
                       (metering::reading)  └── differencing happens here
```

§ 2 Satz 1 Nr. 27 MsbG defines the input verbatim:

> die Messung einer Reihe viertelstündig ermittelter Zählerstände von
> elektrischer Arbeit und stündlich ermittelter Zählerstände von Gasmengen

Note the two resolutions — electricity quarter-hourly, gas hourly.

## Differencing a series

```rust
use metering::reading::{LastgangConfig, MeterReading, to_lastgang};
use rust_decimal::dec;
use time::macros::datetime;

let zsg: Vec<MeterReading> = [dec!(1000.0), dec!(1002.5), dec!(1004.8)]
    .into_iter()
    .enumerate()
    .map(|(i, v)| MeterReading::measured(
        datetime!(2026-06-01 0:00 UTC) + time::Duration::minutes(i as i64 * 15), v))
    .collect();

let lastgang = to_lastgang(&zsg, &LastgangConfig::strom());
assert_eq!(lastgang.intervals.len(), 2);  // n readings give n−1 intervals
assert_eq!(lastgang.intervals[0].value, dec!(2.5));
```

The derived intervals are labelled `1-0:1.29.0` — a Lastgang is a different OBIS
channel from the Zählerstand it came from (D = 29, not D = 8).

## Register rollover belongs here

A six-digit Zählwerk counts to 999 999 and returns to zero. That is a property
of a **register**, so it can only be detected where readings live.

```rust
use metering::reading::{LastgangConfig, MeterReading, to_lastgang};
use rust_decimal::dec;
use time::macros::datetime;

let zsg = vec![
    MeterReading::measured(datetime!(2026-06-01 0:00 UTC), dec!(999998.5)),
    MeterReading::measured(datetime!(2026-06-01 0:15 UTC), dec!(1.5)), // wrapped
];

// Without a register width the drop is unexplainable, so nothing is invented.
let blind = to_lastgang(&zsg, &LastgangConfig::strom());
assert!(blind.intervals.is_empty());
assert_eq!(blind.anomalies.len(), 1);

// With one, the wrap is reconstructed: (1 000 000 − 999 998.5) + 1.5 = 3.
let cfg = LastgangConfig::strom().with_register_digits(6);
let wrapped = to_lastgang(&zsg, &cfg);
assert_eq!(wrapped.intervals[0].value, dec!(3.0));
assert_eq!(wrapped.rollovers.len(), 1);
```

Two safeguards make reconstruction safe rather than dangerous:

- **No register width, no reconstruction.** Guessing the width wrong turns a
  meter exchange into a million kWh.
- **A plausibility cap.** A backwards step has two explanations — a wrap and an
  undocumented exchange — and `with_capacity_kw(30, 900)` is what tells them
  apart. Without it, a meter swapped at 800 000 reads as 200 000 kWh of
  consumption in a quarter-hour.

## The conversion never invents a value

Where no honest difference exists, `to_lastgang` emits **no interval** and
records an `Anomaly`. The hole then surfaces as an ordinary V01 gap in
[validation](@/docs/validation.md) and is filled, with an audit trail, by
[Ersatzwertbildung](@/docs/substitute-values.md). Guessing at this layer would
bury the problem inside a value that looks measured.

## Two readings a year apart

The SLP path has no interval series at all — only a Jahresablesung:

```rust
use metering::reading::{LastgangConfig, MeterReading, consumption_between};
use rust_decimal::dec;
use time::macros::datetime;

let start = MeterReading::measured(datetime!(2025-01-01 0:00 UTC), dec!(14_230));
let end   = MeterReading::measured(datetime!(2026-01-01 0:00 UTC), dec!(17_845));
assert_eq!(consumption_between(&start, &end, &LastgangConfig::default())?, dec!(3615));
# Ok::<(), metering::reading::Anomaly>(())
```

## Across a meter exchange

The readings cannot be subtracted across the boundary, because the new register
starts over. `MeterExchangeEvent` pairs the old meter's final reading with the
new one's first, and differences them separately.

These methods return `Result`. They used to clamp a backwards step to zero under
the name *"rollover protection"* — which meant a Jahresabrechnung whose old
register had wrapped billed **0 kWh** for the whole pre-exchange span, silently.
