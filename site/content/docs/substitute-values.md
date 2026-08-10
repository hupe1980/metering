+++
title = "Ersatzwertbildung"
description = "Substitute values under § 60 MsbG: the four methods, the calendar-aware grid, and why the audit trail records what ran rather than what was asked for."
weight = 5
+++

## The legal basis, precisely

**§ 60 Abs. 1 MsbG** places the duty on the Messstellenbetreiber: the data
collected under §§ 55–59 must be *aufbereitet* and transmitted to the berechtigte
Stellen. **§ 60 Abs. 2 MsbG** names what that preparation includes:

> Bei Messstellen mit intelligenten Messsystemen sollen die Aufbereitung der
> Messwerte, insbesondere die Plausibilisierung und die Ersatzwertbildung im
> Smart-Meter-Gateway, und die Datenübermittlung über das Smart-Meter-Gateway
> direkt an die berechtigten Stellen erfolgen…

Note what that sentence does **not** contain: any procedure. It says
Ersatzwertbildung is owed and where it belongs; it prescribes no method, no
reference period and no ranking between them. The process rules are BNetzA
Festlegungen (currently **BK6-24-174**) and the technical ones VDE-AR-N 4400.

Because VDE-AR-N 4400 is a paywalled Anwendungsregel whose text cannot be
reproduced or verified here, every threshold in this module is a parameter with
a documented default rather than a hard-coded claim of conformance.

## The four methods

| Method | Use |
|---|---|
| `LinearInterpolation` | short gaps between plausible values |
| `PriorPeriodAverage` | the same (weekday, hour, minute) slot over the preceding week |
| `LastValueCarryForward` | conservative fallback |
| `ZeroFill` | an affirmatively documented shutdown |

```rust
use metering::{FillGapsConfig, IntervalResolution, SubstituteMethod, fill_gaps};
# use metering::{MeterInterval, QualityFlag};
# use rust_decimal::dec;
# use time::macros::datetime;
# let series = vec![
#   MeterInterval { from: datetime!(2026-01-01 0:00 UTC), to: datetime!(2026-01-01 0:15 UTC),
#     value: dec!(0), quality: QualityFlag::Measured, obis_code: None },
#   MeterInterval { from: datetime!(2026-01-01 1:00 UTC), to: datetime!(2026-01-01 1:15 UTC),
#     value: dec!(100), quality: QualityFlag::Measured, obis_code: None }];
let filled = fill_gaps(
    &series,
    &FillGapsConfig::new(
        IntervalResolution::QuarterHour,
        datetime!(2026-01-01 0:00 UTC),
        datetime!(2026-01-01 1:15 UTC),
    )
    .short_gap_threshold(10),
);

// Three unknowns between 0 and 100 sit at the quarter points.
let values: Vec<_> = filled.intervals.iter().map(|iv| iv.value).collect();
assert_eq!(values, vec![dec!(0), dec!(25), dec!(50), dec!(75), dec!(100)]);
assert!(filled.substitutions.iter().all(|e| e.method == SubstituteMethod::LinearInterpolation));
```

The interpolation fractions are **interior** — `1/(n+1) … n/(n+1)`. Using
`i/n` would put the first substitute exactly on the last measured value and
never reach the closing one: a systematic bias on every rising or falling gap.

## The grid is calendar-aware, and mandatory

The resolution and the period are constructor arguments, not loose positionals.
They are the two things a gap fill cannot proceed without and the two most
easily got wrong.

The resolution is an `IntervalResolution`, not a second count, so a daily or
monthly fill walks Europe/Berlin calendar periods. Stepping a fixed 86 400 s
drifts by an hour at each DST transition and never recovers: every slot after
the last Sunday in March sits an hour off its Liefertag, measured values stop
matching the grid, and the whole rest of the year is silently substituted.

## The audit trail records what ran

A requested method can be impossible: a prior-period average with no matching
reference slot, an interpolation with nothing after the gap to interpolate
towards. Every such case falls back, and `SubstituteEntry::method` reports the
method **that actually produced the value**. Recording the request instead would
put a claim in the trail that the number does not support.

## § 60 Abs. 6 MsbG is a deletion duty

Worth stating plainly, because it is commonly read backwards:

> Der Messstellenbetreiber muss personenbezogene Messwerte […] **löschen oder
> […] anonymisieren**, sobald […] eine Speicherung […] nicht mehr erforderlich
> ist, **spätestens jedoch nach drei Jahren** ab dem Schluss des Kalenderjahres,
> in dem der jeweilige Messwert erhoben wurde […]

Three years is a **ceiling**, not a retention mandate, and the operative trigger
is earlier still — as soon as the data is no longer needed. Substitute values
are Messwerte for this purpose.
