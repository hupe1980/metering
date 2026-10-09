+++
title = "Billing quantities"
description = "Arbeitsmenge, Spitzenleistung and Benutzungsstundenzahl; tariff registers and § 14a Modul 3; Blindmehrarbeit, Mehr-/Mindermengen, network losses and the annual forecast."
weight = 7
aliases = ["/docs/tariff-registers/"]
[extra]
group = "Tasks"
+++

Quantities only — kWh, kW, kvarh, hours; prices are the Preisblatt's.

## Arbeitsmenge and Spitzenleistung

`aggregate(&series, period)` sums the **billable** intervals in the period and
takes the Spitzenleistung as the highest average power of one of them,
`kWh × 3600 ÷ s` — the Jahreshöchstleistung of § 17 Abs. 2 StromNEV. On a grid
coarser than an hour or a non-energy channel it is `None`.

```rust
use metering::prelude::*;
use time::{Duration, macros::date};

let day = DayBoundary::Strom.day(date!(2026 - 06 - 01)).unwrap();
// The evening's last six hours never arrived.
let slots = (0..72)
    .map(|i| MeterInterval::quarter_hour(day.start() + Duration::minutes(15 * i), dec!(0.5), QualityFlag::Measured))
    .collect::<Result<Vec<_>, _>>().unwrap();
let series = Series::new(Resolution::QUARTER_HOUR, DayBoundary::Strom, slots).unwrap();

let period = aggregate(&series, day).unwrap();
assert_eq!(period.arbeitsmenge, dec!(36));
assert_eq!(period.spitzenleistung_kw, Some(dec!(2)));
assert_eq!(period.spitzenleistung_at, Some(day.start()));
assert_eq!(period.coverage.pct(), Some(dec!(75))); // of the declared day
```

Coverage is against the **declared** period. Import and export are two
channels, so two calls; `sum_by_direction` balances intervals by the direction
their OBIS code names.

**Benutzungsstundenzahl** — `BillingPeriod::benutzungsdauer_h()`, Arbeitsmenge
÷ Spitzenleistung, cut to `BENUTZUNGSDAUER_DP` (§ 17 Abs. 1 StromNEV, Anlage
4). StromNEV expires on 31.12.2028 (`STROMNEV_AUSSERKRAFT`); nothing gates on
the date.

## Tariff registers

A `Zaehlzeitdefinition` is ordered windows over months × day group × **local**
time band, each naming a register, with a fallback — the Zweitarif, § 14a
Modul 3, or any definition transmitted over UTILTS. `in_land` books the Land's
holidays as Sundays.

### § 14a Modul 3 {#modul-3}

HT, NT and ST, mandatory for every Netzbetreiber from 1 April 2025 (BNetzA
BK8-22/010-A). HT/NT may be restricted to single quarters (BDEW AWH Modul 3
v1.1, § 2: *"Der Netzbetreiber hat das Wahlrecht, den Gültigkeitszeitraum auf
einzelne Quartale zu beschränken"*); outside them everything books into ST.

```rust
use metering::billing::zaehlzeit::{
    HT, NT, ST, Modul3Conformance, Modul3Context, Quarter, Zaehlzeitdefinition, assess_modul_3,
};
use metering::time::holiday::Bundesland;
use time::macros::{date, datetime};

let zzd = Zaehlzeitdefinition::modul_3(
    "NB-14A-3",
    date!(2026 - 01 - 01),
    (17 * 60, 20 * 60), // HT 17:00–20:00 local
    (22 * 60, 6 * 60),  // NT 22:00–06:00, across midnight
    &[Quarter::Q1, Quarter::Q4],
)
.unwrap()
.in_land(Bundesland::Nw)
.until(date!(2026 - 12 - 31));

assert_eq!(zzd.register_for(datetime!(2026-01-05 17:00 UTC)), Some(HT)); // 18:00 MEZ
assert_eq!(zzd.register_for(datetime!(2026-01-05 2:00 UTC)), Some(NT));
assert_eq!(zzd.register_for(datetime!(2026-07-06 16:00 UTC)), Some(ST)); // Q3

let assessment = assess_modul_3(&zzd, &Modul3Context::new().at_a_conforming_delivery_point());
assert_eq!(assessment.verdict, Modul3Conformance::Conforms);
```

`assess_modul_3` checks the AWH rules — three reachable registers covering
every instant, HT ≥ 2 h on every day class, identical windows across whole
billed quarters, at least two, one calendar year — and the `Modul3Context`
preconditions (Modul 1, iMSys, no RLM). An unstated precondition is `Unknown`.
Price corridors and the publication deadline are not checked.

### Splitting energy

`split_energy` books each billable interval into its register;
`Σ per_register + Σ straddling + Σ unassigned` is the billable energy exactly.
An interval across a band boundary is `straddling`.

```rust
use metering::billing::zaehlzeit::{HT, Zaehlzeitdefinition};
use metering::{MeterInterval, QualityFlag};
use rust_decimal::dec;
use time::macros::{date, datetime};

let zzd = Zaehlzeitdefinition::ht_nt("NB-1", date!(2026 - 01 - 01), 6 * 60, 22 * 60).unwrap();
let midday = MeterInterval::quarter_hour(datetime!(2026-01-05 9:00 UTC), dec!(3), QualityFlag::Measured).unwrap();
// An hour across 06:00 local holds both registers.
let across = MeterInterval::hour(datetime!(2026-01-05 4:30 UTC), dec!(1), QualityFlag::Measured).unwrap();

let split = zzd.split_energy(&[midday, across]).unwrap();
assert_eq!(split.per_register[HT], dec!(3));
assert_eq!(split.straddling.len(), 1);
assert!(!split.is_complete());
```

## Blindmehrarbeit

`Blindmehrarbeit = max(0, Blindarbeit − ratio × Wirkarbeit)`. The ratio is the
Netzbetreiber's (§ 17 Abs. 1 StromNEV); `ReactiveLimit::HALF` and
`COS_PHI_0_9` are the published practice, neither a default.

```rust
use metering::billing::reactive::{ReactiveBalance, ReactiveLimit};
use rust_decimal::dec;

let b = ReactiveBalance::new(dec!(100000), dec!(62000), ReactiveLimit::HALF).unwrap();
assert_eq!(b.blindmehrarbeit_kvarh(), dec!(12000.0));

let strict = ReactiveBalance::new(dec!(100000), dec!(62000), ReactiveLimit::COS_PHI_0_9).unwrap();
assert_eq!(strict.blindmehrarbeit_kvarh(), dec!(13570.0000));
```

## Mehr-/Mindermengen, losses, forecast

| Item | Computes | Basis |
|---|---|---|
| `ImbalanceSaldo` | metered − **bilanzierte** energy, annual; a Mehrmenge is credited, a Mindermenge invoiced (Netzbetreiber's side) | GPKE (BK6-24-174) Kap. 8.4; GaBi Gas 2.1 Tenorziffer 3a |
| `NetworkLosses` | Σ Einspeisung − Σ Entnahme over a grid area; an indicator, not a settlement quantity | § 22 Abs. 1 EnWG |
| `project_annual_consumption`, `project_annual_slp` | a year from a partial one, by daily rate (optionally prior-year shaped) or SLP-weighted; a missing slot lowers coverage, not the rate | — |
| `wape`, `mase`, `annual_energy_error` | forecast accuracy against metered values; no prediction interval | — |
