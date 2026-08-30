+++
title = "metering — German energy metering for Rust"
description = "Pure Rust domain library for German energy metering: DST-correct Europe/Berlin calendar arithmetic, Zählerstandsgang to Lastgang, gas m³→kWh_Hs, Ersatzwertbildung, EN 50160, §14a Modul 3 tariff registers and netzorientierte Steuerung, and §42b/§42c allocation."
template = "index.html"
+++

```rust
use metering::{AggregationConfig, MeterInterval, QualityFlag, aggregate};
use rust_decimal::dec;
use time::macros::datetime;

let intervals = vec![MeterInterval {
    from: datetime!(2026-06-01 0:00 UTC),
    to:   datetime!(2026-06-01 0:15 UTC),
    value: dec!(2.345),
    quality: QualityFlag::Measured,
    obis_code: Some("1-0:1.8.0".parse().unwrap()),
}];

let period = aggregate(&intervals, &AggregationConfig::rlm());

period.arbeitsmenge;        // 2.345 kWh, exact Decimal
period.spitzenleistung_kw;  // Some(9.38) — and `spitzenleistung_at` says when
period.coverage_pct;        // measured against a declared period, not the data
```
