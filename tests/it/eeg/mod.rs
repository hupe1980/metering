//! MiSpeL — the published examples of the adopted Anlagen, and crate-derived
//! vectors for every case the Festlegung publishes no example for.
//!
//! Published vectors name the Anlage and page. Crate-derived vectors are
//! labelled as such: hand-checkable, built for this crate, and verified with
//! exact rational arithmetic.

mod properties;

use std::collections::BTreeSet;

use metering::eeg::EegError;
use metering::eeg::mispel::abgrenzung::{
    Abgrenzungsoption, Anlagen, Basisfall, Einzelzaehler, Einzelzaehlerfall, GewichteteAnlage,
    Monat, VorrangAnlage, jahr, jahr_einzelzaehler,
};
use metering::eeg::mispel::pauschal::{
    Pauschalbasis, Pauschalgrenzen, Pauschaloption, SolarAnlage, Zeitraum,
};
use metering::eeg::mispel::{Abgrenzungsfall, Einspeisung, Gate, Pauschalfall};
use metering::prelude::*;
use rust_decimal::RoundingStrategy;
use time::macros::{date, datetime};
use time::{Duration, OffsetDateTime};

const T0: OffsetDateTime = datetime!(2026-06-01 0:00 UTC);

fn at(i: usize) -> OffsetDateTime {
    T0 + Duration::minutes(15 * i as i64)
}

fn series_at(start: OffsetDateTime, values: &[i64]) -> Series {
    let ivs = values
        .iter()
        .enumerate()
        .map(|(i, &v)| {
            MeterInterval::quarter_hour(
                start + Duration::minutes(15 * i as i64),
                Decimal::from(v),
                QualityFlag::Measured,
            )
            .unwrap()
        })
        .collect();
    Series::new(Resolution::QUARTER_HOUR, DayBoundary::Strom, ivs).unwrap()
}

fn s(values: &[i64]) -> Series {
    series_at(T0, values)
}

fn flags(bits: &[u8]) -> BTreeSet<OffsetDateTime> {
    bits.iter()
        .enumerate()
        .filter(|(_, b)| **b == 1)
        .map(|(i, _)| at(i))
        .collect()
}

fn d(x: &str) -> Decimal {
    Decimal::from_str_exact(x).unwrap()
}

/// Z1NB, Z1NE, Z2V, Z2E columns of a crate-derived Anlage 1 vector.
struct Cols {
    z1nb: Series,
    z1ne: Series,
    z2v: Series,
    z2e: Series,
}

fn cols(z1nb: &[i64], z1ne: &[i64], z2v: &[i64], z2e: &[i64]) -> Cols {
    Cols {
        z1nb: s(z1nb),
        z1ne: s(z1ne),
        z2v: s(z2v),
        z2e: s(z2e),
    }
}

fn run<'a>(c: &'a Cols, basisfall: Basisfall<'a>, anlagen: Anlagen<'a>) -> Monat {
    Abgrenzungsoption {
        z1nb: &c.z1nb,
        einspeisung: Einspeisung::Z1(&c.z1ne),
        z2v: &c.z2v,
        z2e: &c.z2e,
        basisfall,
        anlagen,
    }
    .monat()
    .unwrap()
}

fn a1_v1() -> Cols {
    cols(&[10, 0, 0, 1], &[0, 3, 7, 2], &[6, 4, 0, 0], &[0, 0, 8, 0])
}

// ── Published: Anlage 1 ─────────────────────────────────────────────────────

/// Anl. 1 p. 15, Beispielrechnung 1: Z2V¼ = 100, Z1NB¼ = 130 — the storage
/// consumption is drawn entirely from the grid, (1)¼ = 100.
#[test]
fn published_anlage1_p15_speichervorrang_withdrawal() {
    let c = cols(&[130], &[0], &[100], &[0]);
    let m = run(&c, Basisfall::Stromspeicher, Anlagen::Eine(Gate::Always));
    assert_eq!(m.viertelstunden[0].f1, dec!(100));
    assert_eq!(m.f3 - m.f9, dec!(30)); // "Die übrigen 30 kWh Netzbezug"
}

/// Anl. 1 p. 16, Beispielrechnung 2: Z2E¼ = 100, Z1NE¼ = 80 — the feed-in
/// comes entirely from the storage, (2)¼ = 80, and (23)¼ = 0.
#[test]
fn published_anlage1_p16_speichervorrang_feed_in() {
    let c = cols(&[0], &[80], &[0], &[100]);
    let m = run(&c, Basisfall::Stromspeicher, Anlagen::Eine(Gate::Always));
    assert_eq!(m.viertelstunden[0].f2, dec!(80));
    assert_eq!(m.viertelstunden[0].f23, dec!(0));
    assert_eq!(m.f6 - m.f11, dec!(20));
}

// ── Crate-derived: Anlage 1, Basisfälle ─────────────────────────────────────

/// A1-V1 (crate-derived): a mixed PV and arbitrage month.
#[test]
fn crate_derived_a1_v1() {
    let c = a1_v1();
    let aw = flags(&[1, 1, 1, 0]);
    let m = run(
        &c,
        Basisfall::Stromspeicher,
        Anlagen::Eine(Gate::Flagged(&aw)),
    );
    assert_eq!(m.faelle, [Abgrenzungsfall::A1]);
    let got = [
        m.f3, m.f4, m.f5, m.f6, m.f9, m.f10, m.f11, m.f12, m.f13, m.f15, m.f16, m.f19, m.f20, m.f21,
    ];
    let want = [
        "11", "12", "10", "8", "6", "4", "7", "0", "7", "3.2", "3.8", "0.95", "4.75", "6.25",
    ]
    .map(d);
    assert_eq!(got, want);
    assert_eq!(m.f14, Some(d("0.8")));
    assert_eq!(m.f17, Some(d("2")));
    assert_eq!(m.f18, Some(d("0.475")));
    let p = m.anlagen[0];
    assert_eq!(
        [p.f26, p.f28, p.f29, p.f31, p.f32],
        ["3", "3.2", "7", "3.2", "6.2"].map(d)
    );
    assert_eq!(p.f30, Some(d("1")));
    assert_eq!(m.foerderfaehig().unwrap(), d("6.2"));
}

/// The Speichervorrang applies per quarter-hour, before the sum. A1-V1 has
/// Σ min(Z1NB¼, Z2V¼) = 6 but min(Σ Z1NB, Σ Z2V) = 10; summing first would
/// give (10) = 0.
#[test]
fn minimum_before_sum_changes_the_result() {
    let c = a1_v1();
    let m = run(&c, Basisfall::Stromspeicher, Anlagen::Eine(Gate::Always));
    assert_eq!(m.f9, dec!(6));
    assert_ne!(m.f9, m.f3.min(m.f5));
    assert_eq!(m.f10, dec!(4));
}

/// A1-V2 (crate-derived): discharge exceeds charge; (14) > 1 and (12) > 0,
/// transcribed unclamped.
#[test]
fn crate_derived_a1_v2_unclamped_efficiency() {
    let c = cols(&[0, 2, 0], &[5, 0, 4], &[0, 2, 0], &[5, 0, 0]);
    let aw = flags(&[1, 1, 0]);
    let m = run(
        &c,
        Basisfall::Stromspeicher,
        Anlagen::Eine(Gate::Flagged(&aw)),
    );
    assert_eq!(m.f12, dec!(3));
    assert_eq!(m.f14, Some(d("2.5")));
    assert_eq!(
        (m.f15, m.f16, m.f17, m.f19),
        (dec!(0), dec!(2), Some(dec!(0)), dec!(0))
    );
    assert_eq!((m.f20, m.f21), (dec!(2), dec!(0)));
    assert_eq!(m.anlagen[0].f32, dec!(0));

    // Annual order: (22) sums monthly (21) — a second month with the
    // opposite fill effect does not cancel this month's (12).
    let c2 = Cols {
        z1nb: series_at(datetime!(2026-07-01 0:00 UTC), &[4]),
        z1ne: series_at(datetime!(2026-07-01 0:00 UTC), &[0]),
        z2v: series_at(datetime!(2026-07-01 0:00 UTC), &[4]),
        z2e: series_at(datetime!(2026-07-01 0:00 UTC), &[0]),
    };
    let m2 = run(&c2, Basisfall::Stromspeicher, Anlagen::Eine(Gate::Always));
    assert_eq!(m2.f21, dec!(4));
    let y = jahr(&[m.clone(), m2]).unwrap();
    assert_eq!(y.f22, dec!(4));
    assert_eq!(y.f33, [dec!(0)]);
}

/// A1-V3 (crate-derived): the storage only charges, (6) = 0. (18) is 0/0,
/// and (19) is 0.
#[test]
fn crate_derived_a1_v3_zero_generation() {
    let c = cols(&[5, 0], &[0, 2], &[5, 3], &[0, 0]);
    let m = run(&c, Basisfall::Stromspeicher, Anlagen::Eine(Gate::Always));
    assert_eq!((m.f6, m.f16, m.f17), (dec!(0), dec!(0), Some(dec!(8))));
    assert_eq!(m.f18, None);
    assert_eq!((m.f19, m.f20, m.f21), (dec!(0), dec!(0), dec!(5)));
    let p = m.anlagen[0];
    assert_eq!(
        (p.f26, p.f28, p.f29, p.f30, p.f31),
        (dec!(2), dec!(0), dec!(0), None, dec!(0))
    );
    assert_eq!(p.f32, dec!(2));
}

fn a2_v1() -> Cols {
    cols(&[10, 0, 0], &[0, 4, 6], &[10, 4, 0], &[0, 0, 6])
}

fn a2_v2() -> Cols {
    cols(&[2, 0], &[0, 10], &[2, 0], &[0, 10])
}

fn a2_v3() -> Cols {
    cols(&[0, 0, 0], &[5, 4, 4], &[5, 0, 0], &[0, 4, 4])
}

/// A2-V1 (crate-derived): charge point baseline.
#[test]
fn crate_derived_a2_v1() {
    let m = run(&a2_v1(), Basisfall::Ladepunkt, Anlagen::Eine(Gate::Always));
    assert_eq!(m.faelle, [Abgrenzungsfall::A2]);
    assert_eq!(m.f14, Some(d("0.85")));
    assert_eq!(
        [
            m.f10, m.f11, m.f12, m.f13, m.f15, m.f16, m.f19, m.f20, m.f21
        ],
        ["4", "6", "0", "6", "3.4", "2.6", "0", "2.6", "7.4"].map(d)
    );
    assert_eq!(m.f17, None);
    let p = m.anlagen[0];
    assert_eq!(
        [p.f26, p.f28, p.f31, p.f32],
        ["4", "3.4", "3.4", "7.4"].map(d)
    );
}

/// A2-V2 (crate-derived): Fremdtankstrom (12) = 8.
#[test]
fn crate_derived_a2_v2_fremdtankstrom() {
    let m = run(&a2_v2(), Basisfall::Ladepunkt, Anlagen::Eine(Gate::Always));
    assert_eq!(
        (m.f12, m.f13, m.f15, m.f16),
        (dec!(8), dec!(2), dec!(0), dec!(2))
    );
    assert_eq!((m.f20, m.f21), (dec!(2), dec!(0)));
    assert_eq!(m.anlagen[0].f32, dec!(0));
}

/// A2-V3 (crate-derived): (20) capped by (3) = 0, and an AW=0 share.
#[test]
fn crate_derived_a2_v3() {
    let aw = flags(&[1, 0, 1]);
    let m = run(
        &a2_v3(),
        Basisfall::Ladepunkt,
        Anlagen::Eine(Gate::Flagged(&aw)),
    );
    assert_eq!(
        (m.f15, m.f16, m.f20, m.f21),
        (d("4.25"), d("0.75"), dec!(0), dec!(0))
    );
    let p = m.anlagen[0];
    assert_eq!(p.f30, Some(d("0.5")));
    assert_eq!(
        [p.f26, p.f28, p.f29, p.f31, p.f32],
        ["5", "4.25", "4", "2.125", "7.125"].map(d)
    );
}

/// A3-V1..V3 (crate-derived): the A3 Formelsatz is A2's — equal values,
/// different code.
#[test]
fn crate_derived_a3_equals_a2_with_its_own_code() {
    let m = run(
        &a1_v1(),
        Basisfall::StromspeicherUndLadepunkt,
        Anlagen::Eine(Gate::Flagged(&flags(&[1, 1, 1, 0]))),
    );
    assert_eq!(m.faelle, [Abgrenzungsfall::A3]);
    assert_eq!(
        (m.f15, m.f16, m.f19, m.f20, m.f21),
        (d("3.4"), d("3.6"), dec!(0), d("3.6"), d("7.4"))
    );
    assert_eq!((m.anlagen[0].f28, m.anlagen[0].f32), (d("3.4"), d("6.4")));

    for c in [a2_v2(), a2_v3()] {
        let aw = flags(&[1, 0, 1]);
        let a2 = run(&c, Basisfall::Ladepunkt, Anlagen::Eine(Gate::Flagged(&aw)));
        let a3 = run(
            &c,
            Basisfall::StromspeicherUndLadepunkt,
            Anlagen::Eine(Gate::Flagged(&aw)),
        );
        assert_eq!(a2.f20, a3.f20);
        assert_eq!(a2.f21, a3.f21);
        assert_eq!(a2.anlagen, a3.anlagen);
        assert_ne!(a2.faelle, a3.faelle);
    }
}

/// A4-V1..V3 (crate-derived): losses from the storage meter Z3.
#[test]
fn crate_derived_a4() {
    let c = a1_v1();
    let (z3v, z3e) = (s(&[6, 2, 0, 0]), s(&[0, 0, 6, 0]));
    let aw = flags(&[1, 1, 1, 0]);
    let m = run(
        &c,
        Basisfall::MitSpeicherzaehler {
            z3v: &z3v,
            z3e: &z3e,
        },
        Anlagen::Eine(Gate::Flagged(&aw)),
    );
    assert_eq!(m.faelle, [Abgrenzungsfall::A4]);
    assert_eq!(
        (m.f7, m.f8, m.f17),
        (Some(dec!(8)), Some(dec!(6)), Some(dec!(2)))
    );
    assert_eq!(
        (m.f18, m.f19, m.f20, m.f21),
        (Some(d("0.45")), d("0.9"), d("4.5"), d("6.5"))
    );
    assert_eq!(m.anlagen[0].f32, d("6.4"));

    let c = a2_v1();
    let (z3v, z3e) = (s(&[4, 0, 0]), s(&[0, 0, 5]));
    let m = run(
        &c,
        Basisfall::MitSpeicherzaehler {
            z3v: &z3v,
            z3e: &z3e,
        },
        Anlagen::Eine(Gate::Always),
    );
    assert_eq!(
        (m.f17, m.f19, m.f20, m.f21),
        (Some(dec!(0)), dec!(0), d("2.6"), d("7.4"))
    );
    assert_eq!(m.anlagen[0].f32, d("7.4"));

    let c = a2_v3();
    let (z3v, z3e) = (s(&[5, 0, 0]), s(&[0, 4, 0]));
    let aw = flags(&[1, 0, 1]);
    let m = run(
        &c,
        Basisfall::MitSpeicherzaehler {
            z3v: &z3v,
            z3e: &z3e,
        },
        Anlagen::Eine(Gate::Flagged(&aw)),
    );
    assert_eq!(
        (m.f17, m.f18, m.f19),
        (Some(dec!(1)), Some(d("0.09375")), d("0.09375"))
    );
    assert_eq!((m.f20, m.f21), (dec!(0), dec!(0)));
    assert_eq!(m.anlagen[0].f32, d("7.125"));
}

// ── Crate-derived: Anlage 1, Sonderfälle ────────────────────────────────────

/// A5 (crate-derived): two plants of the same kind, weights 6 and 4.
#[test]
fn crate_derived_a5() {
    let c = a1_v1();
    let (aw_a, aw_b) = (flags(&[1, 1, 1, 0]), flags(&[1, 0, 1, 1]));
    let m = run(
        &c,
        Basisfall::Stromspeicher,
        Anlagen::Gleichartig(vec![
            GewichteteAnlage {
                leistung: dec!(6),
                aw: Gate::Flagged(&aw_a),
            },
            GewichteteAnlage {
                leistung: dec!(4),
                aw: Gate::Flagged(&aw_b),
            },
        ]),
    );
    assert_eq!(m.faelle, [Abgrenzungsfall::A1, Abgrenzungsfall::A5]);
    let (a, b) = (m.anlagen[0], m.anlagen[1]);
    assert_eq!(
        [a.f26, a.f28, a.f29, a.f31, a.f32],
        ["1.8", "1.92", "7", "1.92", "3.72"].map(d)
    );
    assert_eq!(
        [b.f26, b.f28, b.f29, b.f31, b.f32],
        ["0.8", "1.28", "7", "1.28", "2.08"].map(d)
    );
    assert_eq!(m.f20, d("4.75")); // the levy side is A1's
}

/// The A5-Variante is an identity of A5 with identical gates (Anl. 1
/// p. 50): (32k) = ZFk • (32).
#[test]
fn a5_variante_identity() {
    let c = a1_v1();
    let aw = flags(&[1, 1, 1, 0]);
    let single = run(
        &c,
        Basisfall::Stromspeicher,
        Anlagen::Eine(Gate::Flagged(&aw)),
    );
    let split = run(
        &c,
        Basisfall::Stromspeicher,
        Anlagen::Gleichartig(vec![
            GewichteteAnlage {
                leistung: dec!(6),
                aw: Gate::Flagged(&aw),
            },
            GewichteteAnlage {
                leistung: dec!(4),
                aw: Gate::Flagged(&aw),
            },
        ]),
    );
    assert_eq!(split.anlagen[0].f32, d("0.6") * single.anlagen[0].f32);
    assert_eq!(split.anlagen[1].f32, d("2.48"));
    assert_eq!(
        split.foerderfaehig().unwrap(),
        single.foerderfaehig().unwrap()
    );
}

/// A6 (crate-derived): plant a metered with priority, plant b residual.
#[test]
fn crate_derived_a6() {
    let c = cols(&[4, 0, 0], &[0, 5, 6], &[7, 3, 0], &[0, 0, 6]);
    let z4e = s(&[0, 6, 0]);
    let (aw_a, aw_b) = (flags(&[1, 1, 1]), flags(&[1, 0, 1]));
    let m = run(
        &c,
        Basisfall::Stromspeicher,
        Anlagen::Vorrang {
            vorrang: vec![VorrangAnlage {
                z4e: &z4e,
                aw: Gate::Flagged(&aw_a),
            }],
            rest: Gate::Flagged(&aw_b),
        },
    );
    assert_eq!(m.faelle, [Abgrenzungsfall::A1, Abgrenzungsfall::A6]);
    let f34 = |k: usize| {
        m.viertelstunden
            .iter()
            .map(|q| q.anlagen[k].f34.unwrap())
            .collect::<Vec<_>>()
    };
    let f23 = |k: usize| {
        m.viertelstunden
            .iter()
            .map(|q| q.anlagen[k].f23)
            .collect::<Vec<_>>()
    };
    assert_eq!(f34(0), [dec!(0), dec!(3), dec!(0)]);
    assert_eq!(f34(1), [dec!(3), dec!(0), dec!(0)]);
    assert_eq!(f23(0), [dec!(0), dec!(3), dec!(0)]);
    assert_eq!(f23(1), [dec!(0), dec!(2), dec!(0)]);
    assert_eq!(
        [
            m.f3, m.f5, m.f6, m.f9, m.f10, m.f11, m.f13, m.f15, m.f16, m.f19, m.f20, m.f21, m.f28
        ],
        [
            "4", "10", "6", "4", "6", "6", "6", "3.6", "2.4", "1.6", "4", "0", "3.6"
        ]
        .map(d)
    );
    let (a, b) = (m.anlagen[0], m.anlagen[1]);
    assert_eq!((a.f35, a.f36), (Some(dec!(3)), Some(d("0.5"))));
    assert_eq!(
        [a.f26, a.f28, a.f29, a.f31, a.f32],
        ["3", "1.8", "6", "1.8", "4.8"].map(d)
    );
    assert_eq!(
        [b.f26, b.f28, b.f29, b.f31, b.f32],
        ["0", "1.8", "6", "1.8", "1.8"].map(d)
    );
}

/// A7 (crate-derived): A1-V1 with its feed-in metered at ZW gives A1-V1's
/// values under its own code.
#[test]
fn crate_derived_a7() {
    let c = a1_v1();
    let aw = flags(&[1, 1, 1, 0]);
    let m = Abgrenzungsoption {
        z1nb: &c.z1nb,
        einspeisung: Einspeisung::Zw(&c.z1ne),
        z2v: &c.z2v,
        z2e: &c.z2e,
        basisfall: Basisfall::Stromspeicher,
        anlagen: Anlagen::Eine(Gate::Flagged(&aw)),
    }
    .monat()
    .unwrap();
    let base = run(
        &c,
        Basisfall::Stromspeicher,
        Anlagen::Eine(Gate::Flagged(&aw)),
    );
    assert_eq!(m.faelle, [Abgrenzungsfall::A1, Abgrenzungsfall::A7]);
    assert_eq!(
        (m.f20, m.f21, m.anlagen.clone()),
        (base.f20, base.f21, base.anlagen)
    );
}

/// A8, A10, A11 (crate-derived): the single-meter Sonderfälle.
#[test]
fn crate_derived_single_meter() {
    let (z1nb, z1ne) = (s(&[5, 0, 0]), s(&[0, 10, 10]));
    let aw = flags(&[1, 1, 0]);
    let m = Einzelzaehler {
        z1nb: &z1nb,
        z1ne: &z1ne,
        fall: Einzelzaehlerfall::Kolokation {
            aw: Gate::Flagged(&aw),
        },
    }
    .monat()
    .unwrap();
    assert_eq!(m.fall, Abgrenzungsfall::A8);
    assert_eq!(
        (m.f3, m.f4, m.f20, m.f21),
        (dec!(5), dec!(20), dec!(5), dec!(0))
    );
    assert_eq!(
        (m.f38, m.f39, m.f40, m.f32),
        (
            Some(dec!(10)),
            Some(d("0.5")),
            Some(dec!(15)),
            Some(d("7.5"))
        )
    );
    assert_eq!(jahr_einzelzaehler(&[m]).unwrap().f33, [d("7.5")]);

    let (z1nb, z1ne) = (s(&[8]), s(&[6]));
    let m = Einzelzaehler {
        z1nb: &z1nb,
        z1ne: &z1ne,
        fall: Einzelzaehlerfall::Netzspeicher,
    }
    .monat()
    .unwrap();
    assert_eq!(
        (m.fall, m.f20, m.f21),
        (Abgrenzungsfall::A10, dec!(8), dec!(0))
    );

    let (z1nb, z1ne) = (s(&[10]), s(&[4]));
    let m = Einzelzaehler {
        z1nb: &z1nb,
        z1ne: &z1ne,
        fall: Einzelzaehlerfall::OhneErzeugung,
    }
    .monat()
    .unwrap();
    assert_eq!(
        (m.fall, m.f16, m.f20, m.f21),
        (Abgrenzungsfall::A11, Some(dec!(4)), dec!(4), dec!(6))
    );
    let (z1nb, z1ne) = (s(&[3]), s(&[5]));
    let m = Einzelzaehler {
        z1nb: &z1nb,
        z1ne: &z1ne,
        fall: Einzelzaehlerfall::OhneErzeugung,
    }
    .monat()
    .unwrap();
    assert_eq!((m.f16, m.f20, m.f21), (Some(dec!(5)), dec!(3), dec!(0)));
}

/// The A9-Variante BesAR>1 table (Anl. 1 p. 86), quantity rows in MWh. A9
/// adds no quantity formula: its (20)–(22) are the base case's. Published
/// inputs (3) = 180 and (20) = 30 per month; (21) = 150 and (22) = 1 800
/// are crate-derived from them. The month is built as A11 with a feed-in of
/// 30, so that (20) = MIN [ 30 ; 180 ].
#[test]
fn published_a9_inputs_give_the_crate_derived_annual_sum() {
    let months: Vec<_> = (1..=12u8)
        .map(|mo| {
            let month = time::Month::try_from(mo).unwrap();
            let start = DayBoundary::Strom.month(2026, month).unwrap().start();
            let (nb, ne) = (series_at(start, &[180]), series_at(start, &[30]));
            let m = Einzelzaehler {
                z1nb: &nb,
                z1ne: &ne,
                fall: Einzelzaehlerfall::OhneErzeugung,
            }
            .monat()
            .unwrap();
            assert_eq!((m.f3, m.f20, m.f21), (dec!(180), dec!(30), dec!(150)));
            m
        })
        .collect();
    assert_eq!(jahr_einzelzaehler(&months).unwrap().f22, dec!(1800));
}

// ── Refusals ────────────────────────────────────────────────────────────────

#[test]
fn refusals_name_the_series_and_instant() {
    let c = a1_v1();
    let short = s(&[1, 1, 1]);
    let err = Abgrenzungsoption {
        z1nb: &c.z1nb,
        einspeisung: Einspeisung::Z1(&c.z1ne),
        z2v: &short,
        z2e: &c.z2e,
        basisfall: Basisfall::Stromspeicher,
        anlagen: Anlagen::Eine(Gate::Always),
    }
    .monat()
    .unwrap_err();
    assert_eq!(
        err,
        EegError::Misaligned {
            series: "Z2V",
            at: at(3)
        }
    );

    // Zero weight in A5 is refused.
    let err = Abgrenzungsoption {
        z1nb: &c.z1nb,
        einspeisung: Einspeisung::Z1(&c.z1ne),
        z2v: &c.z2v,
        z2e: &c.z2e,
        basisfall: Basisfall::Stromspeicher,
        anlagen: Anlagen::Gleichartig(vec![GewichteteAnlage {
            leistung: dec!(0),
            aw: Gate::Always,
        }]),
    }
    .monat()
    .unwrap_err();
    assert!(matches!(err, EegError::ZeroWeight { .. }));

    // A month boundary inside the inputs is refused.
    let late = datetime!(2026-06-30 21:45 UTC); // 23:45 local; the next slot is July
    let two = series_at(late, &[1, 1]);
    let err = Einzelzaehler {
        z1nb: &two,
        z1ne: &two,
        fall: Einzelzaehlerfall::Netzspeicher,
    }
    .monat()
    .unwrap_err();
    assert_eq!(
        err,
        EegError::OutsidePeriod {
            at: late + Duration::minutes(15)
        }
    );
}

/// A month containing the spring transition has the calendar's own
/// quarter-hours: 2 976 − 4 in March.
#[test]
fn a_dst_month_is_the_calendars() {
    let march = DayBoundary::Strom.month(2026, time::Month::March).unwrap();
    let n = march.count(Resolution::QUARTER_HOUR).unwrap();
    assert_eq!(n, 2972);
    let ones = vec![1; n as usize];
    let z = series_at(march.start(), &ones);
    let m = Einzelzaehler {
        z1nb: &z,
        z1ne: &z,
        fall: Einzelzaehlerfall::Netzspeicher,
    }
    .monat()
    .unwrap();
    assert_eq!(m.f3, dec!(2972));
}

// ── Published: Anlage 2 ─────────────────────────────────────────────────────

fn half_up(x: Decimal) -> Decimal {
    x.round_dp_with_strategy(0, RoundingStrategy::MidpointAwayFromZero)
}

/// Anl. 2 p. 12, Tabellen 1 und 2 — every cell, after the documented
/// display rounding (half away from zero; the two exact halves 512,5 and
/// 10 312,5 rule out half to even).
#[test]
fn published_anlage2_p12_tabellen_1_und_2() {
    let pinst = [1, 4, 6, 8, 10, 15, 30];
    let skinst = [45, 30, 15, 10, 8, 6, 4];
    let tabelle1: [[i64; 7]; 7] = [
        [501, 502, 503, 505, 506, 508, 513],
        [2018, 2027, 2053, 2080, 2100, 2133, 2200],
        [3040, 3060, 3120, 3180, 3225, 3300, 3450],
        [4071, 4107, 4213, 4320, 4400, 4533, 4800],
        [5111, 5167, 5333, 5500, 5625, 5833, 6250],
        [7750, 7875, 8250, 8625, 8906, 9375, 10313],
        [16000, 16500, 18000, 19500, 20625, 22500, 26250],
    ];
    let tabelle2: [[i64; 7]; 7] = [
        [501, 502, 503, 505, 506, 508, 513],
        [504, 507, 513, 520, 525, 533, 550],
        [507, 510, 520, 530, 538, 550, 575],
        [509, 513, 527, 540, 550, 567, 600],
        [511, 517, 533, 550, 563, 583, 625],
        [517, 525, 550, 575, 594, 625, 688],
        [533, 550, 600, 650, 688, 750, 875],
    ];
    for (r, &p) in pinst.iter().enumerate() {
        for (c, &sk) in skinst.iter().enumerate() {
            let g = Pauschalgrenzen::new(
                Pauschalbasis::Stromspeicher {
                    kapazitaet_kwh: Decimal::from(sk),
                },
                Decimal::from(p),
            )
            .unwrap();
            assert_eq!(
                half_up(g.p4),
                Decimal::from(tabelle1[r][c]),
                "Tabelle 1, {p} kWp / {sk} kWh"
            );
            assert_eq!(
                half_up(g.p4 / Decimal::from(p)),
                Decimal::from(tabelle2[r][c]),
                "Tabelle 2, {p} kWp / {sk} kWh"
            );
        }
    }
    // The exact halves.
    let g = Pauschalgrenzen::new(
        Pauschalbasis::Stromspeicher {
            kapazitaet_kwh: dec!(4),
        },
        dec!(1),
    )
    .unwrap();
    assert_eq!(g.p4, d("512.5"));
    assert_ne!(
        g.p4.round_dp_with_strategy(0, RoundingStrategy::MidpointNearestEven),
        dec!(513)
    );
}

fn year_rows(start: OffsetDateTime, z1ne: &[i64], z1nb: &[i64]) -> (Series, Series) {
    (series_at(start, z1nb), series_at(start, z1ne))
}

/// Anl. 2 p. 13, Beispielrechnung 1: 8 kWp, 10 kWh, feed-in 3 000 kWh —
/// all förderfähig, no room for Saldierung.
#[test]
fn published_anlage2_p13_beispielrechnung_1() {
    let (nb, ne) = year_rows(T0, &[3000], &[0]);
    let y = Pauschaloption {
        z1nb: &nb,
        einspeisung: Einspeisung::Z1(&ne),
        sp: Gate::Always,
        basis: Pauschalbasis::Stromspeicher {
            kapazitaet_kwh: dec!(10),
        },
        anlagen: vec![SolarAnlage {
            kwp: dec!(8),
            aw: Gate::Always,
        }],
        zeitraum: Zeitraum::Kalenderjahr(2026),
    }
    .jahr()
    .unwrap();
    assert_eq!(
        (y.grenzen.p1, y.grenzen.p3, y.grenzen.p4),
        (dec!(4000), dec!(320), dec!(4320))
    );
    assert_eq!(y.anlagen[0].p15, dec!(3000));
    assert_eq!((y.p8, y.p10), (dec!(0), dec!(0)));
}

/// Anl. 2 p. 14, Beispielrechnung 2: feed-in 6 000, withdrawal 1 500 — the
/// Umlage falls to zero for the whole withdrawal.
#[test]
fn published_anlage2_p14_beispielrechnung_2() {
    let (nb, ne) = year_rows(T0, &[6000], &[1500]);
    let y = Pauschaloption {
        z1nb: &nb,
        einspeisung: Einspeisung::Z1(&ne),
        sp: Gate::Always,
        basis: Pauschalbasis::Stromspeicher {
            kapazitaet_kwh: dec!(10),
        },
        anlagen: vec![SolarAnlage {
            kwp: dec!(8),
            aw: Gate::Always,
        }],
        zeitraum: Zeitraum::Kalenderjahr(2026),
    }
    .jahr()
    .unwrap();
    assert_eq!(y.faelle, [Pauschalfall::P1]);
    assert_eq!(y.anlagen[0].p15, dec!(4000));
    assert_eq!(
        (y.p8, y.p9, y.p10, y.p11),
        (dec!(1680), dec!(1500), dec!(1500), dec!(0))
    );
}

/// Anl. 2 pp. 55–56, Rumpfjahre: 8 kWp, a second 5 kWh storage from
/// 16 May. Exact values, and the published figures after display rounding
/// of each — including the sum, which is not the sum of the rounded parts.
#[test]
fn published_anlage2_p56_rumpfjahre() {
    let first = Pauschalgrenzen::new(
        Pauschalbasis::Stromspeicher {
            kapazitaet_kwh: dec!(10),
        },
        dec!(8),
    )
    .unwrap()
    .rumpfjahr(date!(2026 - 01 - 01), date!(2026 - 05 - 16))
    .unwrap();
    assert_eq!((first.tr, first.trs, first.tk), (136, 46, 365));
    assert_eq!(first.p1, Decimal::from(46 * 4000) / Decimal::from(183));
    assert_eq!(
        [half_up(first.p1), half_up(first.p3), half_up(first.p4)],
        [dec!(1005), dec!(119), dec!(1125)]
    );
    let second = Pauschalgrenzen::new(
        Pauschalbasis::Stromspeicher {
            kapazitaet_kwh: dec!(15),
        },
        dec!(8),
    )
    .unwrap()
    .rumpfjahr(date!(2026 - 05 - 17), date!(2026 - 12 - 31))
    .unwrap();
    assert_eq!((second.tr, second.trs), (229, 137));
    assert_eq!(
        [half_up(second.p1), half_up(second.p3), half_up(second.p4)],
        [dec!(2995), dec!(134), dec!(3128)]
    );
}

// ── Crate-derived: Anlage 2 ─────────────────────────────────────────────────

fn p_run(
    nb: &[i64],
    ne: &[i64],
    sp: &[u8],
    basis: Pauschalbasis,
    plants: Vec<(Decimal, Vec<u8>)>,
) -> metering::eeg::mispel::pauschal::Pauschaljahr {
    let (z1nb, z1ne) = (s(nb), s(ne));
    let sp = flags(sp);
    let aw: Vec<BTreeSet<OffsetDateTime>> = plants.iter().map(|(_, b)| flags(b)).collect();
    Pauschaloption {
        z1nb: &z1nb,
        einspeisung: Einspeisung::Z1(&z1ne),
        sp: Gate::Flagged(&sp),
        basis,
        anlagen: plants
            .iter()
            .zip(&aw)
            .map(|((kwp, _), set)| SolarAnlage {
                kwp: *kwp,
                aw: Gate::Flagged(set),
            })
            .collect(),
        zeitraum: Zeitraum::Kalenderjahr(2026),
    }
    .jahr()
    .unwrap()
}

/// P1-V1 (crate-derived): SP and AW gates disagree; SP = 0 passes (P5)¼.
#[test]
fn crate_derived_p1_v1() {
    let y = p_run(
        &[0, 1500, 0],
        &[5000, 2000, 1000],
        &[1, 0, 1],
        Pauschalbasis::Stromspeicher {
            kapazitaet_kwh: dec!(10),
        },
        vec![(dec!(8), vec![1, 0, 0])],
    );
    assert_eq!(
        (y.p7, y.p8, y.p9, y.p10, y.p11),
        (dec!(6000), dec!(1680), dec!(1500), dec!(1500), dec!(0))
    );
    assert_eq!(
        (y.anlagen[0].p14, y.anlagen[0].p15),
        (dec!(5000), dec!(4000))
    );
}

/// P1-V2 (crate-derived): feed-in inside the Indifferenzbereich.
#[test]
fn crate_derived_p1_v2() {
    let y = p_run(
        &[3000],
        &[4100],
        &[1],
        Pauschalbasis::Stromspeicher {
            kapazitaet_kwh: dec!(10),
        },
        vec![(dec!(8), vec![1])],
    );
    assert_eq!((y.p8, y.p10, y.p11), (dec!(0), dec!(0), dec!(3000)));
    assert_eq!(y.anlagen[0].p15, dec!(4000));
}

/// P2-V1 and P3 (crate-derived): (P2)P2 = 0,2; (P2)P3 picks the minimum.
#[test]
fn crate_derived_p2_and_p3() {
    let y = p_run(
        &[1000],
        &[6000],
        &[1],
        Pauschalbasis::Ladepunkt,
        vec![(dec!(8), vec![1])],
    );
    assert_eq!(y.faelle, [Pauschalfall::P2]);
    assert_eq!((y.grenzen.p3, y.grenzen.p4), (dec!(800), dec!(4800)));
    assert_eq!((y.p8, y.p10, y.p11), (dec!(1200), dec!(1000), dec!(0)));

    let p3 = |sk| {
        Pauschalgrenzen::new(
            Pauschalbasis::StromspeicherUndLadepunkt { kapazitaet_kwh: sk },
            dec!(8),
        )
        .unwrap()
    };
    assert_eq!((p3(dec!(2)).p2, p3(dec!(2)).p4), (d("0.2"), dec!(4800)));
    assert_eq!((p3(dec!(10)).p2, p3(dec!(10)).p4), (d("0.08"), dec!(4320)));
    // (P2)P1 is not bounded by 0,2 (Tabelle 1: 30 kWp / 4 kWh).
    let p1 = Pauschalgrenzen::new(
        Pauschalbasis::Stromspeicher {
            kapazitaet_kwh: dec!(4),
        },
        dec!(30),
    )
    .unwrap();
    assert_eq!(p1.p2, d("0.75"));
    assert!(matches!(
        Pauschalgrenzen::new(
            Pauschalbasis::Stromspeicher {
                kapazitaet_kwh: dec!(0)
            },
            dec!(8)
        ),
        Err(EegError::ZeroWeight { .. })
    ));
}

/// P4-V1 (crate-derived) and the P4-Variante identity.
#[test]
fn crate_derived_p4() {
    let basis = Pauschalbasis::Stromspeicher {
        kapazitaet_kwh: dec!(10),
    };
    let y = p_run(
        &[0, 0],
        &[3000, 2000],
        &[1, 1],
        basis,
        vec![(dec!(6), vec![1, 1]), (dec!(2), vec![1, 0])],
    );
    assert_eq!(y.faelle, [Pauschalfall::P1, Pauschalfall::P4]);
    assert_eq!(
        (y.anlagen[0].p14, y.anlagen[0].p15, y.anlagen[0].p16),
        (dec!(5000), dec!(4000), dec!(3000))
    );
    assert_eq!(
        (y.anlagen[1].p14, y.anlagen[1].p15, y.anlagen[1].p16),
        (dec!(3000), dec!(3000), dec!(750))
    );

    let y = p_run(
        &[0, 0],
        &[3000, 2000],
        &[1, 1],
        basis,
        vec![(dec!(6), vec![1, 1]), (dec!(2), vec![1, 1])],
    );
    assert_eq!(
        (y.anlagen[0].p16, y.anlagen[1].p16),
        (dec!(3000), dec!(1000))
    );
    assert_eq!(y.foerderfaehig().unwrap(), dec!(4000));
}

/// P5 (crate-derived): feed-in at ZW, same values, own code.
#[test]
fn crate_derived_p5() {
    let (z1nb, zwne) = (s(&[0, 1500, 0]), s(&[5000, 2000, 1000]));
    let (sp, aw) = (flags(&[1, 0, 1]), flags(&[1, 0, 0]));
    let y = Pauschaloption {
        z1nb: &z1nb,
        einspeisung: Einspeisung::Zw(&zwne),
        sp: Gate::Flagged(&sp),
        basis: Pauschalbasis::Stromspeicher {
            kapazitaet_kwh: dec!(10),
        },
        anlagen: vec![SolarAnlage {
            kwp: dec!(8),
            aw: Gate::Flagged(&aw),
        }],
        zeitraum: Zeitraum::Kalenderjahr(2026),
    }
    .jahr()
    .unwrap();
    assert_eq!(y.faelle, [Pauschalfall::P1, Pauschalfall::P5]);
    assert_eq!(
        (y.p10, y.p11, y.anlagen[0].p15),
        (dec!(1500), dec!(0), dec!(4000))
    );
}

/// R-leap and R-late (crate-derived).
#[test]
fn crate_derived_rumpfjahre() {
    let g = Pauschalgrenzen::new(
        Pauschalbasis::Stromspeicher {
            kapazitaet_kwh: dec!(10),
        },
        dec!(8),
    )
    .unwrap();
    let leap = g
        .rumpfjahr(date!(2028 - 01 - 01), date!(2028 - 05 - 16))
        .unwrap();
    assert_eq!((leap.tr, leap.trs, leap.tk), (137, 46, 366));
    assert_eq!(leap.p3, Decimal::from(137 * 320) / Decimal::from(366));
    assert_eq!(half_up(leap.p4), dec!(1125));

    let late = g
        .rumpfjahr(date!(2026 - 11 - 01), date!(2026 - 12 - 31))
        .unwrap();
    assert_eq!((late.tr, late.trs, late.p1), (61, 0, dec!(0)));
    assert_eq!(late.p4, late.p3);

    assert_eq!(
        g.rumpfjahr(date!(2026 - 05 - 01), date!(2027 - 01 - 01)),
        Err(EegError::Calendar)
    );

    // A quarter-hour outside the Rumpfjahr is refused.
    let (nb, ne) = (s(&[1]), s(&[1]));
    let err = Pauschaloption {
        z1nb: &nb,
        einspeisung: Einspeisung::Z1(&ne),
        sp: Gate::Always,
        basis: Pauschalbasis::Ladepunkt,
        anlagen: vec![SolarAnlage {
            kwp: dec!(8),
            aw: Gate::Always,
        }],
        zeitraum: Zeitraum::Rumpfjahr {
            erster_tag: date!(2026 - 01 - 01),
            letzter_tag: date!(2026 - 05 - 16),
        },
    }
    .jahr()
    .unwrap_err();
    assert_eq!(err, EegError::OutsidePeriod { at: T0 });
}

/// A substituted value is billable: the month is computed from it as from a
/// measured one, and the quarter-hour that used it says so. Only Faulty and
/// Unknown values are refused.
#[test]
fn a_substituted_input_is_used_and_reported_per_quarter_hour() {
    let measured = a1_v1();
    let aw = flags(&[1, 1, 1, 0]);
    let reference = run(
        &measured,
        Basisfall::Stromspeicher,
        Anlagen::Eine(Gate::Flagged(&aw)),
    );

    let mut c = a1_v1();
    let ivs = c
        .z1nb
        .iter()
        .enumerate()
        .map(|(i, iv)| {
            if i == 1 {
                iv.clone().with_quality(QualityFlag::Substituted)
            } else {
                iv.clone()
            }
        })
        .collect();
    c.z1nb = Series::new(Resolution::QUARTER_HOUR, DayBoundary::Strom, ivs).unwrap();
    let m = run(
        &c,
        Basisfall::Stromspeicher,
        Anlagen::Eine(Gate::Flagged(&aw)),
    );

    assert_eq!(
        (m.f3, m.f19, m.f21),
        (reference.f3, reference.f19, reference.f21)
    );
    let qualities: Vec<QualityFlag> = m.viertelstunden.iter().map(|v| v.quality).collect();
    assert_eq!(
        qualities,
        [
            QualityFlag::Measured,
            QualityFlag::Substituted,
            QualityFlag::Measured,
            QualityFlag::Measured
        ]
    );
}
