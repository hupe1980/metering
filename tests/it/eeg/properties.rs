//! Properties of the per-interval operators: the coincident minimum is
//! bounded by the minimum of the sums, and the A5-/P4-Variante are identities
//! of A5/P4 with identical gates.

use std::collections::BTreeSet;

use metering::eeg::mispel::abgrenzung::{Abgrenzungsoption, Anlagen, Basisfall, GewichteteAnlage};
use metering::eeg::mispel::pauschal::{Pauschalbasis, Pauschaloption, SolarAnlage, Zeitraum};
use metering::eeg::mispel::{Einspeisung, Gate};
use metering::eeg::zeitgleichheit::zeitgleichheit;
use metering::prelude::*;
use proptest::prelude::*;
use time::macros::datetime;
use time::{Duration, OffsetDateTime};

const T0: OffsetDateTime = datetime!(2026-06-01 0:00 UTC);

fn series(values: &[u32]) -> Series {
    let ivs = values
        .iter()
        .enumerate()
        .map(|(i, &v)| {
            MeterInterval::quarter_hour(
                T0 + Duration::minutes(15 * i as i64),
                Decimal::from(v),
                QualityFlag::Measured,
            )
            .unwrap()
        })
        .collect();
    Series::new(Resolution::QUARTER_HOUR, DayBoundary::Strom, ivs).unwrap()
}

fn columns(n: usize) -> impl Strategy<Value = Vec<(u32, u32, u32, u32, bool)>> {
    proptest::collection::vec(
        (0..50u32, 0..50u32, 0..50u32, 0..50u32, any::<bool>()),
        1..n,
    )
}

proptest! {
    /// Σ min(a, b) ≤ min(Σ a, Σ b), and each counted share is bounded by both.
    #[test]
    fn coincident_sum_is_bounded_by_the_min_of_sums(rows in columns(40)) {
        let a: Vec<u32> = rows.iter().map(|r| r.0).collect();
        let b: Vec<u32> = rows.iter().map(|r| r.1).collect();
        let z = zeitgleichheit(&series(&a), &series(&b)).unwrap();
        let (sa, sb): (u32, u32) = (a.iter().sum(), b.iter().sum());
        prop_assert!(z.total <= Decimal::from(sa.min(sb)));
        for iv in &z.intervals {
            prop_assert!(iv.counted <= iv.consumption && iv.counted <= iv.withdrawal);
        }
    }

    /// A5 with identical gates splits (32) by ZF exactly — the A5-Variante.
    #[test]
    fn a5_variante_is_an_identity(rows in columns(30), wa in 1..20u32, wb in 1..20u32) {
        let col = |f: fn(&(u32, u32, u32, u32, bool)) -> u32| series(&rows.iter().map(f).collect::<Vec<_>>());
        let (nb, ne, v, e) = (col(|r| r.0), col(|r| r.1), col(|r| r.2), col(|r| r.3));
        let aw: BTreeSet<OffsetDateTime> = rows
            .iter()
            .enumerate()
            .filter(|(_, r)| r.4)
            .map(|(i, _)| T0 + Duration::minutes(15 * i as i64))
            .collect();
        let run = |anlagen| Abgrenzungsoption {
            z1nb: &nb,
            einspeisung: Einspeisung::Z1(&ne),
            z2v: &v,
            z2e: &e,
            basisfall: Basisfall::Stromspeicher,
            anlagen,
        }
        .monat()
        .unwrap();
        let single = run(Anlagen::Eine(Gate::Flagged(&aw)));
        let split = run(Anlagen::Gleichartig(vec![
            GewichteteAnlage { leistung: Decimal::from(wa), aw: Gate::Flagged(&aw) },
            GewichteteAnlage { leistung: Decimal::from(wb), aw: Gate::Flagged(&aw) },
        ]));
        let total = Decimal::from(wa + wb);
        let want_a = single.anlagen[0].f32 * Decimal::from(wa) / total;
        // Equal up to the last of 28 significant digits.
        prop_assert!((split.anlagen[0].f32 - want_a).abs() < Decimal::new(1, 20));
        prop_assert!((split.foerderfaehig().unwrap() - single.anlagen[0].f32).abs() < Decimal::new(1, 20));
        prop_assert_eq!(split.f21, single.f21);
    }

    /// P4 with identical gates gives (P16k) = ZFk • (P15) — the P4-Variante.
    #[test]
    fn p4_variante_is_an_identity(rows in columns(30), ka in 1..20u32, kb in 1..20u32) {
        let nb = series(&rows.iter().map(|r| r.0 * 100).collect::<Vec<_>>());
        let ne = series(&rows.iter().map(|r| r.1 * 100).collect::<Vec<_>>());
        let aw: BTreeSet<OffsetDateTime> = rows
            .iter()
            .enumerate()
            .filter(|(_, r)| r.4)
            .map(|(i, _)| T0 + Duration::minutes(15 * i as i64))
            .collect();
        let run = |anlagen| Pauschaloption {
            z1nb: &nb,
            einspeisung: Einspeisung::Z1(&ne),
            sp: Gate::Always,
            basis: Pauschalbasis::Ladepunkt,
            anlagen,
            zeitraum: Zeitraum::Kalenderjahr(2026),
        }
        .jahr()
        .unwrap();
        let single = run(vec![SolarAnlage { kwp: Decimal::from(ka + kb), aw: Gate::Flagged(&aw) }]);
        let split = run(vec![
            SolarAnlage { kwp: Decimal::from(ka), aw: Gate::Flagged(&aw) },
            SolarAnlage { kwp: Decimal::from(kb), aw: Gate::Flagged(&aw) },
        ]);
        let want = single.anlagen[0].p15 * Decimal::from(ka) / Decimal::from(ka + kb);
        prop_assert!((split.anlagen[0].p16 - want).abs() < Decimal::new(1, 20));
        prop_assert_eq!(split.p11, single.p11);
    }
}
