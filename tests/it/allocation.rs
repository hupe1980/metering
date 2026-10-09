//! The § 42b / § 42c allocation invariants, under random communities.
//!
//! Exact-arithmetic identities the output must satisfy in every interval, the
//! statutory ceiling of § 42b Abs. 5 —
//!
//! > die rechnerisch aufteilbare Strommenge \[ist\] begrenzt … auf die
//! > Strommenge, die innerhalb eines 15-Minuten-Zeitintervalls in der
//! > Solaranlage erzeugt oder von allen teilnehmenden Letztverbrauchern
//! > verbraucht wird, je nachdem welche dieser Strommengen geringer ist.
//!
//! — and the property that makes one engine enough: a tenant's grid draw from
//! the community rows is the number its own Berechnungsformel computes.

use metering::allocation::community::{AllocationKey, allocate};
use metering::allocation::formula::{Formula, SplitFactor};
use metering::{DayBoundary, Direction, MeloId, MeterInterval, QualityFlag, Resolution, Series};
use proptest::prelude::*;
use rust_decimal::Decimal;
use std::collections::BTreeMap;
use time::macros::datetime;
use time::{Duration, OffsetDateTime};

const BASE: OffsetDateTime = datetime!(2026-06-01 0:00 UTC);

/// kWh in a quarter-hour, three decimal places, never negative.
fn arb_kwh() -> impl Strategy<Value = Decimal> + Clone {
    (0i64..40_000).prop_map(|milli| Decimal::new(milli, 3))
}

/// kWh that is negative a fifth of the time — a meter reporting a sign error.
fn arb_signed_kwh() -> impl Strategy<Value = Decimal> + Clone {
    prop_oneof![
        4 => arb_kwh(),
        1 => (1i64..5_000).prop_map(|milli| Decimal::new(-milli, 3)),
    ]
}

fn series(values: &[Decimal]) -> Series {
    let ivs = values
        .iter()
        .enumerate()
        .map(|(i, value)| {
            MeterInterval::quarter_hour(
                BASE + Duration::minutes(15 * i as i64),
                *value,
                QualityFlag::Measured,
            )
            .unwrap()
        })
        .collect();
    Series::new(Resolution::QUARTER_HOUR, DayBoundary::Strom, ivs).unwrap()
}

fn melo(i: usize) -> MeloId {
    format!("DE0001234567890000000000000{i:06}")
        .parse()
        .unwrap()
}

const PLANT: usize = 999_999;

/// A plant and `n` participants over the same `len` quarter-hours.
fn arb_community(
    value: impl Strategy<Value = Decimal> + Clone + 'static,
) -> impl Strategy<Value = (Series, Vec<Series>)> {
    (1usize..6, 1usize..12).prop_flat_map(move |(participants, len)| {
        (
            prop::collection::vec(value.clone(), len),
            prop::collection::vec(prop::collection::vec(value.clone(), len), participants),
        )
            .prop_map(|(plant, tenants)| {
                (series(&plant), tenants.iter().map(|t| series(t)).collect())
            })
    })
}

/// Fractions that are each positive, at most six places, summing to ≤ 1.
fn fractions(n: usize, weights: &[i64]) -> BTreeMap<MeloId, SplitFactor> {
    let w: Vec<i64> = (0..n).map(|i| weights[i % weights.len()].max(1)).collect();
    let total: i64 = w.iter().sum();
    (0..n)
        .map(|i| {
            let f = Decimal::new((w[i] * 1_000_000 / total).max(1), 6);
            (melo(i), SplitFactor::new(f).unwrap())
        })
        .collect()
}

fn participants(tenants: &[Series]) -> Vec<(MeloId, &Series)> {
    tenants
        .iter()
        .enumerate()
        .map(|(i, s)| (melo(i), s))
        .collect()
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// Every identity, for every key, in every interval — negative values
    /// included, which are clamped and never abort the run.
    #[test]
    fn the_allocation_identities_hold_for_any_community(
        (plant, tenants) in arb_community(arb_signed_kwh()),
        key in 0usize..4,
        weights in prop::collection::vec(1i64..100, 1..6),
    ) {
        let parts = participants(&tenants);
        let key = match key {
            0 => AllocationKey::Proportional,
            1 => AllocationKey::EqualShares,
            2 => AllocationKey::Constant { fractions: fractions(parts.len(), &weights) },
            _ => AllocationKey::Cascading {
                weights: (0..parts.len()).map(|i| (melo(i), Decimal::from(weights[i % weights.len()]))).collect(),
            },
        };
        let rows = allocate(&plant, &parts, &key).expect("a valid key never fails");
        prop_assert_eq!(rows.len(), plant.len());

        for row in &rows {
            prop_assert_eq!(row.allocated() + row.residual, row.pool(), "conservation");
            prop_assert!(row.residual >= Decimal::ZERO, "over-allocated");
            let demand: Decimal = row.shares.iter().map(|s| s.consumption.max(Decimal::ZERO)).sum();
            prop_assert!(
                row.allocated() <= row.pool().min(demand),
                "allocated {} exceeds the § 42b Abs. 5 ceiling",
                row.allocated()
            );
            for s in &row.shares {
                prop_assert!(s.allocated >= Decimal::ZERO);
                prop_assert!(s.allocated <= s.consumption.max(Decimal::ZERO), "credited more than drawn");
                prop_assert!(s.net_grid_draw() >= Decimal::ZERO);
                prop_assert_eq!(s.clamped(), s.consumption < Decimal::ZERO);
            }
        }
    }

    /// One engine: each tenant's grid draw in the community rows is exactly
    /// what its own § 42b Berechnungsformel evaluates to.
    #[test]
    fn a_tenant_formula_and_the_community_never_disagree(
        (plant, tenants) in arb_community(arb_kwh()),
        proportional in any::<bool>(),
        weights in prop::collection::vec(1i64..100, 1..6),
    ) {
        let parts = participants(&tenants);
        let ids: Vec<MeloId> = parts.iter().map(|(m, _)| *m).collect();
        let fractions = fractions(parts.len(), &weights);
        let key = if proportional {
            AllocationKey::Proportional
        } else {
            AllocationKey::Constant { fractions: fractions.clone() }
        };
        let rows = allocate(&plant, &parts, &key).unwrap();

        let sources = |m: &MeloId, d: Direction| match d {
            Direction::Export => (*m == melo(PLANT)).then_some(&plant),
            Direction::Import => parts.iter().find(|(p, _)| p == m).map(|(_, s)| *s),
        };
        for (i, tenant) in ids.iter().enumerate() {
            let formula = if proportional {
                Formula::proportional_share(*tenant, melo(PLANT), &ids).unwrap()
            } else {
                Formula::constant_share(*tenant, melo(PLANT), fractions[tenant]).unwrap()
            };
            let evaluated = formula.eval(sources).unwrap();
            for (row, iv) in rows.iter().zip(evaluated.series.iter()) {
                prop_assert_eq!(row.from, iv.from());
                let share = &row.shares[i];
                prop_assert_eq!(share.net_grid_draw(), iv.value(), "{} at {}", tenant, row.from);
            }
        }
    }

    /// Listing the participants in another order is the same community.
    #[test]
    fn the_participant_order_does_not_change_the_result(
        (plant, tenants) in arb_community(arb_kwh()),
    ) {
        let forward = participants(&tenants);
        let mut backward = forward.clone();
        backward.reverse();
        let a = allocate(&plant, &forward, &AllocationKey::Proportional).unwrap();
        let b = allocate(&plant, &backward, &AllocationKey::Proportional).unwrap();
        for (x, y) in a.iter().zip(&b) {
            prop_assert_eq!(x.residual, y.residual);
            for s in &x.shares {
                prop_assert_eq!(Some(s), y.share(&s.participant));
            }
        }
    }

    /// A cascade never credits a participant less than its first-pass offer
    /// allows — it only re-offers what a ceiling refused — and exhausts a pool
    /// the ceilings can hold, up to the quotient cut: a millionth per
    /// participant.
    #[test]
    fn a_cascade_dominates_its_first_pass(
        (plant, tenants) in arb_community(arb_kwh()),
    ) {
        let parts = participants(&tenants);
        let key = AllocationKey::Cascading {
            weights: (0..parts.len()).map(|i| (melo(i), Decimal::ONE)).collect(),
        };
        for row in allocate(&plant, &parts, &key).unwrap() {
            for s in &row.shares {
                prop_assert!(s.allocated >= s.share.min(s.consumption), "{}", s.participant);
            }
            let demand: Decimal = row.shares.iter().map(|x| x.consumption).sum();
            let slack = Decimal::new(i64::try_from(parts.len()).unwrap(), 6);
            prop_assert!(row.allocated() + slack >= row.pool().min(demand));
        }
    }
}
