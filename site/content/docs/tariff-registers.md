+++
title = "Tariff registers"
description = "HT/NT and § 14a EnWG Modul 3 in one mechanism: ordered windows over months, day groups and minute bands, resolved in Europe/Berlin local time."
weight = 8
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

// The keys borrow from the definition, so they are exactly the strings
// `registers()` lists and a lookup allocates nothing.
let ht: Decimal = registers.get(&Some(HT)).copied().unwrap_or_default();
assert!(ht > Decimal::ZERO);
```

A 00:00–06:00 low-tariff band holds **20** quarter-hours on the spring-forward
day, **24** on an ordinary one and **28** on the fall-back day — the skipped and
repeated hours both land inside it.

Energy the definition does not cover, or that falls outside its validity, lands
in a `None` bucket so it is visible rather than lost.

`aggregate` does not compute the split: it returns one Arbeitsmenge, and the
breakdown is a separate question composed at the call site.

## Is this calendar a valid Modul 3?

A portfolio of curated DSO calendars is worth refusing at the door rather than
at the optimiser. `assess_modul_3` checks a `Zaehlzeitdefinition` against the
BDEW *Anwendungshilfe für die Umsetzung von Modul 3* v1.1 (07.02.2025) §2:

| Rule | Checked |
|---|---|
| three Netzentgelttarife HT/NT/ST | ✓ |
| each of them reachable on some day of some month | ✓ |
| every instant books into one of them | ✓ |
| HT at least two hours per day | ✓ |
| windows *ganzjährig identisch* | ✓ |
| billed in at least two quarters, not necessarily adjacent | ✓ |
| set per calendar year | ✓ |
| only with Modul 1, iMSys, no RLM | ✓ from `Modul3Context` |
| HT ≤ 100 % Aufschlag, NT 10–40 % of ST, ST = SLP-Arbeitspreis | ✗ |
| Netzentgeltgleichheit for an H0 customer | ✗ |
| publication on the vorläufiges Preisblatt | ✗ |

```rust
use metering::zaehlzeit::{Modul3Conformance, Modul3Context, Quarter, assess_modul_3};

let ctx = Modul3Context::default()
    .billed_in([Quarter::Q1, Quarter::Q4])   // need not be adjacent
    .at_a_conforming_delivery_point();   // Modul 1, no RLM, iMSys

let (verdict, findings) = assess_modul_3(&zzd, &ctx);
assert_eq!(verdict, Modul3Conformance::Conforms, "{findings:?}");
```

### The three that are missing on purpose

The first two of them are **price corridors**, and this crate computes
quantities, not money. It has no Arbeitspreis type to compare against, and
inventing one so a validator could use it would be the wrong end of the
telescope. Check those where the price sheet is parsed.

The third is a **Fristen** question, which belongs in a process engine — and the
AWH states its 15.10. date for the first year (2024, for 2025) rather than as a
standing rule, so a general check here would be an extrapolation the document
does not support.

### The two-hour rule is read per day *class*

*"min. an 2 Stunden pro Tag"* is checked on every kind of day the definition can
distinguish: an ordinary weekday, a Saturday, a Sunday and — where
`holiday_land` is set — a statutory holiday. A definition whose HT applies only
Monday to Friday therefore reports `HochtarifBelowTwoHours`, because on a Sunday
it offers no Hochtarif at all.

### Missing input is `Unknown`, not clean

`Modul3Context` is all optional, because a curated portfolio is routinely
incomplete. A rule that could not be checked reports a finding whose
`is_unknown()` is true and lands the verdict on `Unknown`; a rule that was
broken lands it on `Violates`. A breach outranks an unknown, and nothing
reports `Conforms` on data it never saw.

## Whose calendar is it?

`id` is *NB-assigned*, so it identifies a definition only inside one operator's
price sheet: `HT/NT-1` from two Netzbetreiber are two different calendars under
one name. `netzbetreiber` carries the Marktpartner-ID that published it.

```rust
let zzd = zzd.published_by("9900987654321".parse()?);
```

There is deliberately **no `year` field**: that is `valid_from` and `valid_to`,
and a second copy of one fact is a second thing to keep in step. Nor a `source`
URL or hash — where a calendar was fetched from is a property of the fetch, not
of the calendar.
