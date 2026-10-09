+++
title = "Heat"
description = "HeizkostenV quantities: the § 6a monthly consumption information, the § 9 warm-water heat and fuel quantity, and the § 9a estimate with its 25 % limit."
weight = 9
[extra]
group = "Tasks"
+++

`heat::heizkosten` computes the HeizkostenV quantities the Verordnung defines
precisely. Out: the cost distribution of §§ 7 and 8 (money), and § 9b
Gradtagszahlen and the § 6a Abs. 3 weather adjustment (not freely citable
technical rules).

| Clause | Function | Quantity |
|---|---|---|
| § 6a Abs. 2 | `verbrauchsinformation` | the month's consumption and its two comparisons |
| § 9 Abs. 2 | `warm_water_heat_kwh`, `warm_water_heat_kwh_unmetered` | heat attributable to central warm water |
| § 9 Abs. 3 | `brennstoffverbrauch` | the fuel quantity B of the warm-water system |
| § 9a | `verbrauchsschaetzung` | whether an estimate is permitted |

## § 6a — monthly consumption information

The last month's consumption in kWh, compared with the previous month and the
same month of the previous year *"soweit diese Daten erhoben worden sind"*.
Months are Berlin calendar months; an interval across a month boundary is
refused; a partial month is marked incomplete. Heizkostenverteiler units need a
kWh factor, which is an argument. The comparison with a *"normierten oder durch
Vergleichstests ermittelten Durchschnittsnutzers derselben Nutzerkategorie"* is
undefined by the Verordnung; supply that figure yourself.

```rust
use metering::heat::heizkosten::{Erfassung, verbrauchsinformation};
use metering::prelude::*;
use time::Month;
use time::macros::date;

// Daily heat-meter values, 10 kWh a day, through February and March 2026.
let days = (0..59)
    .map(|i| {
        let d = DayBoundary::Strom.day(date!(2026 - 02 - 01) + time::Duration::days(i)).unwrap();
        MeterInterval::new(d.start(), d.end(), dec!(10), QualityFlag::Measured)
    })
    .collect::<Result<Vec<_>, _>>().unwrap();
let series = Series::new(Resolution::Day, DayBoundary::Strom, days).unwrap();

let info = verbrauchsinformation(&series, Erfassung::Waermemenge, 2026, Month::March).unwrap();
let march = info.monat.unwrap();
assert_eq!(march.kwh, dec!(310));
assert!(march.vollstaendig);
assert_eq!(info.vormonat.unwrap().kwh, dec!(280));
assert!(info.vorjahresmonat.is_none()); // not recorded
```

## § 9 — warm water

§ 9 Abs. 2 Satz 1 requires a **Wärmezähler**; the fallbacks:

| Function | Formula | When |
|---|---|---|
| `warm_water_heat_kwh` | `Q = 2,5 × V [m³] × (t_w − 10 °C)`; `t_w` < 10 °C refused | metering the heat *"nur mit einem unzumutbar hohen Aufwand"* |
| `warm_water_heat_kwh_unmetered` | `Q = 32 × A [m²]` | *"in Ausnahmefällen"* |

`WarmWaterAdjustments` applies the cumulative Satz 6 corrections (Erdgas
brennwertbezogen × 1,11; gewerbliche Wärmelieferung ÷ 1,15; monovalente
Wärmepumpe × 0,30). § 9 Abs. 3: `B = Q ÷ Hi`, with the supplier's Heizwert
first and the statutory table as fallback.

```rust
use metering::heat::heizkosten::{
    Brennstoff, Heizwert, WarmWaterAdjustments, brennstoffverbrauch, warm_water_heat_kwh,
};
use rust_decimal::dec;

// 40 m³ of warm water at 60 °C: 2,5 × 40 × 50.
let q = warm_water_heat_kwh(dec!(40), dec!(60), WarmWaterAdjustments::NONE).unwrap();
assert_eq!(q, dec!(5000));

// From Erdgas H at the statutory fallback of 10 kWh/m³.
let b = brennstoffverbrauch(q, Heizwert::Hilfsweise(Brennstoff::ErdgasH)).unwrap();
assert_eq!(b.menge, dec!(500)); // m³
```

## § 9a — estimation

On one of three named bases — comparable periods, comparable rooms, the
building's average. Above 25 % of the area (or volume) it is refused and the
costs follow the fixed-cost key alone; exactly 25 % is admitted — the limit
bites when the share *"überschreitet"* it. The estimate itself is your figure.

```rust
use metering::heat::heizkosten::{Schaetzgrundlage, SchaetzungError, verbrauchsschaetzung};
use rust_decimal::dec;

let ok = verbrauchsschaetzung(Schaetzgrundlage::VergleichbareZeitraeume, dec!(4200), dec!(25), dec!(100)).unwrap();
assert_eq!(ok.anteil_prozent, dec!(25));

assert_eq!(
    verbrauchsschaetzung(Schaetzgrundlage::VergleichbareRaeume, dec!(4200), dec!(26), dec!(100)).unwrap_err(),
    SchaetzungError::UeberSchaetzgrenze,
);
```
