//! Order independence, as a mechanical property rather than a promise.
//!
//! Several entry points in this crate document that the order of the input
//! slice does not affect the result — `aggregate` says so in as many words,
//! `validate_intervals` evaluates its adjacency rules in timestamp order
//! whatever arrives, `resample` says the input "need not be contiguous or
//! sorted", and `to_lastgang` sorts before differencing. A MSCONS delivery
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
//! This file is the counterpart of `string_canonicalisation.rs`: one property,
//! asserted over random input, for every function the crate makes the claim
//! about.

use metering::{
    AggregationConfig, DayBoundary, FillGapsConfig, IntervalResolution, LastgangConfig,
    MeterInterval, MeterReading, QualityConfig, QualityFlag, ResampleConfig, ValidationConfig,
    ValidationRuleId, Zaehlzeitdefinition, aggregate, fill_gaps, resample, score_intervals,
    to_lastgang, validate_intervals,
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
/// what gives this file its teeth. A tie is what makes a maximum
/// order-dependent, and ties do not happen by accident in a 200 000-wide value
/// space — the `spitzenleistung_at` bug survived a fine-grained generator for
/// exactly that reason, and survives one that merely *mixes* coarse and fine
/// values within a series, because the fine ones win the maximum. Coarse
/// *series* make several intervals share the peak; fine ones keep the outlier,
/// coverage and gap rules seeing realistic variety.
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
    MeterInterval {
        from,
        to: from + Duration::minutes(15),
        value: sample.value,
        quality: sample.quality,
        obis_code: None,
    }
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

/// A validation finding, stripped of everything that legitimately depends on
/// where in the *caller's* slice the interval sat.
///
/// `interval_index` points into the slice the caller passed, so it moves under
/// a permutation by design — that is the field's whole purpose. Everything
/// else describes the data, and must not move.
type Finding = (
    &'static str,
    metering::ValidationSeverity,
    Option<OffsetDateTime>,
    Option<Decimal>,
    String,
);

trait Issues {
    fn issues(&self) -> &[metering::ValidationIssue];
}

impl Issues for metering::ValidationResult {
    fn issues(&self) -> &[metering::ValidationIssue] {
        &self.issues
    }
}

impl Issues for metering::QualityReport {
    fn issues(&self) -> &[metering::ValidationIssue] {
        &self.issues
    }
}

fn findings(result: &impl Issues) -> Vec<Finding> {
    let mut out: Vec<Finding> = result
        .issues()
        .iter()
        // V11 *reports* the disorder, so its message names slice positions and
        // is expected to differ. Its presence is asserted separately below.
        .filter(|i| i.rule_id != ValidationRuleId::UnorderedSeries)
        .map(|i| {
            (
                i.rule_id.as_str(),
                i.severity,
                i.affected_from,
                i.affected_value,
                i.message.clone(),
            )
        })
        .collect();
    out.sort();
    out
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// Every quantity a billing period reports, including the instant the peak
    /// was reached in.
    #[test]
    fn aggregate_is_order_independent(samples in arb_series()) {
        let (ordered, shuffled) = both_orders(&samples);
        for cfg in [
            AggregationConfig::rlm(),
            AggregationConfig::arbeitsmenge_only(),
            AggregationConfig::rlm().over_period(BASE, BASE + Duration::days(4)),
        ] {
            prop_assert_eq!(aggregate(&ordered, &cfg), aggregate(&shuffled, &cfg));
        }
    }

    /// Bucket totals, peaks, counts **and quality** — the field a tie in the
    /// severity ranks would make depend on the slice order.
    #[test]
    fn resample_is_order_independent(samples in arb_series()) {
        let (ordered, shuffled) = both_orders(&samples);
        for cfg in [
            ResampleConfig::to_hourly(),
            ResampleConfig::to_daily(),
            ResampleConfig::to_daily().on(DayBoundary::Gastag),
            ResampleConfig::to_monthly(),
        ] {
            let a = resample(&ordered, &cfg);
            let b = resample(&shuffled, &cfg);
            prop_assert_eq!(a.len(), b.len());
            for (x, y) in a.iter().zip(&b) {
                prop_assert_eq!(x.from, y.from);
                prop_assert_eq!(x.to, y.to);
                prop_assert_eq!(x.total, y.total);
                prop_assert_eq!(x.peak_kw, y.peak_kw);
                prop_assert_eq!(x.interval_count, y.interval_count);
                prop_assert_eq!(x.expected_count, y.expected_count);
                prop_assert_eq!(x.quality, y.quality, "bucket quality at {}", x.from);
            }
        }
    }

    /// Every rule but V11 sees the same series, so it reaches the same
    /// findings — and V11 fires exactly when the input was out of order.
    #[test]
    fn validation_findings_are_order_independent(samples in arb_series()) {
        let (ordered, shuffled) = both_orders(&samples);
        let cfg = ValidationConfig::default()
            .over_period(BASE, BASE + Duration::days(4))
            .at_reference_instant(BASE + Duration::days(2));

        let a = validate_intervals(&ordered, &cfg);
        let b = validate_intervals(&shuffled, &cfg);
        prop_assert_eq!(findings(&a), findings(&b));
        prop_assert_eq!(a.evaluated, b.evaluated);

        prop_assert!(
            a.by_rule(ValidationRuleId::UnorderedSeries).next().is_none(),
            "an ascending series is not unordered",
        );
        prop_assert_eq!(
            b.by_rule(ValidationRuleId::UnorderedSeries).count() > 0,
            ordered != shuffled,
            "V11 fires exactly when the slice was permuted",
        );
    }

    /// Every quantity the grade is built from is order-independent, and the
    /// grade itself moves in exactly one way: V11.
    ///
    /// A shuffled series *is* different — the disorder is a finding, and a
    /// series that arrives out of order usually means a broken merge upstream.
    /// So an otherwise clean series grades `A` in order and `B` shuffled, and
    /// that is the intended behaviour rather than a leak. V11 is a Warning, so
    /// it can only ever cost the `A`; it can never reach `C`, which needs a
    /// blocking finding. Everything else must match exactly.
    #[test]
    fn grading_moves_only_by_the_disorder_it_reports(samples in arb_series()) {
        let (ordered, shuffled) = both_orders(&samples);
        for cfg in [
            QualityConfig::default(),
            QualityConfig::for_sparte(metering::Sparte::Gas),
            QualityConfig::default().over_period(BASE, BASE + Duration::days(4)),
        ] {
            let a = score_intervals(&ordered, &cfg);
            let b = score_intervals(&shuffled, &cfg);

            prop_assert_eq!(a.coverage_pct, b.coverage_pct);
            prop_assert_eq!(a.max_zero_run, b.max_zero_run);
            prop_assert_eq!(a.gaps_detected, b.gaps_detected);
            prop_assert_eq!(a.outliers_detected, b.outliers_detected);
            prop_assert_eq!(a.blocking_findings, b.blocking_findings);
            prop_assert_eq!(a.intervals_consistent, b.intervals_consistent);
            prop_assert_eq!(a.evaluated, b.evaluated);
            prop_assert_eq!(findings(&a), findings(&b), "every non-V11 finding");

            let disorder = |r: &metering::QualityReport| {
                r.issues
                    .iter()
                    .filter(|i| i.rule_id == ValidationRuleId::UnorderedSeries)
                    .count()
            };
            prop_assert_eq!(disorder(&a), 0, "an ascending series is not unordered");

            prop_assert!(b.grade >= a.grade, "shuffling can only cost the A");
            if a.grade != b.grade {
                prop_assert_eq!(a.grade, metering::QualityGrade::A);
                prop_assert_eq!(b.grade, metering::QualityGrade::B);
                prop_assert_eq!(disorder(&b), 1, "and only because of V11");
            }
        }
    }

    /// The filled series, and the audit trail that explains it.
    #[test]
    fn gap_filling_is_order_independent(samples in arb_series()) {
        let (ordered, shuffled) = both_orders(&samples);
        let cfg = FillGapsConfig::new(
            IntervalResolution::QuarterHour,
            BASE,
            BASE + Duration::days(1),
        );
        prop_assert_eq!(fill_gaps(&ordered, &cfg), fill_gaps(&shuffled, &cfg));
    }

    /// Per-register sums, including the `None` bucket for unassigned energy.
    #[test]
    fn the_register_split_is_order_independent(samples in arb_series()) {
        let (ordered, shuffled) = both_orders(&samples);
        let zzd = Zaehlzeitdefinition::modul_3(
            "NB-14A-3",
            date!(2026 - 01 - 01),
            (17 * 60, 20 * 60),
            (22 * 60, 6 * 60),
        );
        prop_assert_eq!(zzd.split_energy(&ordered), zzd.split_energy(&shuffled));
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

        let cfg = LastgangConfig::strom().with_register_digits(6);
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
