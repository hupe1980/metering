//! Property tests for validation and substitution on a `Series`.

use metering::time::holiday::Bundesland;
use metering::vee::substitute::{Policy, SubstitutionReason, substitute};
use metering::vee::validation::{Grade, Rule, Rules, Severity, validate};
use metering::{DayBoundary, MeterInterval, Period, QualityFlag, Resolution, Series};
use proptest::prelude::*;
use rust_decimal::Decimal;
use time::macros::{date, datetime};
use time::{Duration, OffsetDateTime};

fn day() -> Period {
    DayBoundary::Strom.day(date!(2026 - 06 - 03)).unwrap()
}

const NOW: OffsetDateTime = datetime!(2027-01-01 0:00 UTC);

fn arb_kwh() -> impl Strategy<Value = Decimal> {
    (0i64..100_000).prop_map(|n| Decimal::new(n, 3))
}

/// A day of quarter-hours; `None` is a missing slot.
fn series(values: &[Option<Decimal>], quality: QualityFlag) -> Series {
    let ivs = values
        .iter()
        .enumerate()
        .filter_map(|(i, v)| {
            v.map(|v| {
                MeterInterval::quarter_hour(
                    day().start() + Duration::minutes(15 * i as i64),
                    v,
                    quality,
                )
                .unwrap()
            })
        })
        .collect();
    Series::new(Resolution::QUARTER_HOUR, DayBoundary::Strom, ivs).unwrap()
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(200))]

    /// The grade is a function of the findings and nothing else.
    #[test]
    fn the_grade_is_the_worst_severity(values in prop::collection::vec(prop::option::weighted(0.9, arb_kwh()), 96)) {
        let report = validate(&series(&values, QualityFlag::Measured), &Rules::strom(day(), NOW, None));
        let expected = match report.findings.iter().map(|f| f.severity()).max() {
            None => Grade::A,
            Some(Severity::Warning) => Grade::B,
            Some(Severity::Error) => Grade::C,
            Some(_) => Grade::F,
        };
        prop_assert_eq!(report.grade(), expected);
        let missing = values.iter().filter(|v| v.is_none()).count();
        prop_assert_eq!(missing == 0, report.coverage.is_complete());
        prop_assert_eq!(missing == 0, report.by_rule(Rule::Gap).next().is_none());
    }

    /// Every slot of a faulty series is covered by a non-billable error.
    #[test]
    fn faulty_values_are_errors(n in 1usize..=96) {
        let values: Vec<_> = (0..96).map(|i| (i < n).then_some(Decimal::ONE)).collect();
        let report = validate(&series(&values, QualityFlag::Faulty), &Rules::strom(day(), NOW, None));
        let covered: Duration = report.by_rule(Rule::NonBillable).map(|f| f.to - f.from).sum();
        prop_assert_eq!(covered, Duration::minutes(15 * n as i64));
        prop_assert!(report.grade() >= Grade::C);
    }

    /// A register-anchored fill conserves energy exactly: the filled series
    /// sums to the register advance to the last decimal place.
    #[test]
    fn a_register_anchored_fill_conserves_energy(
        values in prop::collection::vec(arb_kwh(), 96),
        gap_start in 1usize..80,
        gap_len in 1usize..15,
        extra in 0i64..50_000,
    ) {
        let mut slots: Vec<Option<Decimal>> = values.into_iter().map(Some).collect();
        for slot in slots.iter_mut().skip(gap_start).take(gap_len) {
            *slot = None;
        }
        let s = series(&slots, QualityFlag::Measured);
        let actual = s.billable_total().unwrap();
        let advance = actual + Decimal::new(extra, 3);
        let start = metering::series::reading::MeterReading::measured(day().start(), Decimal::new(500, 0));
        let end = metering::series::reading::MeterReading::measured(day().end(), Decimal::new(500, 0) + advance);
        let rules = Rules::strom(day(), NOW, None);
        let report = validate(&s, &rules);
        let policy = Policy::strom(Bundesland::Be, SubstitutionReason::CommunicationFailure)
            .short(Duration::hours(24))
            .anchor(start, end)
            .unwrap();
        let filled = substitute(&s, &report, &policy).unwrap();
        prop_assert!(filled.refusals.is_empty(), "{:?}", filled.refusals);
        prop_assert_eq!(filled.series.billable_total(), Some(advance));
    }
}
