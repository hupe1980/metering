+++
title = "Getting started"
description = "Install the crate and do the five everyday things: build a series, validate and substitute it, aggregate it, derive it from readings, and convert gas."
weight = 1
[extra]
group = "Start"
+++

## Install

```bash
cargo add metering
cargo add metering --features serde
```

MSRV 1.88, edition 2024; [`serde`](@/docs/design.md#serde) is optional. The
prelude holds the core nouns, the pipeline entry points and `Decimal` with
`dec!`.

**Instants are UTC, periods are local** (Europe/Berlin) — the market's split,
EDI@Energy *Allgemeine Festlegungen* 6.1d, Kap. 3: *"Die Angabe von Zeiten in
einer EDIFACT Nachricht erfolgt in koordinierter Weltzeit (Coordinated
Universal Time, UTC)."* and *"Alle in den Prozessen genannten Zeitpunkte …
nutzen die gesetzliche deutsche Zeit."*

## 1. Build intervals, a series and identifiers

A `MeterInterval` is one value over `[from, to)` with an explicit
`QualityFlag`. A `Series` is sorted, unique, on one grid, one channel. See
[Series and readings](@/docs/series-and-readings.md) and
[Identifiers](@/docs/identifiers.md).

```rust
use metering::prelude::*;
use time::{Duration, macros::date};

let day = DayBoundary::Strom.day(date!(2026 - 06 - 01)).unwrap();
let slots = (0..96)
    .map(|i| MeterInterval::quarter_hour(day.start() + Duration::minutes(15 * i), dec!(0.5), QualityFlag::Measured))
    .collect::<Result<Vec<_>, _>>().unwrap();
let series = Series::new(Resolution::QUARTER_HOUR, DayBoundary::Strom, slots).unwrap();
assert_eq!(series.billable_total(), Some(dec!(48)));

// Identifiers verify their check digit at the parse.
let malo: MaloId = "41373559241".parse().unwrap();
assert_eq!(malo.check_digit(), 1);
assert!("41373559214".parse::<MaloId>().is_err());
```

## 2. Validate, then substitute

`validate` returns findings, the rules that ran, coverage and a grade;
`substitute` replaces exactly the slots an error finding covers. See
[Validation and substitution](@/docs/validation-and-substitution.md).

```rust
use metering::prelude::*;
use metering::time::holiday::Bundesland;
use metering::vee::substitute::SubstitutionReason;
use metering::vee::validation::{Grade, Rule};
use time::{Duration, macros::{date, datetime}};

let day = DayBoundary::Strom.day(date!(2026 - 06 - 01)).unwrap();
let slots = (0..96)
    .filter(|i| !(40..44).contains(i)) // one hour missing
    .map(|i| MeterInterval::quarter_hour(day.start() + Duration::minutes(15 * i), dec!(1), QualityFlag::Measured))
    .collect::<Result<Vec<_>, _>>().unwrap();
let series = Series::new(Resolution::QUARTER_HOUR, DayBoundary::Strom, slots).unwrap();

let rules = Rules::strom(day, datetime!(2026-07-01 0:00 UTC), None);
let report = validate(&series, &rules);
assert_eq!(report.grade(), Grade::C);
assert_eq!(report.by_rule(Rule::Gap).count(), 1);

let policy = Policy::strom(Bundesland::Be, SubstitutionReason::CommunicationFailure);
let filled = substitute(&series, &report, &policy).unwrap();
assert_eq!(filled.substitutes.len(), 4);
assert_eq!(filled.substitutes[0].code, Some("Z92")); // interpolation
assert_eq!(filled.substitutes[0].reason.code(), "Z75"); // STS+Z40
assert_eq!(filled.series.len(), 96);
```

## 3. Aggregate, and split into tariff registers

See [Billing quantities](@/docs/billing-quantities.md).

```rust
use metering::billing::zaehlzeit::{HT, NT, Zaehlzeitdefinition};
use metering::prelude::*;
use time::{Duration, macros::date};

let day = DayBoundary::Strom.day(date!(2026 - 06 - 01)).unwrap(); // a Monday
let slots = (0..96)
    .map(|i| MeterInterval::quarter_hour(day.start() + Duration::minutes(15 * i), dec!(1), QualityFlag::Measured))
    .collect::<Result<Vec<_>, _>>().unwrap();
let series = Series::new(Resolution::QUARTER_HOUR, DayBoundary::Strom, slots).unwrap();

let period = aggregate(&series, day).unwrap();
assert_eq!(period.arbeitsmenge, dec!(96));
assert_eq!(period.spitzenleistung_kw, Some(dec!(4)));

// HT 06:00–22:00 on weekdays, NT for the rest.
let zzd = Zaehlzeitdefinition::ht_nt("NB-1", date!(2026 - 01 - 01), 6 * 60, 22 * 60).unwrap();
let split = zzd.split_energy(series.as_slice()).unwrap();
assert_eq!(split.per_register[HT], dec!(64));
assert_eq!(split.per_register[NT], dec!(32));
assert!(split.is_complete());
```

## 4. From readings to a series

`to_lastgang` differences a Zählerstandsgang into one Lastgang per register.
See [Series and readings](@/docs/series-and-readings.md).

```rust
use metering::prelude::*;
use time::{Duration, macros::datetime};

let t = datetime!(2026-06-01 0:00 UTC);
let readings: Vec<MeterReading> = [dec!(1000.0), dec!(1002.5), dec!(1004.8), dec!(1007.0)]
    .into_iter()
    .enumerate()
    .map(|(i, v)| MeterReading::measured(t + Duration::minutes(15 * i as i64), v))
    .collect();

let lastgang = &to_lastgang(&readings, &LastgangConfig::strom())[0];
assert!(lastgang.is_clean());
let series = lastgang.to_series(Resolution::QUARTER_HOUR, DayBoundary::Strom).unwrap();
assert_eq!(series.len(), 3); // n readings, n − 1 intervals
assert_eq!(series.billable_total(), Some(dec!(7.0)));
```

## 5. The Gastag and gas conversion

The Gastag runs 06:00 to 06:00 local; kWh_Hs = m³ × Brennwert × Zustandszahl.
See [Gas, units and load profiles](@/docs/gas-and-units.md).

```rust
use metering::gas::conversion::gas_m3_to_kwh_hs;
use metering::prelude::*;
use time::macros::{date, datetime};

let gastag = DayBoundary::Gas.day(date!(2026 - 01 - 15)).unwrap();
assert_eq!(gastag.start(), datetime!(2026-01-15 5:00 UTC)); // 06:00 MEZ

// 100 m³ at a Brennwert of 11.2 kWh/m³ and a Zustandszahl of 0.95.
assert_eq!(gas_m3_to_kwh_hs(dec!(100), dec!(11.2), dec!(0.95)), Some(dec!(1064.000)));
```
