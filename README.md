# metering

[![Crates.io](https://img.shields.io/crates/v/metering.svg)](https://crates.io/crates/metering)
[![Docs.rs](https://docs.rs/metering/badge.svg)](https://docs.rs/metering)
[![CI](https://github.com/hupe1980/metering/actions/workflows/ci.yml/badge.svg)](https://github.com/hupe1980/metering/actions/workflows/ci.yml)
[![MSRV](https://img.shields.io/badge/MSRV-1.88-blue.svg)](#install)
[![License](https://img.shields.io/crates/l/metering.svg)](#contributing-and-license)

**The open, checkable reference for German energy-metering quantities, in Rust.**
Every number traces to a published clause — MsbG, EnWG, EEG, HeizkostenV, the
BNetzA Festlegungen, the EDI@Energy documents — and every quoted passage is
checked against the PDF it comes from.

A pure library — no I/O, no async, no clock. Quantities are exact `Decimal`s,
every rounding names its places and mode, and time is Europe/Berlin: a day of
23, 24 or 25 hours, a Gastag from 06:00. It computes kWh, m³ and kW, not money.

[Guides](https://hupe1980.github.io/metering) · [API reference](https://docs.rs/metering) · [Changelog](CHANGELOG.md)

## Install

```bash
cargo add metering
cargo add metering --features serde   # wire formats for the inputs, codes and results
```

The `serde` feature enables `serde` and `time/serde`; see
[Design](https://hupe1980.github.io/metering/docs/design/#serde) for the
representation. **MSRV 1.88** (edition 2024), tested in CI; raising it is a
minor-version bump while 0.x.

## Quick start

A day of quarter-hours with one value missing: validate, substitute, aggregate.

```rust
use metering::prelude::*;
use metering::time::holiday::Bundesland;
use metering::vee::substitute::SubstitutionReason;
use metering::vee::validation::Grade;
use time::{Duration, macros::{date, datetime}};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // The Liefertag 1 June 2026, 00:00 to 00:00 Europe/Berlin, as a UTC period.
    let day = DayBoundary::Strom.day(date!(2026 - 06 - 01)).unwrap();

    // 96 quarter-hours of 0.5 kWh, except slot 50, which never arrived.
    let slots = (0..96)
        .filter(|i| *i != 50)
        .map(|i| {
            let from = day.start() + Duration::minutes(15 * i);
            MeterInterval::quarter_hour(from, dec!(0.5), QualityFlag::Measured)
        })
        .collect::<Result<Vec<_>, _>>()?;
    let series = Series::new(Resolution::QUARTER_HOUR, DayBoundary::Strom, slots)?;

    // Validate: the gap is an error finding, so the grade is C.
    let rules = Rules::strom(day, datetime!(2026-07-01 0:00 UTC), None);
    let report = validate(&series, &rules);
    assert_eq!(report.grade(), Grade::C);

    // Substitute: a gap up to 2 h is interpolated (STS+Z32 code Z92).
    let policy = Policy::strom(Bundesland::Be, SubstitutionReason::CommunicationFailure);
    let filled = substitute(&series, &report, &policy)?;
    assert_eq!(filled.substitutes[0].code, Some("Z92"));

    // Aggregate: Arbeitsmenge and Spitzenleistung of the billed day.
    let period = aggregate(&filled.series, day)?;
    assert_eq!(period.arbeitsmenge, dec!(48));
    assert_eq!(period.spitzenleistung_kw, Some(dec!(2)));
    Ok(())
}
```

[`examples/pipeline.rs`](examples/pipeline.rs) runs the whole chain — readings,
validation, substitution, aggregation, tariff registers — on the 25-hour autumn
day: `cargo run --example pipeline`.

## What's in it

| Group | Modules | Guide |
|---|---|---|
| Series, time, IDs | `series` (intervals, `Series`, readings, resampling), `time` (`DayBoundary`, `Period`, holidays), `ids` (MaLo, MeLo, EIC, BDEW codes, OBIS) | [Time](https://hupe1980.github.io/metering/docs/time-and-calendar/) · [Series and readings](https://hupe1980.github.io/metering/docs/series-and-readings/) · [Identifiers](https://hupe1980.github.io/metering/docs/identifiers/) |
| VEE | `vee` — validation, grading, Ersatzwertbildung with MSCONS codes | [Validation and substitution](https://hupe1980.github.io/metering/docs/validation-and-substitution/) |
| Billing | `billing` — Arbeitsmenge, Spitzenleistung, tariff registers and § 14a Modul 3, Blindmehrarbeit, Mehr-/Mindermengen, losses, forecast | [Billing quantities](https://hupe1980.github.io/metering/docs/billing-quantities/) |
| SLP | `slp` — BDEW SLP Strom, SigLinDe SLP Gas | [Gas and units](https://hupe1980.github.io/metering/docs/gas-and-units/) |
| Gas | `gas` — m³ → kWh_Hs, Zustandszahl (DVGW G 685) | [Gas and units](https://hupe1980.github.io/metering/docs/gas-and-units/) |
| Heat | `heat` — HeizkostenV § 6a, § 9, § 9a | [Heat](https://hupe1980.github.io/metering/docs/heat/) |
| Grid | `grid` — § 14a EnWG, Redispatch Ausfallarbeit, EN 50160, rollout | [Grid](https://hupe1980.github.io/metering/docs/grid/) · [Power quality](https://hupe1980.github.io/metering/docs/power-quality/) |
| Allocation | `allocation` — UTILTS Berechnungsformel, § 42b community, § 42c sharing, sessions | [Allocation](https://hupe1980.github.io/metering/docs/allocation/) |
| EEG | `eeg` — MiSpeL (adopted 01.10.2026), EEG § 51/§ 51a, EnFG § 46 | [EEG and EnFG](https://hupe1980.github.io/metering/docs/eeg/) |

## Scope

| Not here | Why | Where instead |
|---|---|---|
| Money — prices, Netzentgelte, Umlagen, invoices | the output is kWh, m³ and kW | your billing layer |
| EDIFACT / XML market messages | parsing a MSCONS or UTILTS is not arithmetic | [`mako`](https://github.com/hupe1980/mako) |
| Fristen — counting Werktage to a deadline | a process-engine concern; the holiday calendar here serves SLP day types and tariff registers | your process engine |
| SMGW certificates, device inventory | PKI and asset tracking, not quantities | your device management |

The [regulatory basis](https://hupe1980.github.io/metering/docs/regulatory-basis/)
lists every source and the version in force. Nothing here is legal advice.

## Testing

```bash
just ci          # the CI gate: fmt, clippy, purity, tests, docs, example, package, deny
just references  # fetch the primary sources the citations are checked against
just quotes      # check every quoted German passage against those PDFs
```

The Rust blocks in this README and on the guide pages run as doctests.
`just quotes` needs `pdftotext` and the fetched corpus; it is not a CI job.

## Contributing and license

Issues and pull requests welcome; a change to a regulated calculation cites the
provision it implements. Licensed under [Apache-2.0](LICENSE-APACHE) or
[MIT](LICENSE-MIT) at your option.
