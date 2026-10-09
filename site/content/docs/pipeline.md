+++
title = "The whole pipeline"
description = "A tour of examples/pipeline.rs: one Liefertag from the Zählerstandsgang to the § 14a register split, on the 25-hour autumn day."
weight = 2
[extra]
group = "Start"
+++

[`examples/pipeline.rs`](https://github.com/hupe1980/metering/blob/main/examples/pipeline.rs)
runs every stage of a Messstellenbetreiber's day on **25 October 2026**, the
25-hour autumn day, and asserts its own invariants. No stage is told the day's
length; each resolves it through `DayBoundary::Strom.day`.

```bash
cargo run --example pipeline
```

```text
Zählerstandsgang ─► Lastgang ─► validate ─► substitute ─► aggregate ─► registers
```

## The stages

1. **Readings.** `build_zaehlerstandsgang` produces one quarter-hourly
   Zählerstand more than the day has slots, with two planted defects: a
   six-digit register that wraps past 999 999, and one corrupt reading.
2. **Lastgang.** `to_lastgang` with `LastgangConfig::strom().register_digits(6)`
   and a 30 kW capacity cap. The wrap is reconstructed; the corrupt reading is
   refused, because as a wrap it would imply more than 30 kW. See
   [Series and readings](@/docs/series-and-readings.md).
3. **Validate.** `Rules::strom(liefertag, now, Some(30 kW))`. The refused span
   is an ordinary `Gap` finding, so the grade is C.
4. **Substitute.** `Policy::strom(Bundesland::Nw, ImplausibleValue)`. The
   two-slot gap is under 2 h with measured values on both sides: interpolated,
   `STS+Z32` `Z92`, reason `STS+Z40` `ZA1`.
5. **Aggregate.** `aggregate(&filled.series, liefertag)` — Arbeitsmenge,
   Spitzenleistung with its interval, full coverage.
6. **Registers.** A § 14a Modul 3 `Zaehlzeitdefinition` — HT 17:00–20:00, NT
   22:00–06:00, billed in Q1 and Q4, holidays of Nordrhein-Westfalen — splits
   the energy into HT, NT and ST.
7. **Re-validate.** Warnings may remain; no errors.

## What it asserts

`check_invariants`:

- the filled series has exactly the day's 100 slots;
- coverage is complete after substitution;
- the register split sums to the Arbeitsmenge, to the digit;
- no quarter-hour is unassigned or straddles a band;
- the 22:00–06:00 band holds **36** quarter-hours, not 32 — the repeated
  02:00–03:00 hour falls inside it.
