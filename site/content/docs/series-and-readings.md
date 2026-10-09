+++
title = "Series and readings"
description = "MeterInterval and Series, the validated input of the pipeline; Zählerstände differenced into a Lastgang; resampling onto a coarser grid."
weight = 4
aliases = ["/docs/readings/"]
[extra]
group = "Concepts"
+++

## `MeterInterval`: one value, one span, one quality

The energy or volume in `[from, to)`, its `QualityFlag`, and optionally the
OBIS code of its channel. Constructors refuse a reversed or empty span and take
the quality explicitly.

| `QualityFlag` | Billable | MSCONS `QTY` |
|---|---|---|
| `Measured` | yes | `220` Wahrer Wert |
| `Substituted` | yes | `67` Ersatzwert |
| `Estimated` | yes | `187` Prognosewert |
| `Preliminary` | yes | `Z18` Vorläufiger Wert |
| `Calculated`, `Corrected` | yes | — (a Rechenwert or Korrekturgrund is an `STS` status) |
| `Faulty` | **no** | `20` Nicht verwendbarer Wert |
| `Unknown` | **no** | — (the status did not arrive) |

`value` is in the channel's unit — kWh for Strom and Wärme, m³ for Gas before
conversion and for Wasser.

```rust
use metering::{MeterInterval, QualityFlag};
use rust_decimal::dec;
use time::macros::datetime;

let iv = MeterInterval::new(datetime!(2026-06-01 12:00 UTC), datetime!(2026-06-01 12:15 UTC), dec!(2.5), QualityFlag::Measured)
    .unwrap();
assert_eq!(iv.demand_kw(), Some(dec!(10))); // kWh × 3600 ÷ s
assert!(MeterInterval::new(iv.to(), iv.from(), dec!(1), QualityFlag::Measured).is_err());
```

## `Series`: validated once

`Series::new(resolution, boundary, intervals)` checks once that intervals are
sorted, unique and non-overlapping, each exactly the `Resolution` bucket laid
out from the local day start of the `DayBoundary`, and of one channel.
**Gaps are allowed** — a missing slot is a
[validation](@/docs/validation-and-substitution.md) finding. Each refusal names
the instant: `SeriesError::Duplicate`, `Overlap`, `Misaligned`,
`MixedChannels`, `OutOfRange`.

```rust
use metering::series::SeriesError;
use metering::{DayBoundary, MeterInterval, QualityFlag, Resolution, Series};
use rust_decimal::dec;
use time::macros::datetime;

// 12:07 is not on the quarter-hour grid.
let off = MeterInterval::new(datetime!(2026-06-01 12:07 UTC), datetime!(2026-06-01 12:22 UTC), dec!(1), QualityFlag::Measured)
    .unwrap();
let err = Series::new(Resolution::QUARTER_HOUR, DayBoundary::Strom, vec![off]).unwrap_err();
assert!(matches!(err, SeriesError::Misaligned { .. }));
```

## Resampling sums, and never invents

`resample(&series, Resolution)` sums onto a coarser grid on the series' own
boundary (a gas series onto Gastage). A bucket missing any source slot is
**left out** — a gap, not a short total — so substitute first. A bucket's
quality is the worst of its slots.

```rust
use metering::prelude::*;
use time::{Duration, macros::date};

let day = DayBoundary::Strom.day(date!(2026 - 10 - 25)).unwrap();
let slots = (0..100)
    .map(|i| MeterInterval::quarter_hour(day.start() + Duration::minutes(15 * i), dec!(0.25), QualityFlag::Measured))
    .collect::<Result<Vec<_>, _>>().unwrap();
let series = Series::new(Resolution::QUARTER_HOUR, DayBoundary::Strom, slots).unwrap();

let daily = resample(&series, Resolution::Day).unwrap();
assert_eq!(daily.len(), 1); // one 25-hour day
assert_eq!(daily.as_slice()[0].value(), dec!(25));
```

## From Zählerstände to a Lastgang

The gateway delivers a **Zählerstandsgang** (BNetzA BK6-24-174), which § 2
Satz 1 Nr. 27 MsbG defines as *"die Messung einer Reihe viertelstündig
ermittelter Zählerstände von elektrischer Arbeit und stündlich ermittelter
Zählerstände von Gasmengen"*.

`to_lastgang` groups readings by register (OBIS code), sorts them, and returns
one `Lastgang` per register; `LastgangConfig::strom()` relabels it as the
register's Lastgang channel (`1-0:1.8.0` → `1-0:1.29.0`). A backwards step is a
wrap or a meter exchange, so a rollover is reconstructed only with both a
register width and a plausibility cap:

```rust
use metering::prelude::*;
use time::{Duration, macros::datetime};

let t = datetime!(2026-06-01 0:00 UTC);
let readings: Vec<MeterReading> = [dec!(999998), dec!(1), dec!(4)]
    .into_iter()
    .enumerate()
    .map(|(i, v)| MeterReading::measured(t + Duration::minutes(15 * i as i64), v))
    .collect();

// No width configured: the backwards step is refused, not guessed.
let refused = &to_lastgang(&readings, &LastgangConfig::strom())[0];
assert_eq!(refused.anomalies.len(), 1);
assert_eq!(refused.intervals.len(), 1);

// A six-digit register on a 30 kW connection: the wrap is 3 kWh, plausible.
let config = LastgangConfig::strom()
    .register_digits(6)
    .capacity_kw(dec!(30), Resolution::QUARTER_HOUR)
    .unwrap();
let lastgang = &to_lastgang(&readings, &config)[0];
assert_eq!(lastgang.rollovers.len(), 1);
assert_eq!(lastgang.intervals[0].value(), dec!(3));
assert_eq!(lastgang.total(), Some(dec!(6)));
```

A difference that cannot be taken emits **no interval** and an `Anomaly`; the
hole is a gap for validation and `substitute`. A derived interval's quality is
the worse of its two readings.

| Function | Does |
|---|---|
| `Lastgang::to_series(resolution, boundary)` | the result as a `Series` |
| `consumption_between(start, end, &config)` | two readings far apart (an annual read); refuses reversed arguments and mismatched registers |
| `detect_reading_cadence` | the cadence of the readings, for `capacity_kw` |
