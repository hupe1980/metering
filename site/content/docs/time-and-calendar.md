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

### The boundary travels with the calculation

A Gastag is not a shorter or longer day than a Liefertag — it is the same day
cut six hours later. That makes the choice a *boundary*, not a resolution, and
`DayBoundary` carries it into the two places a daily grid is actually built:

```rust
use metering::calendar::{self, DayBoundary};
use metering::{FillGapsConfig, IntervalResolution, ResampleConfig};
use time::macros::date;

// Daily gas totals per Gastag rather than per calendar day.
let cfg = ResampleConfig::to_gas_daily();
assert_eq!(cfg.day_boundary, DayBoundary::Gastag);

// ...and a gap fill that walks 06:00-to-06:00 slots.
let fill = FillGapsConfig::new(
    IntervalResolution::Day,
    calendar::gas_day_start_utc(date!(2026 - 10 - 23)),
    calendar::gas_day_start_utc(date!(2026 - 10 - 27)),
)
.on(DayBoundary::Gastag);
assert_eq!(fill.day_boundary, DayBoundary::Gastag);

// ...and the boundary carries up to the month.
assert_eq!(
    DayBoundary::Gastag.month_start_utc(date!(2026 - 02 - 14)),
    calendar::gas_day_start_utc(date!(2026 - 02 - 01)),
);
```

It carries all the way up: a gas month runs 06:00 on the first to 06:00 on the
first of the next, so a monthly gas total is a whole number of Gastage rather
than a calendar month shifted. Summing a gas Lastgang over the calendar day
instead books the 00:00–06:00 draw into the wrong Bilanzierungstag — six hours,
every day, on every delivery point.

## No fixed second count for a calendar period

`IntervalResolution::fixed_seconds()` returns `None` for `Day`, `Month` and
`Year`. That is deliberate. Returning 86 400 would be right on 363 days a year,
and the two it is wrong on are exactly the ones that matter.

`nominal_seconds()` exists for buffer sizing and ordering, and its documentation
says in as many words never to use it for interval counts.

The fixed resolutions have the opposite property: **exactly one value per second
count**. `IntervalResolution::Custom` carries an opaque `CustomSeconds` that
refuses 0, 900, 1800 and 3600, so `Custom(900)` cannot be built alongside
`QuarterHour`. The two would otherwise have been distinct values meaning one
thing — two database keys for one 15-minute grid, and only one of them
surviving a round trip, since `Custom(900)` writes `PT900S` and `PT900S` reads
back as `QuarterHour`. `from_seconds` is the one constructor and it normalises.

```rust
use metering::{CustomSeconds, IntervalResolution};

assert_eq!(CustomSeconds::new(900), None, "900 s is the QuarterHour");
assert_eq!(IntervalResolution::from_seconds(900), Some(IntervalResolution::QuarterHour));

// A custom length always writes seconds; the parser still takes the hour form.
let two_hours = IntervalResolution::from_seconds(7200).unwrap();
assert_eq!(two_hours.to_string(), "PT7200S");
assert_eq!("PT2H".parse::<IntervalResolution>(), Ok(two_hours));
```

## A local time that does not exist, and one that happens twice

Turning a Berlin wall-clock time into an instant has two awkward cases, and both
are decided in one place:

- the repeated autumn hour resolves to the **earlier** pass, so consecutive
  periods tile without overlap;
- the skipped spring hour is pushed **forward by the gap**, so 02:30 on the
  transition Sunday becomes 03:30 — the convention `java.time`, `chrono` and
  Python's `zoneinfo` share.

```rust
use metering::calendar;
use time::macros::datetime;

// Monday 02:30 CEST, one day back: Sunday 02:30 does not exist, so 03:30 does.
let back = calendar::shift_back_days(datetime!(2026-03-30 0:30 UTC), 1);
assert_eq!(back, datetime!(2026-03-29 1:30 UTC));
assert_eq!(calendar::to_berlin(back).hour(), 3);
```

This matters wherever a window is anchored on a local time: the Vergleichstag
window of `substitute`, and the year-earlier window of `forecast`.

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

## Addressing a period the way the market does

A Bilanzierungsmonat is named, not constructed: *"Juni 2021"*. `DayBoundary`
turns that name into the half-open UTC range it stands for, on either boundary,
without the caller doing any date arithmetic:

```rust
use metering::calendar::DayBoundary;
use time::Month;
use time::macros::datetime;

// March 2026 contains the spring-forward Sunday, so the electricity
// Bilanzierungsmonat is one hour short of 31 days.
let (from, to) = DayBoundary::Midnight.bilanzierungsmonat(2026, Month::March);
assert_eq!(from, datetime!(2026-02-28 23:00 UTC));
assert_eq!(to,   datetime!(2026-03-31 22:00 UTC));
assert_eq!((to - from).whole_hours(), 31 * 24 - 1);

// The gas month is the same span, shifted six hours.
let (gas_from, gas_to) = DayBoundary::Gastag.bilanzierungsmonat(2026, Month::March);
assert_eq!(gas_from, datetime!(2026-03-01 5:00 UTC));
assert_eq!(gas_to,   datetime!(2026-04-01 4:00 UTC));
```

That is the market's own rule rather than an extrapolation. EDI@Energy
*Allgemeine Festlegungen* v6.1c Kap. 3.1:

> Die Angabe des Bilanzierungsmonats erfolgt unter Angabe von Jahr und Monat
> (z. B. Juni 2021), sodass damit der Zeitraum vom 01.06.2021 00:00 Uhr bis
> 01.07.2021 00:00 Uhr gesetzlicher deutscher Zeit abgedeckt ist, wenn es sich
> um den Bilanzierungsmonat in der Sparte Strom handelt, in der Sparte Gas ist
> damit der Zeitraum vom 01.06.2021 06:00 Uhr bis 01.07.2021 06:00 Uhr
> gesetzlicher deutscher Zeit abgedeckt.

`day_range_utc`, `month_range_utc` and `year_range_utc` return the same pair for
a period identified by a date, and feed straight into
`AggregationConfig::over_period`.

## One grid, one implementation

`DayBoundary::bucket_bounds` answers *"which slot of this resolution contains
this instant"*, and it is the only implementation of that question in the crate:
`resample` buckets with it and `split_session` places a charging session on it.
A second implementation would drift from the first, and the two would then
disagree about which slot a kWh belongs to.

```rust
use metering::calendar::DayBoundary;
use metering::IntervalResolution;
use time::macros::datetime;

let (from, to) = DayBoundary::Midnight
    .bucket_bounds(datetime!(2026-06-01 12:07 UTC), IntervalResolution::QuarterHour);
assert_eq!(from, datetime!(2026-06-01 12:00 UTC));
assert_eq!(to,   datetime!(2026-06-01 12:15 UTC));
```
