+++
title = "Validation and quality"
description = "The order-independent rule engine V01–V12, why the outlier test is a Hampel identifier, and the A/B/C/F grade layered over it."
weight = 6
+++

## The rules

| Rule | ID | Severity | What it catches |
|---|---|---|---|
| Gap | V01 | Error | Any uncovered span — a missing interval, or a series off the grid |
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

A rule's id is its `Vnn` code everywhere — `Display`, `as_str` and the `serde`
tag all say `V01` — so a stored finding reads back through the vocabulary it was
written with.

**V10 is deliberately unused.** A rollover is a property of a *register*, and a
`MeterInterval` carries interval energy rather than a cumulative Zählerstand, so
detection lives in [`reading`](@/docs/readings.md). The number stays unused so a
stored `V10` finding cannot be reinterpreted.

## Gaps are measured against what is covered, not against the previous interval

V01 reports **any** uncovered span, not only a whole missing interval: a series
whose intervals are the right length but sit off the grid — 00:00–00:15 then
00:20–00:35 — leaves five minutes uncovered that V06 cannot see, because every
interval is exactly 900 s.

The span is measured from the **furthest end seen so far**, which is not the
same as the previous interval's end once anything overlaps. Sorted by `from`, a
short interval swallowed by a long one is followed by a start earlier than the
long one's end, and comparing pairwise would report the covered remainder of the
long interval as missing. An overlapping series is already an error (V02); a
second, wrong finding on top of it sends the reader to a slot that has data.

## A clean report is not a clean series

Four of the eleven rules are **opt-in**. They need a number this library refuses
to invent, and leaving the field `None` turns the rule off:

| Field | Rules it disables when `None` |
|---|---|
| `expected_interval_secs` | V01 `GapDetected`, V06 `InconsistentIntervalLength` |
| `outlier_sigma` | V04 `StatisticalOutlier` |
| `now` | V08 `FutureTimestamp` |
| `max_plant_power_kw` | V12 `ImplausiblePower` |

Two more are **opt-out** — on by default, and switched off by a value rather
than by a `None`:

| Field | Rule it disables |
|---|---|
| `negative_energy_is_error = false` | V03 `NegativeEnergy` |
| `zero_run_threshold = 0` | V05 `SuspiciousZeroRun` |

`enabled_rules()` reports both kinds, so a `0` threshold cannot leave a clean
report claiming a stuck meter was looked for.

This matters in practice: **`QualityConfig::for_sparte` — the per-commodity
configuration — sets no `max_plant_power_kw`**, because a nameplate capacity is
a property of a device, not of a commodity. V12 therefore does not fire on that
path unless a caller supplies one, and a service that assumed otherwise would
describe an Error-severity rule it never evaluated.

```rust
use metering::{QualityConfig, Sparte, ValidationRuleId, ValidationConfig, validate_intervals};

// Before a run: what the configuration permits.
let cfg = QualityConfig::for_sparte(Sparte::Strom);
assert_eq!(cfg.validation.disabled_rules().to_string(), "V08, V12");

// Each inert rule names the field that would arm it.
for rule in cfg.validation.disabled_rules() {
    let field = rule.enabling_field().unwrap_or("(always on)");
    println!("{rule} is off — set ValidationConfig::{field}");
}

// After a run: what actually ran on this series.
let report = validate_intervals(&[], &ValidationConfig::default());
assert!(report.skipped().contains(ValidationRuleId::ImplausiblePower));
```

`enabled_rules()` is the config's answer and `ValidationResult::evaluated` is
the run's, and the two differ exactly when the **data** — not the config — was
what stopped a rule: V04 needs more points than its window is wide, so a
20-interval series evaluates ten rules where a 96-interval one evaluates eleven.

`QualityReport` carries the same set, because a grade condenses whatever ran and
cannot speak for a rule that did not. `covers_every_rule()` says whether an `A`
speaks for all eleven.

## Order independence

Adjacency rules are evaluated in timestamp order whatever order the caller
supplies, so a shuffled series cannot produce spurious gaps or overlaps. The
disorder itself is reported once as V11, and every `interval_index` still points
into the caller's slice.

The same promise holds for `aggregate`, `resample`, `fill_gaps`,
`split_energy`, `to_lastgang` and the grader, and it is not left as prose:
`tests/order_independence.rs` shuffles a generated series and asserts an
identical result for each of them under proptest. Two real defects hid there
until it existed, and both needed a **tie** to show — a tied peak power, and two
quality flags that shared a severity rank — which is why the generator draws
half its series from a coarse half-kWh value grid.

## V05 counts the run, not the threshold

A stuck meter is reported once per run, anchored at the run's first interval,
carrying the length the run actually reached. `zero_run_threshold` decides
*whether* to report it, never what number appears in the finding — a meter
frozen for three weeks says three weeks.

A **gap ends a run**: zeros either side of a hole are two runs, because what
happened in the hole is unknown and reporting a stuck meter across it claims a
measurement nobody took. Adjacency is the same test V01 uses for a gap.

## Declare the period, or gaps at the edges are invisible

```rust
use metering::{ValidationConfig, validate_intervals};
# use metering::MeterInterval;
# use rust_decimal::dec;
use time::macros::datetime;
# let delivered =
#     vec![MeterInterval::quarter_hour(datetime!(2026-06-01 0:00 UTC), dec!(2.0))];

// Without a period the data defines its own extent — a truncated delivery is clean.
assert!(validate_intervals(&delivered, &ValidationConfig::default()).is_clean());

// With one, the missing tail is an Error.
let cfg = ValidationConfig::default()
    .over_period(datetime!(2026-06-01 0:00 UTC), datetime!(2026-06-01 2:00 UTC));
assert!(validate_intervals(&delivered, &cfg).has_errors());
```

A month whose last week never arrived validates clean without a declared period.
That is the failure mode that matters most at billing time.

## V06 knows a calendar day is not 86 400 seconds

A daily series read once a day is 23 hours long each spring and 25 each autumn.
Where `expected_interval_secs` is `86_400` and an interval starts exactly on a
day boundary, V06 measures it against the calendar rather than the flat second
count — otherwise every gas and water series drew a warning on both transition
days every year, for being exactly right.

Which boundary is `ValidationConfig::day_boundary`, so a daily **gas** series on
the 06:00 Gastag gets the same allowance as an electricity series on the
Liefertag. The transition days are not even the same date for the two: the
clocks move at 02:00/03:00, inside the Gastag that began the previous morning.

```rust
use metering::{ValidationConfig, calendar::DayBoundary};

let gas_daily = ValidationConfig {
    expected_interval_secs: Some(86_400),
    ..Default::default()
}
.on(DayBoundary::Gastag);
assert_eq!(gas_daily.day_boundary, DayBoundary::Gastag);
```

A fixed 24-hour window that merely happens to be 86 400 s long is a different
thing from a calendar day and gets no allowance.

## V01 reports **any** uncovered span

Not only a whole missing interval. A series of exactly-900-second intervals that
does not sit on the grid —

```text
00:00–00:15   00:20–00:35   00:40–00:55  …
```

— leaves five minutes unaccounted for in every slot, which V06 cannot see
because every interval is the right length. Any positive hole is an Error; one
shorter than the grid says so in its message rather than reporting "0 intervals
missing".

## V07 looks at the repeated hour, on every day the series covers

When CEST ends, local 02:00–03:00 happens **twice** — once at UTC+2, once at
UTC+1 — and the two passes occupy a two-hour UTC window either side of the
transition. A series converted from local time without carrying the offset keeps
only one of them, and an hour of energy vanishes.

V07 looks only at that window. Comparing the whole day's covered duration
against 25 hours instead cannot tell a collapsed hour from an ordinary gap: any
two missing quarter-hours anywhere on a fall-back day would be reported as "the
repeated hour was collapsed", which sends the reader to the wrong place.

A gap at midday is a V01 gap and nothing else. A genuinely collapsed hour is
caught even on a day that is otherwise complete.

**Every fall-back day the series spans is examined, not only the first** — a
month of MSCONS, an annual export or a MaBiS Summenzeitreihe can hold one
anywhere inside it, and those are the deliveries where a missing hour is
invisible by eye. The transition comes from the tz database, which has not
always put it on the last Sunday in October.

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
| `C` | blocking findings within `max_review_findings`, coverage at or above `min_review_coverage_pct` — somebody has to look |
| `F` | anything worse, and any empty series |

The B/C line is **severity**, not a count: twenty spike warnings still bill, one
gap does not. Where the C/F line falls is `QualityConfig`'s — a Klärfall queue
that absorbs three corrections a day cannot absorb thirty — and only `F` blocks
automated billing.

The grader computes no statistics of its own: every rule has one
implementation, in the validation engine, and a test asserts the grade and the
findings can never disagree.

### A shuffled series grades `B`, on purpose

V11 is a finding like any other, so a series that is otherwise spotless but
arrived out of order has one finding and cannot be an `A`. That is intended: a
shuffled MSCONS delivery usually means a broken merge upstream, and "bill it and
note it" is the right verdict.

It is worth knowing before it surprises you — a series read back out of a
`HashMap` will grade `B` for no reason visible in the data. V11 is a Warning, so
it can only ever cost the `A`; it can never reach `C`, which needs a blocking
finding. Every other quantity the report carries — coverage, the zero run, the
counts, `evaluated` — is identical either way, and
`tests/order_independence.rs` pins exactly that.
