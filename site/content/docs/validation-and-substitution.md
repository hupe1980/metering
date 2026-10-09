+++
title = "Validation and substitution"
description = "Plausibilisierung and Ersatzwertbildung: the rules, the grade, and the substitution policy — interpolation up to 2 h, comparable days beyond, a register anchor — with their sources and MSCONS codes."
weight = 6
aliases = ["/docs/validation/", "/docs/substitute-values/"]
[extra]
group = "Tasks"
+++

1. `validate(&series, &rules)` → `Report`: findings, the rules that ran,
   coverage of the declared period, a grade.
2. `substitute(&series, &report, &policy)` → `Filled`: the repaired series, one
   `Substitute` per slot, and the spans it refused.

## Validation

`Rules::strom(period, now, capacity_kw)`, `Rules::gas(…)` and
`Rules::new(sparte, …)` are presets. The period is the one the series is
**declared** to cover, so a gap at its head or tail is found.

| `Rule` | Severity | Catches | Runs | Basis |
|---|---|---|---|---|
| `Gap` | error | a slot of the period with no value | always | MeteringCode 2006 A7.1.2.1/A7.1.2.2 |
| `NonBillable` | error | a `Faulty` or `Unknown` value | always | A7.1.2.3 |
| `Negative` | error | a negative quantity | on; `without_negative()` | Codeliste OBIS 2.5c § 2.1 |
| `Capacity` | error | average power above the plant capacity | with a `capacity_kw` | A7.2 |
| `Future` | blocking | a slot not yet ended at `now` | always | — |
| `Laengsvergleich` | blocking | Σ load curve ≠ register advance beyond a tolerance | opt-in: `laengsvergleich(start, end, tol)` | A7.2; Elexon BSCP502 § 4.1.5 |
| `OutsidePeriod` | warning | data outside the declared period | always | — |
| `ZeroRun` | warning | zeros for at least a set duration | on; `without_zero_run()` | A7.2 |
| `Stale` | warning | a non-zero value frozen over a window | Strom preset; `stale(…)` | pvanalytics `stale_values_diff` |
| `Spike` | warning | a value far from its neighbours (Hampel, MAD floor) | on; `without_spike()` | A7.2 |
| `Seasonal` | warning | a value far from the same slot on comparable days | opt-in: `seasonal(land, days)` | A7.2 |

The basis is VDN *MeteringCode 2006*, Anlage 7. It names the checks, not their
limits, so every threshold is a setter on `Rules` with a documented preset.

**Grade**: `A` no findings, `B` warnings only, `C` errors (repairable by
substitution), `F` anything blocking. `Report::evaluated` lists the rules that
ran and `skipped()` the rest, so "found nothing" and "never looked" differ.

```rust
use metering::prelude::*;
use metering::vee::validation::{Grade, Rule};
use time::{Duration, macros::{date, datetime}};

let day = DayBoundary::Strom.day(date!(2026 - 06 - 01)).unwrap();
let slots = (0..96)
    .map(|i| {
        let kwh = if i == 40 { dec!(-0.3) } else { dec!(1.2) };
        MeterInterval::quarter_hour(day.start() + Duration::minutes(15 * i), kwh, QualityFlag::Measured)
    })
    .collect::<Result<Vec<_>, _>>().unwrap();
let series = Series::new(Resolution::QUARTER_HOUR, DayBoundary::Strom, slots).unwrap();

let report = validate(&series, &Rules::strom(day, datetime!(2026-07-01 0:00 UTC), None));
assert_eq!(report.grade(), Grade::C);
assert_eq!(report.by_rule(Rule::Negative).count(), 1);
assert!(report.skipped().contains(&Rule::Capacity)); // no capacity given
assert!(report.skipped().contains(&Rule::Seasonal)); // opt-in
```

`Report::coverage` counts delivered intervals; `BillingPeriod::coverage`
counts billable ones.

## Substitution

`substitute` replaces every slot an **error** finding covers and nothing else.
Each substitute records its method, the MSCONS `STS+Z32` method code, the
`STS+Z40` reason, the gap length, its reference instants, and the original
value of a rejected slot.

| Situation | Method | Strom | Gas |
|---|---|---|---|
| gap ≤ 2 h, validated measured values on both sides | `Interpolation` | `Z92` | `Z92` |
| longer gap, or a gap at the edge of the data | `Vergleichswert` — mean of the same slot on the previous same-day-type days | `ZJ2` | `Z95` |
| gas only, chosen explicitly (`Policy::hold`) | `Hold` — the last measured value | — | `Z93` |
| a confirmed supply interruption (`Policy::outage`) | `Zero` | — | — |

- **2-hour line, two methods**: MeteringCode 2006 A8.2.2.1 (interpolation from
  checked values) and A8.2.2.2 (Vergleichswertverfahren); § 55 Abs. 2 Z 1 ElWG
  (AT) and CPUC VEE Rev. 2.0 § 4.1 draw the same line. Inclusive;
  `Policy::short` sets it.
- **Comparable days**: the previous days of the same day class in the Land (a
  holiday counts as a Sunday), three by default (`Policy::days`), from the
  series or `Policy::history`.
- **References** are only validated measured values, the window anchored at the
  gap start.
- **Zeros** only for an established outage (A7.1.2.2). `Policy::hold` refuses
  any Sparte but gas, the only one with a code for it.
- **No applicable method**: the slot is refused with its reason
  (`NoComparableHistory`, `NoHeldValue`, …).

The reason is a `SubstitutionReason` (`STS+Z40`, MSCONS MIG 2.5), stated by the
caller. A value rejected by a plausibility rule carries `ZA1` *Messwert
unplausibel* instead.

### The register anchor

Two register readings bracketing the gap (`Policy::anchor`) rescale the
substitutes so the series sums to the register advance **exactly** —
MeteringCode A8.2.2.2 *Skalierung*, Elexon BSCP502 § 4.2, NL Meetcode § 5.4.3.
One cut to `SUBSTITUTE_DP`; the remainder goes on the last substitute.

```rust
use metering::prelude::*;
use metering::time::holiday::Bundesland;
use metering::vee::substitute::SubstitutionReason;
use time::{Duration, macros::{date, datetime}};

let day = DayBoundary::Strom.day(date!(2026 - 06 - 01)).unwrap();
let at = |i: i64| day.start() + Duration::minutes(15 * i);
let slots = (0..96)
    .filter(|i| !(40..44).contains(i)) // one hour missing
    .map(|i| MeterInterval::quarter_hour(at(i), dec!(1), QualityFlag::Measured))
    .collect::<Result<Vec<_>, _>>().unwrap();
let series = Series::new(Resolution::QUARTER_HOUR, DayBoundary::Strom, slots).unwrap();
let report = validate(&series, &Rules::strom(day, datetime!(2026-07-01 0:00 UTC), None));

// The register advanced 6 kWh over 09:30–11:30 local. Four measured slots hold
// 4 kWh, so the four substitutes are scaled to share the other 2 kWh.
let policy = Policy::strom(Bundesland::Be, SubstitutionReason::CommunicationFailure)
    .anchor(MeterReading::measured(at(38), dec!(100)), MeterReading::measured(at(46), dec!(106)))
    .unwrap();
let filled = substitute(&series, &report, &policy).unwrap();

let substituted: Decimal = filled.substitutes.iter().map(|s| s.value).sum();
assert_eq!(substituted, dec!(2));
assert!(filled.substitutes.iter().all(|s| s.scaled));
```

A filled series is an ordinary `Series`: re-validate it, then resample.
