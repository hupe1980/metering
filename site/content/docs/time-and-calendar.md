+++
title = "Time and the calendar"
description = "Why a German day is 23, 24 or 25 hours, how that breaks completeness checks, and the calendar API that resolves it."
weight = 2
+++

German metering periods are **local calendar periods**, and a German day is not
24 hours long.

| Day | Length | Quarter-hours |
|---|---|---|
| ordinary | 24 h | 96 |
| last Sunday in March (spring forward) | **23 h** | **92** |
| last Sunday in October (fall back) | **25 h** | **100** |

A completeness check built on a hard-coded 96 raises a false alarm every spring
and — worse — **hides a genuine four-interval gap every autumn**, because 96 of
an expected 100 looks complete.

```rust
use metering::{IntervalResolution, calendar};
use time::macros::{date, datetime};

// A day's real length — never assumed, always resolved from the tz database.
assert_eq!(calendar::intervals_in_day(date!(2026 - 03 - 29), IntervalResolution::QuarterHour), Some(92));
assert_eq!(calendar::intervals_in_day(date!(2026 - 10 - 25), IntervalResolution::QuarterHour), Some(100));

// March 2026 holds 2 972 quarter-hours, not 31 × 96 = 2 976.
assert_eq!(calendar::intervals_in_month(date!(2026 - 03 - 01), IntervalResolution::QuarterHour), Some(2_972));

// A German day starts at 23:00 UTC in winter, 22:00 UTC in summer.
assert_eq!(calendar::day_start_utc(date!(2026 - 01 - 15)), datetime!(2026-01-14 23:00 UTC));
assert_eq!(calendar::day_start_utc(date!(2026 - 07 - 15)), datetime!(2026-07-14 22:00 UTC));
```

The transitions come from the IANA tz database via `time-tz`, so historical ones
are right too: Germany's 1980 transition was on 6 April, not the last Sunday in
March.

## The Gastag is a different day

Gas balances on the **Gastag** — 06:00 local to 06:00 the next morning — not
on the calendar day. The calendar module owns that boundary too:

```rust
use metering::calendar;
use time::macros::{date, datetime};

// Winter: 06:00 CET is 05:00 UTC.
assert_eq!(calendar::gas_day_start_utc(date!(2026 - 01 - 15)), datetime!(2026-01-15 5:00 UTC));

// 05:30 local still belongs to the previous Gastag.
assert_eq!(calendar::local_gas_day(datetime!(2026-07-15 3:30 UTC)), date!(2026 - 07 - 14));
```

The clocks change at 02:00/03:00 local — *before* the 06:00 boundary — so the
23- or 25-hour Gastag is the one named after the **Saturday**, not the
transition Sunday. The SLP-Gas Leitfaden calls this out explicitly.

## No fixed second count for a calendar period

`IntervalResolution::fixed_seconds()` returns `None` for `Day`, `Month` and
`Year`. That is deliberate. Returning 86 400 would be right on 363 days a year,
and the two it is wrong on are exactly the ones that matter.

`nominal_seconds()` exists for buffer sizing and ordering, and its documentation
says in as many words never to use it for interval counts.

## Counting days, not durations

```rust
use metering::calendar;
use time::macros::date;

let from = calendar::day_start_utc(date!(2026 - 03 - 23));
let to   = calendar::day_start_utc(date!(2026 - 04 - 06));

assert_eq!((to - from).whole_days(), 13);         // the naive count, one short
assert_eq!(calendar::days_between(from, to), 14); // the calendar count
```

Fourteen calendar days spanning the spring transition are 335 hours, and integer
division reports thirteen. A daily average built on that count is 7.7 % too
high, and so is any annual projection built on it.

## Holidays are Land law

Only nine statutory holidays are common to all sixteen Bundesländer. The rest
are set by Landesrecht, so the calendar of a delivery point is the calendar of
the Land it sits in.

```rust
use metering::{Bundesland, Holiday, SlpDayType, slp_day_type};
use time::macros::date;

// Fronleichnam 2026 — a holiday in Bavaria, an ordinary Thursday in Berlin.
let fronleichnam = date!(2026 - 06 - 04);
assert_eq!(Bundesland::By.holiday(fronleichnam), Some(Holiday::Fronleichnam));
assert_eq!(Bundesland::Be.holiday(fronleichnam), None);

assert_eq!(slp_day_type(fronleichnam, Bundesland::By), SlpDayType::SonnFeiertag);
assert_eq!(slp_day_type(fronleichnam, Bundesland::Be), SlpDayType::Werktag);
```

Everything is computed — Easter by the Anonymous Gregorian algorithm, the
movable feasts as offsets from it, Buß- und Bettag from the weekday of
23 November — so there is no year table to run out. Easter is pinned against
published dates from 2020 to 2285 and asserted to be a Sunday within
22 March – 25 April for 300 consecutive years.

The Bundesland scoping is the BDEW's own. From the *Hinweise zu den
aktualisierten Standardlastprofilen Strom* (17.03.2025), §1:

> Alle neuen Profile arbeiten mit drei Typtagen: Werktage (WT), Samstage (SA)
> sowie Sonn- und Feiertage (FT). Es gilt der **bundeslandspezifische
> Feiertagskalender** nach Definition des BDEW.

### Municipal scope is not modelled

Fronleichnam in parts of Sachsen and Thüringen, and Mariä Himmelfahrt in the
predominantly Catholic municipalities of Bayern, are statutory below Land level.
`Bundesland` has no finer resolution, so those are reported as *not* holidays.

### This is not a Fristenkalender

Counting Werktage to a GPKE deadline — and the EDI@Energy rule that a holiday in
one Bundesland counts nationwide — are market-*communication* concerns. They
belong in a process engine, not in a library that computes kWh. Nothing here
counts business days.
