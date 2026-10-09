//! Numbers printed in a source document, reproduced by the crate.
//!
//! Every test here asserts a value the document itself prints, and names the
//! document and the page or clause it is printed on. A test whose numbers were
//! chosen for this crate belongs in its area's module instead; the published
//! MiSpeL examples sit with the rest of the MiSpeL vectors in `eeg`, named
//! `published_*`.

use metering::vee::validation::{Grade, Rule, Rules, validate};
use metering::{DayBoundary, MeterInterval, ObisCode, QualityFlag, Resolution, Series};
use rust_decimal::{Decimal, dec};
use time::macros::{date, datetime};

/// EDI@Energy *Allgemeine Festlegungen* 6.1d, Kap. 3.1, p. 29: the only
/// `HHMM` values a spartenspezifische prozessuale Zeitangabe may carry —
/// *"sind an der Stelle, an der im DE2380 die Stunden und Minuten (HHMM)
/// angegeben werden, ausschließlich folgende Werte möglich"*:
///
/// | Sparte | MEZ | MESZ |
/// |---|---|---|
/// | Strom | `2300` (23:00 UTC = 00:00 MEZ) | `2200` (22:00 UTC = 00:00 MESZ) |
/// | Gas   | `0500` (05:00 UTC = 06:00 MEZ) | `0400` (04:00 UTC = 06:00 MESZ) |
///
/// Those four instants are exactly the start of a Strom day and a Gastag in
/// winter and in summer.
#[test]
fn day_starts_are_the_four_codings_of_the_allgemeine_festlegungen() {
    let start = |b: DayBoundary, d| b.day(d).unwrap().start();
    assert_eq!(
        start(DayBoundary::Strom, date!(2026 - 01 - 15)),
        datetime!(2026-01-14 23:00 UTC),
        "2300"
    );
    assert_eq!(
        start(DayBoundary::Strom, date!(2026 - 07 - 15)),
        datetime!(2026-07-14 22:00 UTC),
        "2200"
    );
    assert_eq!(
        start(DayBoundary::Gas, date!(2026 - 01 - 15)),
        datetime!(2026-01-15 5:00 UTC),
        "0500"
    );
    assert_eq!(
        start(DayBoundary::Gas, date!(2026 - 07 - 15)),
        datetime!(2026-07-15 4:00 UTC),
        "0400"
    );
}

/// *Allgemeine Festlegungen* 6.1d, Kap. 3.1, pp. 28–30: the worked examples.
///
/// - The Bilanzierungsmonat Juni 2021 covers 01.06.2021 00:00 to 01.07.2021
///   00:00 for Strom and 06:00 to 06:00 for Gas, gesetzliche deutsche Zeit.
/// - An Anmeldung zum 01.06.2021 is sent as 31.05.2021 22:00 UTC (Strom) and
///   01.06.2021 04:00 UTC (Gas).
/// - The befristete Anmeldung ends 30.11.2021, sent as `DTM+93:202111292300`
///   (Strom) and `DTM+93:202111300500` (Gas).
#[test]
fn the_worked_examples_of_kap_3_1() {
    let june = |b: DayBoundary| b.month(2021, time::Month::June).unwrap();
    assert_eq!(
        june(DayBoundary::Strom).start(),
        datetime!(2021-05-31 22:00 UTC)
    );
    assert_eq!(
        june(DayBoundary::Strom).end(),
        datetime!(2021-06-30 22:00 UTC)
    );
    assert_eq!(
        june(DayBoundary::Gas).start(),
        datetime!(2021-06-01 4:00 UTC)
    );
    assert_eq!(june(DayBoundary::Gas).end(), datetime!(2021-07-01 4:00 UTC));

    let start = |b: DayBoundary, d| b.day(d).unwrap().start();
    assert_eq!(
        start(DayBoundary::Strom, date!(2021 - 11 - 30)),
        datetime!(2021-11-29 23:00 UTC)
    );
    assert_eq!(
        start(DayBoundary::Gas, date!(2021 - 11 - 30)),
        datetime!(2021-11-30 5:00 UTC)
    );
}

/// EDI@Energy *Codeliste der OBIS-Kennzahlen und Medien* 2.5c, § 2.1 (p. 7),
/// § 2.2 and § 2.3 (p. 8), against the predicates that read them.
#[test]
fn the_obis_value_groups_follow_the_codeliste() {
    // § 2.1: *"+ Bezug des Kunden aus dem Netz (z. B. 1-b:1.x.y)"* and
    // *"- (Rück-) Lieferung des Kunden an das Netz (z. B. 1-b:2.x.y)"*.
    // `x` and `y` are free, so the Messart never enters the direction.
    for messart in [6u8, 8, 9, 29] {
        let bezug: ObisCode = format!("1-0:1.{messart}.0").parse().unwrap();
        let lieferung: ObisCode = format!("1-0:2.{messart}.0").parse().unwrap();
        assert!(bezug.is_import() && !bezug.is_export(), "{bezug}");
        assert!(
            lieferung.is_export() && !lieferung.is_import(),
            "{lieferung}"
        );
    }

    // § 2.1 and the Messart column of § 2.2: 6 Maximum, 8 Zeitintegral 1
    // (Zählerstände), 9 Zeitintegral 2 (Vorschübe), 29 Zeitintegral 5
    // (*"Lastgang (Energiemengen für Zeitintervalle von äquidistanter
    // Dauer)"*).
    assert!(ObisCode::STROM_BEZUG_MAXIMUM.is_maximum());
    assert!(ObisCode::STROM_BEZUG_TOTAL.is_zaehlerstand());
    assert!(ObisCode::STROM_BEZUG_VORSCHUB.is_vorschub());
    assert!(ObisCode::STROM_BEZUG_LASTGANG.is_lastgang());

    // § 2.2, Messgröße C: 3/4 Blindleistung positiv/negativ, 5…8 Blindleistung
    // Q I…Q IV.
    for c in 3..=8u8 {
        let code: ObisCode = format!("1-0:{c}.8.0").parse().unwrap();
        assert!(code.is_reactive(), "1-0:{c}.8.0");
    }

    // § 2.2, Tarif E: 0 Total, 1…62 Tarif, 63 Fehlerregister — a real code
    // that must parse and must not be mistaken for tariff 63.
    let fehler: ObisCode = "1-0:1.8.63".parse().unwrap();
    assert!(fehler.is_fehlerregister());
    assert_eq!(fehler.tariff(), None);
    assert_eq!("1-0:1.8.62".parse::<ObisCode>().unwrap().tariff(), Some(62));

    // § 2.3: *"Wertegruppe F wird für die Kommunikation im deutschen Gasmarkt
    // nicht verwendet."* So the canonical string carries five groups.
    assert_eq!(ObisCode::GAS_VOLUME_M3.to_string(), "7-0:3.0.0");
    assert_eq!(ObisCode::STROM_BEZUG_LASTGANG.to_string(), "1-0:1.29.0");
}

/// VDN *MeteringCode* 2006, A7.1.2.1: a day holds 96 registration periods,
/// the spring change day 92 and the autumn change day 100. Each, complete,
/// validates clean — and a flat 96 on the autumn day is four missing.
#[test]
fn a_day_holds_92_96_or_100_quarter_hours() {
    let now = datetime!(2027-01-01 0:00 UTC);
    for (date, count) in [
        (date!(2026 - 03 - 29), 92),
        (date!(2024 - 02 - 29), 96),
        (date!(2026 - 10 - 25), 100),
    ] {
        let day = DayBoundary::Strom.day(date).unwrap();
        let slots = |n: i64| {
            let ivs = (0..n)
                .map(|i| {
                    let v = Decimal::new(1000 + (i * 37) % 200, 3);
                    MeterInterval::quarter_hour(
                        day.start() + time::Duration::minutes(15 * i),
                        v,
                        QualityFlag::Measured,
                    )
                    .unwrap()
                })
                .collect();
            Series::new(Resolution::QUARTER_HOUR, DayBoundary::Strom, ivs).unwrap()
        };
        let report = validate(&slots(count), &Rules::strom(day, now, None));
        assert_eq!(report.grade(), Grade::A, "{date}: {:?}", report.findings);
        if count == 100 {
            let short = validate(&slots(96), &Rules::strom(day, now, None));
            let gap = short.by_rule(Rule::Gap).next().unwrap();
            assert_eq!(gap.reference, Some(dec!(4)));
        }
    }
}
