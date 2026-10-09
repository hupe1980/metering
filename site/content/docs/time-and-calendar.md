+++
title = "Time and the calendar"
description = "Why a German day is 23, 24 or 25 hours, why the Gastag starts at 06:00, and the one API — DayBoundary to Period — that answers both."
weight = 3
[extra]
group = "Concepts"
+++

German metering periods are **local calendar periods**; the transitions come
from the IANA tz database via `time-tz`.

| Day | Length | Quarter-hours |
|---|---|---|
| ordinary | 24 h | 96 |
| last Sunday in March (spring forward) | **23 h** | **92** |
| last Sunday in October (fall back) | **25 h** | **100** |

A fixed 96 raises a false alarm every spring and **hides a four-slot gap every
autumn**.

## One API: `DayBoundary` → `Period`

A `DayBoundary` — `Strom` at 00:00 local, `Gas` at 06:00 — names a day, month
or year as a half-open UTC `Period`. Years 1900 to 9998; outside them every
constructor answers `None`.

```rust
use metering::{DayBoundary, Resolution};
use metering::time::calendar::DayKind;
use time::Month;
use time::macros::{date, datetime};

let spring = DayBoundary::Strom.day(date!(2026 - 03 - 29)).unwrap();
assert_eq!(spring.count(Resolution::QUARTER_HOUR), Some(92));
assert_eq!(spring.kind(), DayKind::ShortDay);

// March 2026 holds 2 972 quarter-hours, not 31 × 96 = 2 976.
let march = DayBoundary::Strom.month(2026, Month::March).unwrap();
assert_eq!(march.count(Resolution::QUARTER_HOUR), Some(2_972));

// A German day starts at 23:00 UTC in winter and 22:00 UTC in summer.
assert_eq!(DayBoundary::Strom.day(date!(2026 - 01 - 15)).unwrap().start(), datetime!(2026-01-14 23:00 UTC));
assert_eq!(DayBoundary::Strom.day(date!(2026 - 07 - 15)).unwrap().start(), datetime!(2026-07-14 22:00 UTC));

// The repeated autumn hour starts at the transition instant.
let autumn = DayBoundary::Strom.day(date!(2026 - 10 - 25)).unwrap();
assert_eq!(autumn.transition(), Some(datetime!(2026-10-25 1:00 UTC)));
```

`Resolution::fixed_seconds()` is `None` for `Day`, `Month` and `Year`; count
with `Period::count`.

## The Gastag

The **Gastag** runs 06:00 local to 06:00 (GaBi Gas, Art. 3 Nr. 6 VO (EU)
312/2014), and the gas month with it — *Allgemeine Festlegungen* 6.1d, Kap. 3.1:
*"in der Sparte Gas ist damit der Zeitraum vom 01.06.2021 06:00 Uhr bis
01.07.2021 06:00 Uhr gesetzlicher deutscher Zeit abgedeckt"*. The 23- or
25-hour Gastag is the one named after the **Saturday** (SLP-Gas Leitfaden:
*"Daher ist die Zeitumstellung in den Werten für den Samstag vor der Umstellung
zu berücksichtigen."*).

```rust
use metering::DayBoundary;
use time::macros::{date, datetime};

assert_eq!(DayBoundary::Gas.day(date!(2026 - 01 - 15)).unwrap().start(), datetime!(2026-01-15 5:00 UTC));
assert_eq!(DayBoundary::Gas.day(date!(2026 - 10 - 24)).unwrap().duration().whole_hours(), 25);

// 05:30 local on 15 July still belongs to the Gastag of the 14th.
assert_eq!(DayBoundary::Gas.day_of(datetime!(2026-07-15 3:30 UTC)), Some(date!(2026 - 07 - 14)));
```

A `Series` carries its boundary; resampling, validation and substitution use it.

## Which day an instant belongs to

Not `instant.date()` — that is the UTC day. Ask the boundary:

```rust
use metering::{DayBoundary, Resolution};
use metering::time::calendar;
use time::macros::{date, datetime};

// 23:30 UTC on 14 July is already 01:30 on 15 July in Berlin.
assert_eq!(DayBoundary::Strom.day_of(datetime!(2026-07-14 23:30 UTC)), Some(date!(2026 - 07 - 15)));

// The quarter-hour containing an instant, laid out from the local day start.
let slot = DayBoundary::Strom.bucket(datetime!(2026-06-01 12:07 UTC), Resolution::QUARTER_HOUR).unwrap();
assert_eq!(slot.start(), datetime!(2026-06-01 12:00 UTC));

// "One week earlier, same local time" is 169 hours across the autumn change.
let back = calendar::shift_back_days(datetime!(2026-10-28 11:00 UTC), 7).unwrap();
assert_eq!((datetime!(2026-10-28 11:00 UTC) - back).whole_hours(), 169);
```

A local time that occurs twice resolves to the **earlier** instant; one the
spring change skips is pushed forward by the gap.

## Holidays are Land law

A delivery point's holiday calendar is its Land's; every holiday rule carries
the years it is valid in.

```rust
use metering::slp::strom::SlpDayType;
use metering::time::holiday::{Bundesland, Holiday, SlpCalendar};
use time::macros::date;

// Fronleichnam 2026: a holiday in Bavaria, an ordinary Thursday in Berlin.
let fronleichnam = date!(2026 - 06 - 04);
assert_eq!(Bundesland::By.holiday(fronleichnam), Some(Holiday::Fronleichnam));
assert_eq!(SlpCalendar::new(Bundesland::By).day_type(fronleichnam), SlpDayType::SonnFeiertag);
assert_eq!(SlpCalendar::new(Bundesland::Be).day_type(fronleichnam), SlpDayType::Werktag);

// Frauentag is a Berlin holiday from 2019, not before.
assert!(Bundesland::Be.is_holiday(date!(2019 - 03 - 08)));
assert!(!Bundesland::Be.is_holiday(date!(2018 - 03 - 08)));
```

Holidays below Land level (Fronleichnam in parts of Sachsen and Thüringen,
Mariä Himmelfahrt in Catholic municipalities of Bayern) are not modelled;
supply your own day type. This is **not a Fristenkalender**.

## Leap seconds

Allgemeine Festlegungen Kap. 3.9 permits `23:59:60`; `time::OffsetDateTime`
has no such second, so it fails at the parse rather than becoming `23:59:59`.
