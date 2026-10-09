+++
title = "metering — German energy-metering quantities for Rust"
description = "The open, checkable reference for German energy-metering quantities: exact decimals, Europe/Berlin and Gastag calendar arithmetic, the market's own code lists, and every quoted clause checked against the published PDF."
template = "index.html"
+++

```rust
use metering::prelude::*;
use time::{Duration, macros::date};

// The autumn Liefertag is 25 hours: 100 quarter-hours, not 96.
let day = DayBoundary::Strom.day(date!(2026 - 10 - 25)).unwrap();
let slots = (0..100)
    .map(|i| MeterInterval::quarter_hour(day.start() + Duration::minutes(15 * i), dec!(0.25), QualityFlag::Measured))
    .collect::<Result<Vec<_>, _>>()
    .unwrap();
let series = Series::new(Resolution::QUARTER_HOUR, DayBoundary::Strom, slots).unwrap();

let period = aggregate(&series, day).unwrap();
assert_eq!(period.arbeitsmenge, dec!(25));            // exact Decimal kWh
assert_eq!(period.spitzenleistung_kw, Some(dec!(1)));  // kW, from kWh × 3600 ÷ s
assert_eq!(period.coverage.pct(), Some(dec!(100)));    // against the declared day
```
