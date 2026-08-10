+++
title = "Getting started"
description = "Install the crate, aggregate a billing period, and understand the two conventions everything else rests on."
weight = 1
+++

## Install

```bash
cargo add metering
```

Serde support for every public type is behind a feature flag:

```bash
cargo add metering --features serde
```

The MSRV is Rust 1.94 (edition 2024), pinned in `rust-toolchain.toml` and
verified by a dedicated CI lane.

## A first billing period

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
assert_eq!(period.arbeitsmenge, dec!(2.345));
```

`AggregationConfig::rlm()` computes the Spitzenleistung as well as the
Arbeitsmenge; `AggregationConfig::arbeitsmenge_only()` is for SLP and gas, where
there is no Leistungspreis.

## Two conventions to internalise first

### Timestamps are UTC, periods are local

Every `from` and `to` in this crate is a UTC instant. Every *period* — a
Liefertag, a Liefermonat, a tariff band — is Europe/Berlin local. That is the
market's own split, stated in the EDI@Energy *Allgemeine Festlegungen* v6.1b,
Kap. 3:

> Die Angabe von Zeiten in einer EDIFACT Nachricht erfolgt in koordinierter
> Weltzeit (UTC). […] Alle in den Prozessen genannten Zeitpunkte […] nutzen die
> gesetzliche deutsche Zeit.

Getting this wrong books the first hour of every German day into the previous
one. See [Time and the calendar](@/docs/time-and-calendar.md).

### The unit is the Sparte's, and the interval does not carry it

`MeterInterval::value` is kWh for Strom, kWh_Hs for Gas *after* conversion,
kWh_th for Wärme and **m³ for Wasser**. The field is `value`, not `value_kwh`,
because water is a supported Sparte and a field named `_kwh` holding cubic
metres is a lie the compiler cannot catch.

The unit lives on the `MeasurementPoint` or is derived from the OBIS medium —
not on the hottest type in the crate, where a redundant copy could drift.

One consequence: `demand_kw()` is only meaningful where the unit is energy,
because cubic metres over hours is a flow rate, not a power.

## Where to go next

- [Time and the calendar](@/docs/time-and-calendar.md) — the single most
  consequential thing the crate gets right.
- [Readings and intervals](@/docs/readings.md) — Zählerstand → Lastgang, and
  where register rollover belongs.
- [Validation and quality](@/docs/validation.md) — the rule engine and the
  A/B/C/F grade over it.
