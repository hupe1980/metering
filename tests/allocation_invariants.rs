//! The § 42b / § 42c allocation invariants, under random communities.
//!
//! Three of these are exact-arithmetic identities the output must satisfy in
//! every interval, and one is the statutory ceiling of § 42b Abs. 5:
//!
//! > die rechnerisch aufteilbare Strommenge \[ist\] begrenzt … auf die
//! > Strommenge, die innerhalb eines 15-Minuten-Zeitintervalls in der
//! > Solaranlage erzeugt oder von allen teilnehmenden Letztverbrauchern
//! > verbraucht wird, je nachdem welche dieser Strommengen geringer ist.
//!
//! `compute_community_allocation` does not clamp to that figure. It does not
//! have to: with fractions summing to at most 1, the per-participant `Pos()`
//! cap of the BDEW Anwendungshilfe already implies it. That is an argument,
//! and an argument about a billed quantity is worth a proof obligation — so
//! the inequality is asserted over generated communities rather than reasoned
//! about once in a doc comment.

use metering::{AllocationKey, MeterInterval, QualityFlag, compute_community_allocation};
use proptest::prelude::*;
use rust_decimal::Decimal;
use std::collections::{BTreeMap, HashMap};
use time::macros::datetime;
use time::{Duration, OffsetDateTime};

const BASE: OffsetDateTime = datetime!(2026-06-01 0:00 UTC);

/// kWh in a quarter-hour, three decimal places, never negative — the
/// Codeliste v2.5c §2.1 shape for a MaKo channel.
fn arb_kwh() -> impl Strategy<Value = Decimal> {
    (0i64..40_000).prop_map(|milli| Decimal::new(milli, 3))
}

fn series(values: Vec<Decimal>) -> Vec<MeterInterval> {
    values
        .into_iter()
        .enumerate()
        .map(|(i, value)| {
            let from = BASE + Duration::minutes(15 * i as i64);
            MeterInterval {
                from,
                to: from + Duration::minutes(15),
                value,
                quality: QualityFlag::Measured,
                obis_code: None,
            }
        })
        .collect()
}

/// A plant and `n` participants over the same `len` quarter-hours.
fn arb_community() -> impl Strategy<Value = (HashMap<String, Vec<MeterInterval>>, Vec<String>)> {
    (1usize..6, 1usize..12).prop_flat_map(|(participants, len)| {
        (
            prop::collection::vec(arb_kwh(), len),
            prop::collection::vec(prop::collection::vec(arb_kwh(), len), participants),
        )
            .prop_map(|(plant, tenants)| {
                let mut sources = HashMap::new();
                sources.insert("PLANT".to_owned(), series(plant));
                let mut ids = Vec::new();
                for (i, values) in tenants.into_iter().enumerate() {
                    let id = format!("T{i}");
                    sources.insert(id.clone(), series(values));
                    ids.push(id);
                }
                (sources, ids)
            })
    })
}

/// Fractions that are each positive and sum to at most 1 — the precondition
/// the pool-cap argument rests on.
///
/// Built from generated weights rather than generated directly, because the
/// property under test is about a *subscribable* community: an
/// over-subscribed one is rejected by the constructor and never reaches the
/// arithmetic. Integer division only ever rounds a share down, so the sum
/// stays at or below one.
fn fractions_from(ids: &[String], weights: &[i64]) -> BTreeMap<String, Decimal> {
    let total: i64 = weights.iter().map(|w| w.max(&1)).sum();
    ids.iter()
        .zip(weights)
        .map(|(id, w)| (id.clone(), Decimal::new(w.max(&1) * 1_000_000 / total, 6)))
        .collect()
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// Every identity, on both keys, in every interval.
    #[test]
    fn the_allocation_identities_hold_for_any_community(
        (sources, ids) in arb_community(),
        proportional in any::<bool>(),
    ) {
        let key = if proportional {
            AllocationKey::Proportional { participants: ids.clone() }
        } else {
            // A deterministic, valid constant key: equal shares.
            let each = Decimal::ONE / Decimal::from(ids.len() as u64);
            AllocationKey::Constant {
                fractions: ids.iter().map(|id| (id.clone(), each)).collect(),
            }
        };

        let out = compute_community_allocation("PLANT", &key, &sources)
            .expect("every named series is present and the shares are valid");

        for iv in &out {
            // Per participant: consumption splits exactly into credited and drawn.
            for p in &iv.participants {
                prop_assert_eq!(p.consumption, p.allocated + p.net_grid_draw);
                prop_assert!(p.allocated <= p.consumption, "credited more than drawn");
                prop_assert!(p.allocated <= p.share, "credited more than the key allows");
                prop_assert!(p.net_grid_draw >= Decimal::ZERO);
            }

            // Per community: generation splits exactly into credited and exported.
            prop_assert_eq!(iv.generation, iv.total_allocated() + iv.surplus_to_grid);
            prop_assert_eq!(
                iv.total_consumption,
                iv.total_allocated() + iv.total_net_grid_draw()
            );

            // § 42b Abs. 5: the pool never exceeds the lesser of the two.
            prop_assert_eq!(iv.pool_cap, iv.generation.min(iv.total_consumption));
            prop_assert!(
                iv.total_allocated() <= iv.pool_cap,
                "allocated {} exceeds the § 42b Abs. 5 ceiling {}",
                iv.total_allocated(),
                iv.pool_cap
            );
        }
    }

    /// The constant key with uneven fractions — the case that *can*
    /// over-subscribe, and so the one the ceiling argument really rests on.
    #[test]
    fn uneven_constant_fractions_respect_the_ceiling(
        (sources, ids) in arb_community(),
        weights in prop::collection::vec(1i64..100, 1..6),
    ) {
        let weights: Vec<i64> = ids.iter().enumerate().map(|(i, _)| weights[i % weights.len()]).collect();
        let fractions = fractions_from(&ids, &weights);
        let sum: Decimal = fractions.values().copied().sum();
        prop_assert!(sum <= Decimal::ONE, "the community must stay subscribable: {sum}");

        let key = AllocationKey::Constant { fractions };
        for iv in compute_community_allocation("PLANT", &key, &sources).unwrap() {
            prop_assert!(iv.total_allocated() <= iv.pool_cap);
            prop_assert!(iv.surplus_to_grid >= Decimal::ZERO);
        }
    }

    /// Listing the participants in another order is the same community — the
    /// proportional denominator must not depend on iteration order.
    #[test]
    fn the_participant_order_does_not_change_the_result(
        (sources, ids) in arb_community(),
    ) {
        let forward = AllocationKey::Proportional { participants: ids.clone() };
        let mut backwards = ids;
        backwards.reverse();
        let reversed = AllocationKey::Proportional { participants: backwards };

        prop_assert_eq!(
            compute_community_allocation("PLANT", &forward, &sources).unwrap(),
            compute_community_allocation("PLANT", &reversed, &sources).unwrap(),
        );
    }
}
