+++
title = "Ersatzwertbildung"
description = "Substitute values under § 60 MsbG: the four methods, the market's own 28 reasons, the calendar-aware grid, and why the audit trail records what ran rather than what was asked for."
weight = 7
+++

## The legal basis, precisely

**§ 60 Abs. 1 MsbG** places the duty on the Messstellenbetreiber: the data
collected under §§ 55–59 must be *aufbereitet* and transmitted to the berechtigte
Stellen. **§ 60 Abs. 2 MsbG** names what that preparation includes:

> Bei Messstellen mit intelligenten Messsystemen sollen die Aufbereitung der
> Messwerte, insbesondere die Plausibilisierung und die Ersatzwertbildung im
> Smart-Meter-Gateway, und die Datenübermittlung über das Smart-Meter-Gateway
> direkt an die berechtigten Stellen erfolgen, soweit das Bundesamt für
> Sicherheit in der Informationstechnik dies als technisch möglich bewertet und
> die Bundesnetzagentur auf Basis dieser Bewertung eine Festlegung nach § 75
> Satz 1 Nummer 4 trifft.

Two things that sentence does **not** say. It prescribes no procedure — no
method, no reference period, no ranking between them. And the Smart-Meter-Gateway
placement is *conditional*: until the BSI assesses and the BNetzA decides,
Satz 2 expressly permits the preparation to happen **outside** the gateway,
*"durch den Messstellenbetreiber ganz oder teilweise […] und dauerhaft,
außerhalb des Smart-Meter-Gateways"*. Computing Ersatzwerte in a process rather
than in a gateway is the case the statute currently describes, not a workaround.

The process rules are BNetzA Festlegungen (currently **BK6-24-174**) and the
technical ones VDE-AR-N 4400.

Because VDE-AR-N 4400 is a paywalled Anwendungsregel whose text cannot be
reproduced or verified here, every threshold in this module is a parameter with
a documented default rather than a hard-coded claim of conformance.

## The four methods

| Method | Use |
|---|---|
| `LinearInterpolation` | short gaps between plausible values |
| `PriorPeriodAverage` | the same slot on comparable days of the preceding week |
| `LastValueCarryForward` | conservative fallback |
| `ZeroFill` | an affirmatively documented shutdown |

The Vergleichstag week is seven **Berlin calendar days**, not 168 hours: the
matching slot one week earlier is 169 UTC hours back across the autumn
fall-back, and a fixed-duration window would silently drop the reference and
degrade the method to carry-forward for a week every October.

**Which days count as comparable is a choice.** By default it is the same
weekday, which needs no calendar. `matching_day_types(land)` uses the SLP day
type instead, so a gap on 1 May draws on Sundays and holidays rather than on the
previous working Fridays — and a Wednesday gap draws on every Werktag of the
week rather than on one Wednesday:

```rust
# use metering::{Bundesland, FillGapsConfig, IntervalResolution};
# use metering::substitute::ReferenceDayMatch;
# use time::macros::datetime;
let cfg = FillGapsConfig::new(
    IntervalResolution::QuarterHour,
    datetime!(2026-05-01 0:00 UTC),
    datetime!(2026-05-02 0:00 UTC),
)
.matching_day_types(Bundesland::By);

assert_eq!(cfg.reference_days, ReferenceDayMatch::DayType(Bundesland::By));
```

A daily, monthly or yearly grid is cut at midnight by default and at the 06:00
Gastag on request — `FillGapsConfig::on(DayBoundary::Gastag)` — so filling a gas
SLP series produces Gastage rather than Liefertage.

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

The anchors are the **billable** values either side, at their true grid-slot
distances. A present-but-faulty slot is never overwritten — the fill invents
only missing slots — but it does not anchor the line either: the missing slots
around a faulty reading all land on the one straight line between the billable
values that bracket it.

## The vocabulary is the market's, not this crate's

§ 60 MsbG prescribes no method and no reason, but the market communication does.
EDI@Energy's MSCONS MIG carries both as code lists, and this crate speaks them
rather than inventing a parallel set that would have to be mapped at the
boundary — a mapping that is never quite one-to-one, and where a reason quietly
changes meaning.

**`STS+Z40 Grund der Ersatzwertbildung`** — 28 Statusanlässe, and
`SubstitutionReason` is exactly that list:

```rust
use metering::{Sparte, SubstitutionReason};

let reason = SubstitutionReason::CommunicationFailure;
assert_eq!(reason.code(), "Z75");                       // what MSCONS carries
assert_eq!(reason.as_str(), "COMMUNICATION_FAILURE");   // what a database column holds
assert_eq!(reason.description(), "Kommunikationsstörung");
assert_eq!(SubstitutionReason::from_code("Z75"), Some(reason));

// The MIG annotates most codes per commodity.
assert!(SubstitutionReason::VoltageFailure.applies_to(Sparte::Strom));
assert!(!SubstitutionReason::VoltageFailure.applies_to(Sparte::Gas));
```

**`STS+Z32 Ersatzwertbildungsverfahren`** — and here the answer depends on the
commodity, because the MIG's own annotations do:

| Method | Strom | Gas |
|---|---|---|
| `LinearInterpolation` | `Z92` | `Z92` |
| `PriorPeriodAverage` | `ZJ2` Statistische Methode | `Z95` Historische Messwerte |
| `LastValueCarryForward` | — | `Z93` Haltewert |
| `ZeroFill` | — | — |

```rust
use metering::{Sparte, SubstituteMethod};

assert_eq!(SubstituteMethod::PriorPeriodAverage.market_code(Sparte::Gas), Some("Z95"));
assert_eq!(SubstituteMethod::LastValueCarryForward.market_code(Sparte::Strom), None);
```

Both `None`s are real answers. **A held value has no Strom code** — `Z93` is
annotated *Gas*, and the Strom list offers only the Vergleichswertverfahren —
and **a zero fill is not an Ersatzwertbildung at all**, since it asserts that
nothing was delivered rather than reconstructing what was. Where this returns
`None`, the caller has a process question, not a formatting one.

The value's own status is the `QTY` qualifier, and `QualityFlag` carries it:

```rust
use metering::QualityFlag;

assert_eq!(QualityFlag::Measured.market_code(), Some("220"));    // Wahrer Wert
assert_eq!(QualityFlag::Substituted.market_code(), Some("67"));  // Ersatzwert
assert_eq!(QualityFlag::Unknown.market_code(), None);            // no such code exists
```

## Nothing measured is silently replaced

A slot is filled only where nothing arrived for it. An interval that lands on no
grid slot — a series sitting off the grid, or starting on a different boundary —
would otherwise be dropped *and* its slot invented, which is the one outcome
worse than refusing to fill at all:

```rust
# use metering::{FillGapsConfig, IntervalResolution, MeterInterval, fill_gaps};
# use rust_decimal::dec;
# use time::macros::datetime;
// Seven minutes off the grid.
let off_grid = MeterInterval::measured(
    datetime!(2026-01-01 0:07 UTC),
    datetime!(2026-01-01 0:22 UTC),
    dec!(9),
);

let filled = fill_gaps(
    &[off_grid.clone()],
    &FillGapsConfig::new(
        IntervalResolution::QuarterHour,
        datetime!(2026-01-01 0:00 UTC),
        datetime!(2026-01-01 0:30 UTC),
    ),
);

assert!(!filled.placed_everything());
assert_eq!(filled.unplaced, vec![off_grid]);
```

## The grid is calendar-aware, and mandatory

The resolution and the period are constructor arguments, not loose positionals.
They are the two things a gap fill cannot proceed without and the two most
easily got wrong.

The resolution is an `IntervalResolution`, not a second count, so a daily or
monthly fill walks Europe/Berlin calendar periods. Stepping a fixed 86 400 s
drifts by an hour at each DST transition and never recovers: every slot after
the last Sunday in March sits an hour off its Liefertag, measured values stop
matching the grid, and the whole rest of the year is silently substituted.

## A substituted value is a number someone can write down

Two of the four methods divide — an interpolation by the distance between its
anchors, an average by its sample count — so both can produce a quotient with
twenty-eight significant digits. What they produce is not an intermediate: it is
written into the returned series and settled on. Every synthesised value is cut
to `SUBSTITUTE_DP` (6), half away from zero. No conservation identity runs
through a substitute — a filled series is complete, not balanced — so the
nearest representable value is the honest one, and truncating would bias a long
outage downwards.

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
