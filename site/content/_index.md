+++
title = "metering — German energy metering for Rust"
description = "Pure Rust domain library for German energy metering quantities: DST-correct Europe/Berlin and Gastag calendar arithmetic, exact decimal values, and the market's own code lists — with every regulatory citation checked against the published PDF."
template = "index.html"
+++

```rust
use metering::{AggregationConfig, MeterInterval, ObisCode, aggregate};
use rust_decimal::dec;
use time::macros::datetime;

let intervals = vec![
    MeterInterval::quarter_hour(datetime!(2026-06-01 0:00 UTC), dec!(2.345))
        .with_obis(ObisCode::STROM_BEZUG_TOTAL),
];

let period = aggregate(&intervals, &AggregationConfig::rlm());

period.arbeitsmenge;        // 2.345 kWh, exact Decimal
period.spitzenleistung_kw;  // Some(9.38) — and `spitzenleistung_at` says when
period.coverage_pct;        // measured against a declared period, not the data
```
