//! Order independence, as a mechanical property rather than a promise.
//!
//! Several entry points in this crate document that the order of the input
//! does not affect the result: `Series::new` sorts, so `validate`,
//! `substitute`, `aggregate` and `resample` see one canonical order, and
//! `to_lastgang` sorts before differencing. A MSCONS delivery
//! merged from two files, a database query without an `ORDER BY`, and a
//! `HashMap` iteration all produce a shuffled series, so the promise is one
//! consumers rely on without noticing.
//!
//! It is also a promise that has been broken twice, in ways no example-based
//! test caught, because both needed a **tie**:
//!
//! - `QualityFlag::severity_rank` gave `Corrected` and `Substituted` the same
//!   rank, and `Faulty` and `Unknown` the same rank. `worse_of` keeps `self` on
//!   a tie, so a resampled bucket — or a virtual meter, or a differenced
//!   Lastgang — took whichever of the two the caller happened to list first.
//! - `aggregate` kept the first interval it saw at the maximum power, so a flat
//!   load reported a `spitzenleistung_at` that depended on the slice order.
//!
//! This module is the counterpart of `string_canonicalisation`: one property,
//! asserted over random input, for every function the crate makes the claim
//! about.

use metering::allocation::session::{
    MeterSample, SessionSplitConfig, merge_sessions, split_session,
};
use metering::time::holiday::Bundesland;
use metering::vee::substitute::{Policy, SubstitutionReason, substitute};
use metering::vee::validation::{Rules, validate};
use metering::{
    DayBoundary, MeloId, MeterInterval, QualityFlag, Resolution, Series,
    allocation::community::AllocationKey, allocation::community::allocate,
    billing::aggregation::aggregate, billing::aggregation::sum_by_direction,
    billing::zaehlzeit::Zaehlzeitdefinition, series::reading::LastgangConfig,
    series::reading::MeterReading, series::reading::to_lastgang, series::resample::resample,
};
use proptest::prelude::*;
use rust_decimal::Decimal;
use time::macros::{date, datetime};
use time::{Duration, OffsetDateTime};

/// The instant every generated series starts at — a Berlin midnight, so the
/// daily and monthly grids line up with it.
const BASE: OffsetDateTime = datetime!(2025-12-31 23:00 UTC);

/// One generated interval: which grid slot it sits on, what it carries, how
/// good it is, and the key its shuffled position is derived from.
#[derive(Debug, Clone)]
struct Sample {
    slot: i64,
    value: Decimal,
    quality: QualityFlag,
    shuffle_key: u64,
}

fn arb_sample(value: BoxedStrategy<Decimal>) -> impl Strategy<Value = Sample> {
    (
        0i64..320, // ~3.3 days of quarter-hours, so gaps are likely
        value,
        0usize..QualityFlag::ALL.len(),
        any::<u64>(),
    )
        .prop_map(|(slot, value, quality, shuffle_key)| Sample {
            slot,
            value,
            quality: QualityFlag::ALL[quality],
            shuffle_key,
        })
}

fn series_of(value: BoxedStrategy<Decimal>) -> impl Strategy<Value = Vec<Sample>> {
    prop::collection::vec(arb_sample(value), 0..90).prop_map(|mut samples| {
        samples.sort_by_key(|s| s.slot);
        samples.dedup_by_key(|s| s.slot);
        samples
    })
}

/// A set of samples on distinct slots, ascending — the "ordered" input.
///
/// Half the series are drawn from a **coarse** half-kWh grid, and that half is
/// what gives this file its teeth. Order dependence only becomes visible on a
/// **tie**, and ties do not happen by accident in a 200 000-wide value space. A
/// generator that merely *mixes* coarse and fine values within one series does
/// not produce them either: the fine values win the maximum. It has to be whole
/// coarse *series*, so that several intervals share the peak; the fine half
/// keeps the outlier, coverage and gap rules seeing realistic variety.
fn arb_series() -> impl Strategy<Value = Vec<Sample>> {
    prop_oneof![
        // −10.0 … 19.5 in 0.5 kWh steps: ties at the maximum are the norm.
        series_of((-20i64..40).prop_map(|n| Decimal::new(n * 500, 3)).boxed()),
        // −20.000 … 200.000 kWh: three decimal places, ties vanishingly rare.
        series_of(
            (-20_000i64..200_000)
                .prop_map(|milli| Decimal::new(milli, 3))
                .boxed()
        ),
    ]
}

fn interval(sample: &Sample) -> MeterInterval {
    let from = BASE + Duration::minutes(15 * sample.slot);
    MeterInterval::new(
        from,
        from + Duration::minutes(15),
        sample.value,
        sample.quality,
    )
    .unwrap()
}

/// The same intervals twice: in timestamp order, and in an order derived from
/// the generated shuffle keys.
fn both_orders(samples: &[Sample]) -> (Vec<MeterInterval>, Vec<MeterInterval>) {
    let ordered: Vec<MeterInterval> = samples.iter().map(interval).collect();
    let mut permuted: Vec<&Sample> = samples.iter().collect();
    permuted.sort_by_key(|s| (s.shuffle_key, s.slot));
    let shuffled = permuted.into_iter().map(interval).collect();
    (ordered, shuffled)
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// `Series::new` is the one place order is resolved: a shuffled slice
    /// builds the identical series, so every pipeline step — validation and
    /// its grade, substitution and its audit trail, aggregation including the
    /// instant of a tied peak, and resampling including bucket quality —
    /// reaches the identical result.
    #[test]
    fn the_pipeline_is_order_independent(samples in arb_series()) {
        let (ordered, shuffled) = both_orders(&samples);
        let a = Series::new(Resolution::QUARTER_HOUR, DayBoundary::Strom, ordered).unwrap();
        let b = Series::new(Resolution::QUARTER_HOUR, DayBoundary::Strom, shuffled).unwrap();
        prop_assert_eq!(&a, &b);

        let days = DayBoundary::Strom.day(date!(2026 - 01 - 01)).unwrap();
        let rules = Rules::strom(days, BASE + Duration::days(2), None);
        let (ra, rb) = (validate(&a, &rules), validate(&b, &rules));
        prop_assert_eq!(&ra, &rb);
        let policy = Policy::strom(Bundesland::Be, SubstitutionReason::CommunicationFailure);
        prop_assert_eq!(substitute(&a, &ra, &policy), substitute(&b, &rb, &policy));
        prop_assert_eq!(aggregate(&a, days), aggregate(&b, days));
        for target in [Resolution::Hour, Resolution::Day] {
            prop_assert_eq!(resample(&a, target), resample(&b, target));
        }
    }

    /// Per-register sums, and the straddling, unassigned and non-billable
    /// remainders.
    #[test]
    fn the_register_split_is_order_independent(samples in arb_series()) {
        let (ordered, shuffled) = both_orders(&samples);
        let zzd = Zaehlzeitdefinition::modul_3(
            "NB-14A-3",
            date!(2026 - 01 - 01),
            (17 * 60, 20 * 60),
            (22 * 60, 6 * 60),
            &metering::billing::zaehlzeit::Quarter::ALL,
        )
        .unwrap();
        let (a, b) = (zzd.split_energy(&ordered).unwrap(), zzd.split_energy(&shuffled).unwrap());
        prop_assert_eq!(&a.per_register, &b.per_register);
        let sorted = |v: &[metering::MeterInterval]| {
            let mut v = v.to_vec();
            v.sort_by_key(metering::MeterInterval::from);
            v
        };
        prop_assert_eq!(sorted(&a.straddling), sorted(&b.straddling));
        prop_assert_eq!(sorted(&a.unassigned), sorted(&b.unassigned));
        prop_assert_eq!(sorted(&a.non_billable), sorted(&b.non_billable));
    }

    /// Differencing sorts its input first, so the derived Lastgang, the
    /// reconstructed rollovers and the refused spans are all the same.
    #[test]
    fn differencing_is_order_independent(samples in arb_series()) {
        let readings: Vec<MeterReading> = samples
            .iter()
            .scan(Decimal::new(500_000, 0), |register, s| {
                *register += s.value.abs();
                Some(MeterReading {
                    at: BASE + Duration::minutes(15 * s.slot),
                    value: *register,
                    quality: s.quality,
                    obis_code: None,
                })
            })
            .collect();
        let mut permuted: Vec<MeterReading> = readings.clone();
        permuted.sort_by_key(|r| (r.value.to_string(), r.at));

        let cfg = LastgangConfig::strom().register_digits(6);
        prop_assert_eq!(to_lastgang(&readings, &cfg), to_lastgang(&permuted, &cfg));
    }

    /// The worst flag of a set is the worst flag of the same set reversed —
    /// the property the tied ranks broke, stated on its own.
    #[test]
    fn the_worst_quality_flag_does_not_depend_on_order(
        picks in prop::collection::vec(0usize..QualityFlag::ALL.len(), 0..12),
    ) {
        let flags: Vec<QualityFlag> = picks.iter().map(|&i| QualityFlag::ALL[i]).collect();
        let mut reversed = flags.clone();
        reversed.reverse();
        prop_assert_eq!(
            QualityFlag::worst_of(flags.iter().copied()),
            QualityFlag::worst_of(reversed.iter().copied()),
        );
    }
}

// ── allocation, sessions and the directional balance ─────────────────────────

/// A plant and up to five participants over four quarter-hours, with values
/// drawn coarse enough to tie and fine enough to repeat.
fn arb_community() -> impl Strategy<Value = (Vec<Decimal>, Vec<Vec<Decimal>>)> {
    let kwh = || {
        prop_oneof![
            (0i64..40).prop_map(|n| Decimal::new(n * 500, 3)),
            (0i64..40_000).prop_map(|milli| Decimal::new(milli, 3)),
        ]
    };
    (
        prop::collection::vec(kwh(), 4),
        prop::collection::vec(prop::collection::vec(kwh(), 4), 0..6),
    )
}

fn quarter_hours(values: &[Decimal]) -> Series {
    let ivs = values
        .iter()
        .enumerate()
        .map(|(i, v)| {
            MeterInterval::quarter_hour(
                BASE + Duration::minutes(15 * i as i64),
                *v,
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

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// Listing the participants in another order permutes the shares and
    /// changes nothing else, for every key.
    #[test]
    fn allocation_is_order_independent(
        (plant, tenants) in arb_community(),
        key in 0usize..3,
    ) {
        let plant = quarter_hours(&plant);
        let series: Vec<Series> = tenants.iter().map(|t| quarter_hours(t)).collect();
        let forward: Vec<(MeloId, &Series)> = series.iter().enumerate().map(|(i, s)| (melo(i), s)).collect();
        let mut backward = forward.clone();
        backward.reverse();
        let key = match key {
            0 => AllocationKey::Proportional,
            1 => AllocationKey::EqualShares,
            _ => AllocationKey::Cascading { weights: forward.iter().map(|(m, _)| (*m, Decimal::ONE)).collect() },
        };

        let a = allocate(&plant, &forward, &key).unwrap();
        let b = allocate(&plant, &backward, &key).unwrap();
        for (x, y) in a.iter().zip(&b) {
            prop_assert_eq!(x.residual, y.residual);
            for share in &x.shares {
                prop_assert_eq!(Some(share), y.share(&share.participant));
            }
        }
    }

    /// Register readings describe the shape of a session, not the order they
    /// were collected in — an OCPP backlog flushed out of sequence must place
    /// the same kWh in the same slots.
    #[test]
    fn a_session_split_is_order_independent(
        steps in prop::collection::vec((1i64..5_000, 0i64..4_000), 1..6),
    ) {
        let mut at = BASE + Duration::seconds(37);
        let mut reading = Decimal::ZERO;
        let mut samples = vec![MeterSample::new(at, reading)];
        for (step, milli) in steps {
            at += Duration::seconds(step);
            reading += Decimal::new(milli, 3);
            samples.push(MeterSample::new(at, reading));
        }

        let cfg = SessionSplitConfig::quarter_hourly();
        let mut reversed = samples.clone();
        reversed.reverse();
        prop_assert_eq!(split_session(&samples, &cfg), split_session(&reversed, &cfg));
    }

    /// Merging groups by slot and folds the quality with `worse_of`, whose
    /// ranks are a strict total order — so which session was listed first
    /// cannot reach the answer. This is the exact shape of the two defects
    /// this file exists for.
    #[test]
    fn merging_sessions_is_order_independent(
        offsets in prop::collection::vec(0i64..7_200, 1..5),
        energies in prop::collection::vec(
            (0i64..40_000).prop_map(|milli| Decimal::new(milli, 3)),
            1..5,
        ),
    ) {
        let cfg = SessionSplitConfig::quarter_hourly();
        let series: Vec<Series> = offsets
            .iter()
            .zip(energies.iter().cycle())
            .map(|(offset, energy)| {
                let from = BASE + Duration::seconds(*offset);
                let samples = [
                    MeterSample::new(from, Decimal::ZERO),
                    MeterSample::new(from + Duration::minutes(37), *energy),
                ];
                split_session(&samples, &cfg).unwrap()
            })
            .collect();

        let mut reversed = series.clone();
        reversed.reverse();
        prop_assert_eq!(merge_sessions(&cfg, &series), merge_sessions(&cfg, &reversed));
    }

    /// Three running sums, so the balance cannot depend on the slice order.
    #[test]
    fn the_directional_balance_is_order_independent(samples in arb_series()) {
        let codes = ["1-0:1.8.0", "1-0:2.8.0", "1-0:3.8.0"];
        let tag = |ivs: Vec<MeterInterval>| -> Vec<MeterInterval> {
            ivs.into_iter()
                .enumerate()
                .map(|(i, iv)| iv.with_obis(codes[i % 3].parse().unwrap()))
                .collect()
        };
        let (sorted, shuffled) = both_orders(&samples);
        // Tag by slot, not by position, so both orders carry the same codes.
        let by_slot = |ivs: Vec<MeterInterval>| -> Vec<MeterInterval> {
            let mut v = ivs;
            v.sort_by_key(|iv| iv.from());
            tag(v)
        };
        let a = sum_by_direction(&by_slot(sorted));
        let mut b = by_slot(shuffled);
        b.reverse();
        prop_assert_eq!(a, sum_by_direction(&b));
    }
}
