+++
title = "Redispatch and Ausfallarbeit"
description = "The energy a Redispatch measure prevented — the Wert der Leistungslimitierung, the three Abrechnungsvarianten, and the one that ends in 2028."
weight = 14
+++

A Redispatch measure moves a plant off the output it would otherwise have had.
The **Ausfallarbeit** is the difference, and it is what § 13a Abs. 1a EnWG
settles bilanziell and § 13a Abs. 2 settles financially.

BNetzA **BK6-23-241** (Beschluss 07.05.2026) and its annex *"Bilanzieller
Ausgleich von Redispatch-Maßnahmen (BilAReM)"* replace the three previous
Festlegungen — *"Die Vorgaben ersetzen die Regelungen in den Festlegungen
BK6-20-059, BK6-20-060 und BK6-20-061"* — and Kapitel 3 of the annex is where
the arithmetic lives.

## It is signed, and the sign is the direction

> Ausfallarbeit ist – arbeitsbezogen – die Differenz zwischen der theoretischen
> Erzeugung einer TR und dem Wert der Leistungslimitierung […]; bei negativem
> Redispatch ist die Ausfallarbeit positiv, bei positivem Redispatch ist die
> Ausfallarbeit negativ (Mehrarbeit).

A positive Redispatch makes a plant produce **more** than it would have. That is
Mehrarbeit, and it is carried as a *negative* Ausfallarbeit rather than as a
separate quantity — so a series that mixes both directions sums to the net
effect without anybody choosing a sign convention at the call site.

Everything is per quarter-hour and per Technische Ressource: *"Die Ausfallarbeit
wird für jede TR bestimmt"*, and *"Soweit in diesem Kapitel Leistungswerte
genannt werden, sind Viertelstundenmittelwerte gemeint"*.

## The Wert der Leistungslimitierung

Kapitel 3.1, and the one place the Aufforderungs- and the Duldungsfall part
company:

| Fall | Richtung | `P_lim,i` |
|---|---|---|
| Aufforderung | positiv | `min{P_ist,i ; P_min,i}` |
| Aufforderung | negativ | `max{P_ist,i ; P_max,i}` |
| Duldung | either | `P_ist,i` |

```rust
use metering::ausfallarbeit::{Redispatchfall, Redispatchrichtung, leistungslimitierung_kw};
use rust_decimal::dec;

// Told to cap at 300 kW, actually ran at 420.
let aufforderung = leistungslimitierung_kw(
    Redispatchfall::Aufforderung, Redispatchrichtung::Negativ, dec!(420), dec!(300),
);
assert_eq!(aufforderung, dec!(420));
```

The maximum, not the cap — because in the Aufforderungsfall the plant
implemented the instruction itself, and the Beschluss puts the consequence of
implementing it badly where the decision was: *"Damit trägt im
Aufforderungsfall der BKV des LF das Risiko, dass die Redispatch-Anweisung
korrekt umgesetzt wird."* In the Duldungsfall the grid operator intervened, and
what happened **is** the limit.

A Referenzprofilverfahren takes neither: the Netzbetreiber's own figure is the
limit outright, with no reference to what the plant did. That is a separate
function, because a caller using it has no `P_ist` to pass.

## Three Abrechnungsvarianten, and what this crate computes

| Variante | Datengrundlage | Here |
|---|---|---|
| Spitzabrechnung | *"gemessene Wetterdaten der TR"* | theoretical series **supplied** |
| vereinfachte Spitzabrechnung | *"mit Referenzmesswerten oder Wetterdaten für den Standort"* | supplied |
| Pauschal-Abrechnung | the last quarter-hour, or an Anlagenfaktor | computed |

Turning a measured wind speed into a theoretical power needs a power curve and a
site model — an operator's data and a Wetterdienst's. The crate does the
arithmetic and takes that series, exactly as it takes SLP value tables. The
Wind-Bin-Verfahren for offshore wind is out for the same reason.

The **default** is the middle one: *"Trifft der Anlagenbetreiber keine
Zuordnungsentscheidung, findet die vereinfachte Spitzabrechnung Anwendung."*

## The Pauschal-Abrechnung ends with 2028

```rust
use metering::ausfallarbeit::{Abrechnungsvariante, PAUSCHAL_BESTANDSSCHUTZ_ENDE};
use time::macros::date;

let pauschal = Abrechnungsvariante::PauschalAbrechnung;
assert!(pauschal.is_available_on(date!(2028 - 12 - 31)));
assert!(!pauschal.is_available_on(date!(2029 - 01 - 01)));
assert_eq!(PAUSCHAL_BESTANDSSCHUTZ_ENDE, date!(2028 - 12 - 31));
```

The Beschluss is explicit about why, and the reason is a modelling one rather
than an administrative one: the method assumes the primary energy available
during the measure is unchanged, and *"Das mag bei kurzen Redispatch-Maßnahmen
im Einzelfall näherungsweise zutreffen, bei längeren Maßnahmen aber nicht."*

`is_available_on` answers the part of the admissibility test that is a **date**.
It cannot answer the rest: a TR also has to have been in the Pauschal-Abrechnung
when the Festlegung was published, and a TR in the Planwertmodell is excluded
outright. Both are facts about a portfolio, which this crate cannot see.

## The Pauschal formulas

For a Windenergieanlage an Land (Kap. 3.2.2.3):

```text
W_A,i = max{0; [min(P_0 ; P_inst ; P_mbA,i ; P_bean,i) − P_lim,i] × ¼h}
```

```rust
use metering::ausfallarbeit::pauschal_wind_kwh;
use rust_decimal::dec;

// Running at 900 kW, capped at 300, nothing else binding.
assert_eq!(pauschal_wind_kwh(dec!(900), dec!(1000), None, None, dec!(300)), dec!(150.00));

// A marktbedingte Anpassung to 600 kW lowers what the plant could have made.
assert_eq!(
    pauschal_wind_kwh(dec!(900), dec!(1000), Some(dec!(600)), None, dec!(300)),
    dec!(75.00),
);
```

A solar plant does not carry a last quarter-hour forward at all: its theoretical
power is the installed nominal power times an **Anlagenfaktor** from a published
table, bounded by the inverter.

### The Anlagenfaktor table is in UTC+1

| Jahreszeit | Uhrzeit (UTC+1) | `AF` |
|---|---|---|
| Sommer (01.03.–31.10.) | 19:00–6:00 · 6:00–9:00 · 9:00–15:00 · 15:00–19:00 | 0,0000 · 0,2456 · 0,6189 · 0,2456 |
| Winter (01.11.–28./29.02.) | 16:45–9:00 · 9:00–10:00 · 10:00–14:00 · 14:00–16:45 | 0,0000 · 0,2796 · 0,5030 · 0,2796 |

The Festlegung prints *"Uhrzeit (UTC+1)"* — MEZ all year, **not** local time. In
summer the bands therefore sit an hour off the wall clock, and a caller that
reads them as Europe/Berlin shifts every summer band by an hour. That is the one
thing about this table worth getting right, and it is why `anlagenfaktor` takes
an instant rather than a time of day.

```rust
use metering::ausfallarbeit::anlagenfaktor;
use rust_decimal::dec;
use time::macros::datetime;

// 08:30 Berlin in June is 06:30 UTC — 07:30 MEZ, the early band.
assert_eq!(anlagenfaktor(datetime!(2026-06-15 6:30 UTC)), dec!(0.2456));
// 10:30 Berlin is 09:30 MEZ, already the middle one.
assert_eq!(anlagenfaktor(datetime!(2026-06-15 8:30 UTC)), dec!(0.6189));
// Winter closes at 16:45 MEZ, not on the hour.
assert_eq!(anlagenfaktor(datetime!(2026-01-15 15:50 UTC)), dec!(0.0000));
```

## `P_0` is measured, not merely billable

The Pauschal-Abrechnung carries forward the *"letzten vollständig gemessenen"*
quarter-hour before the measure. An Ersatzwert is **billable** — that is what
Ersatzwertbildung is for — and it is not a measurement. Carrying one forward
would make the compensation rest on a figure no meter produced, so `p0_kw`
requires `QualityFlag::Measured` exactly.

It answers two of the three conditions the Festlegung sets, and says so: whether
the plant could feed *uneingeschränkt* in that quarter-hour is a fact about
curtailment and availability that no interval carries.

```rust
use metering::ausfallarbeit::p0_kw;
# use metering::{MeterInterval, QualityFlag};
# use rust_decimal::dec;
# use time::{Duration, macros::datetime};
# let base = datetime!(2026-06-01 10:00 UTC);
# let iv = |m: i64, v, q| MeterInterval {
#     from: base + Duration::minutes(m), to: base + Duration::minutes(m + 15),
#     value: v, quality: q, obis_code: None };
let series = [
    iv(0,  dec!(100), QualityFlag::Measured),
    iv(15, dec!(120), QualityFlag::Measured),
    iv(30, dec!(130), QualityFlag::Substituted),   // billable, not measured
];
let p0 = p0_kw(&series, base + Duration::minutes(45)).expect("one qualifies");
assert_eq!(p0.value, dec!(120));
assert_eq!(p0.demand_kw(), Some(dec!(480)));
```

## Where the boundary is

The **kWh** is this crate's. The *bilanzieller Ausgleich* built on it — the
Fahrplan the Netzbetreiber delivers, the exchange processes the annex to the
BilAReM applies from 01.10.2026 — is market communication, and belongs where
every other Prüfidentifikator does.
