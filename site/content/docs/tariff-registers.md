+++
title = "Tariff registers"
description = "HT/NT and § 14a EnWG Modul 3 in one mechanism: ordered windows over months, day groups and minute bands, resolved in Europe/Berlin local time."
weight = 6
+++

Every time-of-use split in the German market is the same shape: ordered windows
over (months × day group × time band), each naming a register, with a fallback
for the times no window covers. One type does all of them.

## § 14a Modul 3 has three levels

Since **1 April 2025** every Netzbetreiber must offer Modul 3: a high tariff for
periods of high network load, a low tariff for low load, and a standard tariff
for the rest.

```rust
use metering::zaehlzeit::{HT, NT, ST, Zaehlzeitdefinition};
use time::macros::{date, datetime};

// High tariff 17:00–20:00, low tariff 22:00–06:00 (crossing midnight),
// standard for everything else.
let zzd = Zaehlzeitdefinition::modul_3(
    "NB-14A-3", date!(2026 - 01 - 01), (17 * 60, 20 * 60), (22 * 60, 6 * 60),
);

assert_eq!(zzd.registers(), vec![HT, NT, ST]);
assert_eq!(zzd.register_for(datetime!(2026-01-05 17:00 UTC)), Some(HT)); // 18:00 local
assert_eq!(zzd.register_for(datetime!(2026-01-05 22:00 UTC)), Some(NT)); // 23:00 local
assert_eq!(zzd.register_for(datetime!(2026-01-05 9:00 UTC)),  Some(ST)); // 10:00 local
```

A Niedertarif band is normally written `22:00–06:00`, which is not a half-open
range in minutes-since-midnight at all. `ZaehlzeitFenster::spanning` splits it
at midnight, so forgetting to is not a silent no-match.

## The classic Zweitarif is the same thing

`Zaehlzeitdefinition::ht_nt(id, valid_from, from_minute, to_minute)` builds an
HT window on weekdays with NT as the fallback. The BDEW
Musterleistungsbeschreibung window is 06:00–22:00, but there is no national
standard — each Netzbetreiber sets its own and publishes it in the Preisblatt.

## Feiertage book into the off-peak register

German tariff definitions treat a gesetzlicher Feiertag as a Sunday rather than
the weekday it falls on. Because holidays are Land law, that needs the
Bundesland of the delivery point.

```rust
use metering::zaehlzeit::{HT, NT, Zaehlzeitdefinition};
use metering::Bundesland;
use time::macros::{date, datetime};

let zzd = Zaehlzeitdefinition::ht_nt("NB-1", date!(2026 - 01 - 01), 6 * 60, 22 * 60);
let midday = datetime!(2026-06-04 8:00 UTC); // 10:00 CEST on Fronleichnam

assert_eq!(zzd.register_for(midday), Some(HT));                                 // weekday alone
assert_eq!(zzd.clone().in_land(Bundesland::By).register_for(midday), Some(NT)); // Bavarian Feiertag
assert_eq!(zzd.clone().in_land(Bundesland::Be).register_for(midday), Some(HT)); // not in Berlin
```

An all-days window keeps its Feiertage — Modul 3 bands are about network load,
and there is no other band for them to fall into.

## Splitting a series

`split_energy` books each interval into its register. The totals always
reconstruct the Arbeitsmenge, including across both DST transitions:

```rust
let period    = aggregate(&intervals, &AggregationConfig::rlm());
let registers = zzd.split_energy(&intervals);
assert_eq!(registers.values().sum::<Decimal>(), period.arbeitsmenge);
```

A 00:00–06:00 low-tariff band holds **20** quarter-hours on the spring-forward
day, **24** on an ordinary one and **28** on the fall-back day — the skipped and
repeated hours both land inside it.

Energy the definition does not cover, or that falls outside its validity, lands
in a `None` bucket so it is visible rather than lost.

`aggregate` does not compute the split: it returns one Arbeitsmenge, and the
breakdown is a separate question composed at the call site.
