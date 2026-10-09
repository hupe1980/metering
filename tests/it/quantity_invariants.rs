//! The identities and bounds every calculation here must satisfy, under
//! generated input.
//!
//! The crate's example-based tests are written by a human choosing numbers, and
//! a human picks round, distinct ones because they are easy to reason about —
//! which is precisely the set where ties do not occur and divisions come out
//! exact. Two order-dependence defects and one false exactness claim survived
//! years of such tests for that reason alone.
//!
//! So this file states the invariant and lets `proptest` choose the numbers.
//! Every property below is a *conservation law* — energy in equals energy out,
//! a split reconstructs its total, a mean lies between its extremes — or a
//! bound the API's own documentation promises.

use metering::allocation::session::{
    MeterSample, SessionSplitConfig, merge_sessions, split_session,
};
use metering::time::holiday::Bundesland;
use metering::vee::substitute::{Method, Policy, SubstitutionReason, substitute};
use metering::vee::validation::{Rules, Severity, validate};
use metering::{DayBoundary, Period, Series};
use metering::{
    MeterInterval, QualityFlag, Resolution, Unit, billing::aggregation::aggregate,
    billing::aggregation::sum_by_direction, billing::forecast::project_annual_consumption,
    billing::imbalance::ImbalanceSaldo, billing::losses::NetworkLosses,
    billing::zaehlzeit::Zaehlzeitdefinition, gas::conversion::G685FinalRounding,
    gas::conversion::G685Rounding, gas::conversion::GasConversionParams,
    gas::conversion::ZustandszahlParams, gas::conversion::gas_m3_to_kwh_hs,
    gas::conversion::gas_m3_to_kwh_hs_rounded, gas::conversion::hoehenzonen_luftdruck_mbar,
    gas::conversion::normalize_to_kwh, gas::conversion::zustandszahl, series::resample::resample,
};
use proptest::prelude::*;
use rust_decimal::Decimal;
use time::macros::{date, datetime};
use time::{Duration, OffsetDateTime};

/// A Berlin midnight, so daily and monthly grids line up with the generators.
const BASE: OffsetDateTime = datetime!(2025-12-31 23:00 UTC);

/// kWh in an interval: never negative, three decimals, and drawn from a coarse
/// grid half the time so ties and exact divisions both occur.
fn arb_kwh() -> impl Strategy<Value = Decimal> {
    prop_oneof![
        (0i64..40).prop_map(|n| Decimal::new(n * 500, 3)),
        (0i64..40_000).prop_map(|milli| Decimal::new(milli, 3)),
    ]
}

/// A quantity that may be negative — a Korrekturenergiemenge, a residual load.
fn arb_signed_kwh() -> impl Strategy<Value = Decimal> {
    (-40_000i64..40_000).prop_map(|milli| Decimal::new(milli, 3))
}

fn arb_quality() -> impl Strategy<Value = QualityFlag> {
    (0usize..QualityFlag::ALL.len()).prop_map(|i| QualityFlag::ALL[i])
}

/// A contiguous quarter-hour series from [`BASE`].
fn arb_series(len: std::ops::Range<usize>) -> impl Strategy<Value = Vec<MeterInterval>> {
    prop::collection::vec((arb_kwh(), arb_quality()), len).prop_map(|rows| {
        rows.into_iter()
            .enumerate()
            .map(|(i, (value, quality))| {
                let from = BASE + Duration::minutes(15 * i as i64);
                MeterInterval::new(from, from + Duration::minutes(15), value, quality).unwrap()
            })
            .collect()
    })
}

/// A series with holes: distinct slots, ascending, not necessarily contiguous.
fn arb_sparse_series() -> impl Strategy<Value = Vec<MeterInterval>> {
    prop::collection::vec((0i64..96, arb_kwh(), arb_quality()), 0..40).prop_map(|mut rows| {
        rows.sort_by_key(|(slot, _, _)| *slot);
        rows.dedup_by_key(|(slot, _, _)| *slot);
        rows.into_iter()
            .map(|(slot, value, quality)| {
                let from = BASE + Duration::minutes(15 * slot);
                MeterInterval::new(from, from + Duration::minutes(15), value, quality).unwrap()
            })
            .collect()
    })
}

/// The Berlin day starting at [`BASE`].
fn base_day() -> Period {
    DayBoundary::Strom.day(date!(2026 - 01 - 01)).unwrap()
}

fn qh(intervals: Vec<MeterInterval>) -> Series {
    Series::new(Resolution::QUARTER_HOUR, DayBoundary::Strom, intervals).unwrap()
}

// ── substitute ───────────────────────────────────────────────────────────────

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// Every slot of the period ends up filled or inside a refusal; a value
    /// no error finding covers is never rewritten; each substitute is in the
    /// series with its own value and rests on the evidence its method needs.
    #[test]
    fn every_slot_is_filled_or_refused_and_accounted_for(
        measured in arb_sparse_series(),
        short_hours in 0i64..6,
    ) {
        let series = qh(measured);
        let rules = Rules::strom(base_day(), datetime!(2027-01-01 0:00 UTC), None);
        let report = validate(&series, &rules);
        let policy = Policy::strom(Bundesland::Be, SubstitutionReason::CommunicationFailure)
            .short(Duration::hours(short_hours));
        let filled = substitute(&series, &report, &policy).unwrap();

        let rejected = |at: OffsetDateTime| {
            report.at_least(Severity::Error).any(|f| f.from <= at && at < f.to)
        };
        for i in 0..96 {
            let at = BASE + Duration::minutes(15 * i);
            let refused = filled.refusals.iter().any(|r| r.from <= at && at < r.to);
            prop_assert!(filled.series.get(at).is_some() || refused, "{at}");
            if let Some(iv) = series.get(at).filter(|_| !rejected(at)) {
                prop_assert_eq!(filled.series.get(at), Some(iv));
            }
        }
        for s in &filled.substitutes {
            let out = filled.series.get(s.from).expect("in the series");
            prop_assert_eq!((out.value(), out.quality()), (s.value, QualityFlag::Substituted));
            let refs = s.references.len();
            match s.method {
                Method::Interpolation => prop_assert_eq!(refs, 2),
                Method::Vergleichswert => prop_assert!((1..=3).contains(&refs)),
                Method::Hold => prop_assert_eq!(refs, 1),
                _ => prop_assert_eq!(refs, 0),
            }
            for at in &s.references {
                let r = series.get(*at).expect("a reference is a delivered value");
                prop_assert!(r.quality() == QualityFlag::Measured || r.quality() == QualityFlag::Corrected);
            }
        }
    }

    /// An interpolated value lies between the two values it was interpolated
    /// from — a straight line does not leave its own endpoints.
    #[test]
    fn an_interpolated_value_stays_between_its_anchors(
        measured in arb_sparse_series(),
    ) {
        let series = qh(measured);
        let report = validate(&series, &Rules::strom(base_day(), datetime!(2027-01-01 0:00 UTC), None));
        let policy = Policy::strom(Bundesland::Be, SubstitutionReason::CommunicationFailure)
            .short(Duration::hours(24));
        let filled = substitute(&series, &report, &policy).unwrap();
        for s in filled.substitutes.iter().filter(|s| s.method == Method::Interpolation) {
            let a = series.get(s.references[0]).unwrap().value();
            let b = series.get(s.references[1]).unwrap().value();
            prop_assert!(s.value >= a.min(b) && s.value <= a.max(b), "{} outside [{a}, {b}]", s.value);
        }
    }
}

// ── resample ─────────────────────────────────────────────────────────────────

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// Every bucket of the coarse series is exactly the sum of the source
    /// slots it contains, and a bucket is emitted only when all are present.
    #[test]
    fn resampling_conserves_energy_per_bucket(series in arb_series(0..200)) {
        let n = series.len();
        let series = qh(series);
        for target in [Resolution::Hour, Resolution::Day, Resolution::Month, Resolution::Year] {
            let coarse = resample(&series, target).unwrap();
            for b in &coarse {
                let inside: Vec<_> = series.iter().filter(|iv| b.from() <= iv.from() && iv.to() <= b.to()).collect();
                prop_assert_eq!(b.value(), inside.iter().map(|iv| iv.value()).sum::<Decimal>());
                prop_assert_eq!(b.quality(), QualityFlag::worst_of(inside.iter().map(|iv| iv.quality())));
            }
            let expected = match target {
                Resolution::Hour => n / 4,
                Resolution::Day => usize::from(n >= 96) + usize::from(n >= 192),
                _ => 0,
            };
            prop_assert_eq!(coarse.len(), expected, "{}", target);
        }
    }
}

// ── zaehlzeit ────────────────────────────────────────────────────────────────

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// A register split reconstructs the Arbeitsmenge exactly — registers,
    /// straddling and unassigned together — and books into nothing the
    /// definition does not declare.
    #[test]
    fn the_register_split_reconstructs_the_arbeitsmenge(series in arb_series(0..96)) {
        let zzd = Zaehlzeitdefinition::modul_3(
            "NB-14A-3",
            date!(2026 - 01 - 01),
            (17 * 60, 20 * 60),
            (22 * 60, 6 * 60),
            &metering::billing::zaehlzeit::Quarter::ALL,
        )
        .unwrap();
        let split = zzd.split_energy(&series).unwrap();
        let period = aggregate(&qh(series.clone()), base_day()).unwrap();

        let rest: Decimal = split
            .straddling
            .iter()
            .chain(&split.unassigned)
            .map(MeterInterval::value)
            .sum();
        prop_assert_eq!(split.per_register.values().sum::<Decimal>() + rest, period.arbeitsmenge);
        prop_assert_eq!(
            split.non_billable.len(),
            series.iter().filter(|iv| !iv.quality().is_billable()).count()
        );
        let declared = zzd.registers();
        for id in split.per_register.keys() {
            prop_assert!(declared.contains(id), "undeclared register {id}");
        }
    }
}

// ── forecast ─────────────────────────────────────────────────────────────────

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// A projection is its own stated formula — or a refusal with a reason.
    #[test]
    fn a_projection_is_its_own_formula_or_a_reason(series in arb_series(672..1000)) {
        use metering::billing::forecast::ForecastError;
        let series = metering::Series::new(Resolution::QUARTER_HOUR, metering::DayBoundary::Strom, series).unwrap();
        let f = match project_annual_consumption(&series, None) {
            Ok(f) => f,
            Err(ForecastError::Coverage { billable, expected }) => {
                prop_assert!(u64::from(billable) * 100 < u64::from(expected) * 90);
                return Ok(());
            }
            Err(ForecastError::TooShort { days }) => {
                prop_assert!(days < 7);
                return Ok(());
            }
            Err(ForecastError::Empty) => return Ok(()),
            Err(other) => return Err(TestCaseError::fail(format!("{other}"))),
        };
        prop_assert_eq!(
            f.observed_kwh,
            series.iter().filter(|iv| iv.quality().is_billable()).map(|iv| iv.value()).sum::<Decimal>(),
        );
        prop_assert_eq!(f.seasonal_factor, None);
        prop_assert_eq!(
            f.daily_average_kwh,
            (f.observed_kwh * Decimal::from(f.expected_slots)
                / Decimal::from(f.observed_days * f.billable_slots))
                .round_dp_with_strategy(metering::precision::FORECAST_DP, metering::precision::FORECAST_STRATEGY),
        );
        prop_assert_eq!(
            f.projected_annual_kwh,
            (f.daily_average_kwh * Decimal::from(f.target_year_days))
                .round_dp_with_strategy(metering::precision::FORECAST_DP, metering::precision::FORECAST_STRATEGY),
        );
    }

    /// WAPE of a series against itself is zero, and against a uniformly
    /// scaled copy is the scale's distance from one.
    #[test]
    fn wape_is_zero_on_itself_and_the_scale_error_on_a_copy(
        series in arb_series(96..200),
        percent in 1i64..200,
    ) {
        use metering::billing::forecast::{AccuracyError, wape};
        let actual = metering::Series::new(Resolution::QUARTER_HOUR, metering::DayBoundary::Strom, series).unwrap();
        let k = Decimal::new(percent, 2);
        let scaled = metering::Series::new(
            Resolution::QUARTER_HOUR,
            metering::DayBoundary::Strom,
            actual.iter().map(|iv| iv.clone().with_value(iv.value() * k, iv.quality())).collect(),
        )
        .unwrap();
        match wape(&actual, &actual) {
            Ok(w) => {
                prop_assert_eq!(w, Decimal::ZERO);
                prop_assert_eq!(
                    wape(&actual, &scaled).unwrap(),
                    (Decimal::ONE - k).abs().round_dp_with_strategy(
                        metering::precision::FORECAST_ACCURACY_DP,
                        metering::precision::FORECAST_ACCURACY_STRATEGY,
                    ),
                );
            }
            Err(e) => prop_assert!(matches!(e, AccuracyError::ZeroActual | AccuracyError::NoOverlap)),
        }
    }
}

// ── reactive energy and utilisation hours ────────────────────────────────────

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// The Blindarbeit balance splits the meter's own kvarh and invents none.
    ///
    /// `Freigrenze + Blindmehrarbeit` equals the Blindarbeit whenever there is
    /// an excess, and exceeds it exactly by the unused headroom when there is
    /// not — so no kvarh appears or disappears between the register and the
    /// bill.
    #[test]
    fn the_reactive_balance_conserves_the_register(
        kwh in 0i64..2_000_000,
        kvarh in 0i64..2_000_000,
        ratio_millis in 0i64..2_000,
    ) {
        let wirk = Decimal::from(kwh);
        let blind = Decimal::from(kvarh);
        let limit = metering::billing::reactive::ReactiveLimit::new(Decimal::new(ratio_millis, 3)).unwrap();
        let b = metering::billing::reactive::ReactiveBalance::new(wirk, blind, limit).unwrap();

        prop_assert_eq!(b.freigrenze_kvarh(), limit.ratio() * wirk);
        prop_assert!(b.blindmehrarbeit_kvarh() >= Decimal::ZERO);
        prop_assert!(b.headroom_kvarh() >= Decimal::ZERO);
        // Exactly one of the two is non-zero, and together they close the gap.
        prop_assert_eq!(
            b.freigrenze_kvarh() + b.blindmehrarbeit_kvarh() - b.headroom_kvarh(),
            b.blindarbeit_kvarh()
        );
        prop_assert_eq!(b.is_chargeable(), blind > b.freigrenze_kvarh());
    }

    /// A stricter ratio never charges less.
    #[test]
    fn a_smaller_freigrenze_never_charges_less(
        kwh in 1i64..1_000_000,
        kvarh in 0i64..1_000_000,
    ) {
        let wirk = Decimal::from(kwh);
        let blind = Decimal::from(kvarh);
        use metering::billing::reactive::{ReactiveBalance, ReactiveLimit};
        let loose = ReactiveBalance::new(wirk, blind, ReactiveLimit::HALF).unwrap();
        let strict = ReactiveBalance::new(wirk, blind, ReactiveLimit::COS_PHI_0_9).unwrap();
        prop_assert!(strict.blindmehrarbeit_kvarh() >= loose.blindmehrarbeit_kvarh());
    }

    /// The Benutzungsstundenzahl is bounded by the period it is measured over:
    /// a load that never exceeds its own peak cannot run more hours than the
    /// period holds, and a flat load runs exactly all of them.
    #[test]
    fn the_utilisation_hours_are_bounded_by_the_period(
        values in prop::collection::vec(1i64..4_000, 96..=96),
    ) {
        let period = DayBoundary::Strom.day(date!(2026 - 06 - 01)).unwrap();
        let base = period.start();
        let day: Vec<MeterInterval> = values
            .iter()
            .enumerate()
            .map(|(i, v)| MeterInterval::new(base + Duration::minutes(15 * i as i64), base + Duration::minutes(15 * i as i64 + 15), Decimal::new(*v, 2), QualityFlag::Measured).unwrap())
            .collect();

        let billed = aggregate(&qh(day.clone()), period).unwrap();
        let hours = billed.benutzungsdauer_h().expect("a positive peak");
        prop_assert!(hours > Decimal::ZERO);
        prop_assert!(hours <= Decimal::from(24u32), "{hours} h in a 24 h day");

        // A flat day uses every hour of itself.
        let flat: Vec<MeterInterval> = day
            .iter()
            .map(|iv| iv.clone().with_value(Decimal::ONE, iv.quality()))
            .collect();
        prop_assert_eq!(
            aggregate(&qh(flat), period).unwrap().benutzungsdauer_h(),
            Some(Decimal::from(24u32))
        );
    }
}

// ── conversion ───────────────────────────────────────────────────────────────

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// The gas conversion is a product, and nothing rounds it.
    #[test]
    fn the_gas_conversion_is_an_exact_product(
        m3 in arb_kwh(),
        hs in (8_000i64..13_000).prop_map(|n| Decimal::new(n, 3)),
        z in (900i64..1_100).prop_map(|n| Decimal::new(n, 3)),
    ) {
        prop_assert_eq!(gas_m3_to_kwh_hs(m3, hs, z), Some(m3 * hs * z));

        // Through the unit normaliser, the same number.
        let params = GasConversionParams::new(hs, z);
        prop_assert_eq!(
            normalize_to_kwh(m3, "m3", Some(&params), None).unwrap(),
            m3 * hs * z,
        );
        // …and an already-converted volume applies no Zustandszahl.
        let norm = GasConversionParams::already_converted(hs);
        prop_assert_eq!(
            normalize_to_kwh(m3, "m3", Some(&norm), None).unwrap(),
            m3 * hs,
        );
    }

    /// The Zustandszahl moves the way the gas law says, in all three arguments.
    ///
    /// It rises with absolute pressure and falls with temperature and with the
    /// K-Zahl. A sign or a reciprocal in the wrong place still produces a
    /// plausible number near 1 — the ratios are all close to unity — so the
    /// direction is what a test has to hold, on every input rather than on one.
    #[test]
    fn the_zustandszahl_follows_the_gas_law(
        hoehe in (0i64..2_000).prop_map(Decimal::from),
        p_eff in (0i64..900).prop_map(Decimal::from),
        t_c in (-20i64..60).prop_map(Decimal::from),
        k in (800i64..1_200).prop_map(|n| Decimal::new(n, 3)),
        step in (1i64..500).prop_map(Decimal::from),
    ) {
        let luftdruck = hoehenzonen_luftdruck_mbar(hoehe).unwrap();
        let at = |p_eff, t_c, k| {
            zustandszahl(&ZustandszahlParams::new(luftdruck, p_eff, t_c, k))
                .expect("a positive gas state")
        };
        let base = at(p_eff, t_c, k);
        prop_assert!(base > Decimal::ZERO, "a Zustandszahl is a positive factor");

        // More absolute pressure packs more gas into the same volume.
        prop_assert!(at(p_eff + step, t_c, k) > base);
        // Warmer gas is thinner.
        prop_assert!(at(p_eff, t_c + Decimal::ONE, k) < base);
        // A larger K-Zahl divides a larger denominator.
        prop_assert!(at(p_eff, t_c, k + Decimal::new(1, 3)) < base);

        // A Höhenzone is a straight line in the height, with no rounding.
        prop_assert_eq!(
            hoehenzonen_luftdruck_mbar(hoehe + step),
            Some(luftdruck - Decimal::new(12, 2) * step),
        );
    }

    /// Rounding the inputs and then the result never moves the answer by more
    /// than the rounding it was asked for.
    #[test]
    fn g685_rounding_stays_within_its_own_granularity(
        m3 in (1i64..100_000).prop_map(|n| Decimal::new(n, 2)),
        hs in (8_000i64..13_000).prop_map(|n| Decimal::new(n, 3)),
        z in (9_000i64..11_000).prop_map(|n| Decimal::new(n, 4)),
    ) {
        let exact = gas_m3_to_kwh_hs(m3, hs, z).unwrap();
        // The inputs are already at the configured precision, so only the final
        // rounding can move anything.
        let unrounded = gas_m3_to_kwh_hs_rounded(m3, hs, z, G685Rounding::PUBLISHED_PRACTICE).unwrap();
        prop_assert_eq!(unrounded, exact, "the default rounds nothing");

        for (mode, granularity) in [
            (G685FinalRounding::WholeKwh, Decimal::new(5, 1)),
            (G685FinalRounding::TwoDecimals, Decimal::new(5, 3)),
        ] {
            let rounded = gas_m3_to_kwh_hs_rounded(
                m3, hs, z,
                G685Rounding::PUBLISHED_PRACTICE.final_rounding(mode),
            )
            .unwrap();
            prop_assert!(
                (rounded - exact).abs() <= granularity,
                "{mode:?}: {rounded} vs {exact}",
            );
        }
    }

    /// A **decimal-power** unit converts exactly: the quotient terminates, so
    /// `apply(v) × den` recovers `v × num` digit for digit and doubling the
    /// input doubles the result.
    #[test]
    fn a_decimal_power_unit_converts_exactly(
        value in (0i64..1_000_000).prop_map(|n| Decimal::new(n, 3)),
        unit in prop::sample::select(vec!["kWh", "Wh", "MWh", "GWh"]),
    ) {
        let scale = Unit::parse_scaled(unit).expect("an accepted unit");
        prop_assert_eq!(scale.unit(), Unit::KiloWattHour);
        let (num, den) = scale.factor();

        let converted = normalize_to_kwh(value, unit, None, None).unwrap();
        prop_assert_eq!(Some(converted), scale.apply(value), "one path, one answer");
        prop_assert_eq!(
            converted * Decimal::from(den),
            value * Decimal::from(num),
        );
        let doubled = normalize_to_kwh(value * Decimal::TWO, unit, None, None).unwrap();
        prop_assert_eq!(doubled, converted * Decimal::TWO);
    }

    /// A **rational** unit — GJ is 2500/9, a joule is 1/3 600 000 — rounds
    /// once, at the end, and no more than once.
    ///
    /// Multiplying before dividing is what buys that. Storing the factor as a
    /// `Decimal` (`277.777…8` for GJ) would round when the factor was written
    /// down *and* again per reading, and the error would be systematic. The
    /// bound below is one unit in `Decimal`'s last place scaled by the
    /// denominator — a single rounding, not a drift.
    #[test]
    fn a_rational_unit_rounds_once(
        value in (0i64..1_000_000).prop_map(|n| Decimal::new(n, 3)),
        unit in prop::sample::select(vec!["GJ", "MJ", "JOU", "KJO"]),
    ) {
        let scale = Unit::parse_scaled(unit).expect("an accepted unit");
        let converted = normalize_to_kwh(value, unit, None, None).unwrap();
        prop_assert_eq!(Some(converted), scale.apply(value), "one path, one answer");
        let (num, den) = scale.factor();

        let residue = converted * Decimal::from(den) - value * Decimal::from(num);
        prop_assert!(
            residue.abs() < Decimal::new(1, 15),
            "{unit}: {value} → {converted}, residue {residue}",
        );
    }

    /// The identities the rationals are *chosen* to satisfy hold to the digit,
    /// which is the whole reason the factor is a fraction and not a decimal.
    #[test]
    fn the_defining_identities_are_exact(k in 1i64..10_000) {
        let k = Decimal::from(k);
        // 3.6 GJ ≡ 1 000 kWh, 18 MJ ≡ 5 kWh, 3.6 × 10⁶ J ≡ 1 kWh.
        prop_assert_eq!(
            normalize_to_kwh(Decimal::new(36, 1) * k, "GJ", None, None).unwrap(),
            Decimal::from(1000u32) * k,
        );
        prop_assert_eq!(
            normalize_to_kwh(Decimal::from(18u32) * k, "MJ", None, None).unwrap(),
            Decimal::from(5u32) * k,
        );
        prop_assert_eq!(
            normalize_to_kwh(Decimal::from(3_600_000u32) * k, "JOU", None, None).unwrap(),
            k,
        );
    }

    /// A power integrated over its own interval is an energy.
    #[test]
    fn a_power_becomes_an_energy_over_its_interval(
        kw in (0i64..100_000).prop_map(|n| Decimal::new(n, 3)),
        secs in prop::sample::select(vec![900i64, 1800, 3600, 86_400]),
    ) {
        let kwh = normalize_to_kwh(kw, "kW", None, Some(secs)).unwrap();
        prop_assert_eq!(kwh * Decimal::from(3600u32), kw * Decimal::from(secs));
    }
}

// ── imbalance and losses ─────────────────────────────────────────────────────

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// Mehr and Minder are the two halves of one signed difference: at most one
    /// is positive, and their difference is the delta.
    #[test]
    fn an_imbalance_splits_one_signed_delta(
        actual in arb_signed_kwh(),
        bilanziert in arb_signed_kwh(),
    ) {
        let s = ImbalanceSaldo::new(actual, bilanziert).unwrap();
        prop_assert_eq!(s.delta_kwh(), actual - bilanziert);
        prop_assert_eq!(s.minder_kwh() - s.mehr_kwh(), s.delta_kwh());
        prop_assert!(s.mehr_kwh() >= Decimal::ZERO && s.minder_kwh() >= Decimal::ZERO);
        prop_assert!(!(s.is_mehr() && s.is_minder()), "both sides cannot be open");
        prop_assert_eq!(s.is_balanced(), s.delta_kwh().is_zero());
        prop_assert_eq!(s.delta_pct().is_some(), !bilanziert.is_zero());
    }

    /// The loss balance is a difference, and the share is that difference over
    /// the infeed.
    #[test]
    fn a_loss_balance_is_a_difference(
        einspeisung in arb_kwh(),
        entnahme in arb_kwh(),
    ) {
        let l = NetworkLosses::new(einspeisung, entnahme).unwrap();
        prop_assert_eq!(l.verlust_kwh(), einspeisung - entnahme);
        prop_assert_eq!(l.verlust_prozent().is_some(), einspeisung > Decimal::ZERO);
        if let Some(pct) = l.verlust_prozent() {
            prop_assert_eq!(pct.is_sign_negative(), l.verlust_kwh().is_sign_negative());
        }
    }
}

// ── gas SLP ──────────────────────────────────────────────────────────────────

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// The Allokationstemperatur is a weighted **mean**, so it never leaves the
    /// range of the four daily means it averages, and reduces to the value
    /// itself when they agree.
    #[test]
    fn the_allocation_temperature_is_a_mean(
        temps in prop::array::uniform4((-30_000i64..40_000).prop_map(|n| Decimal::new(n, 3))),
    ) {
        use metering::slp::gas::allocation_temperature;
        let [a, b, c, d] = temps;
        let theta = allocation_temperature(a, b, c, d).unwrap();
        let lo = a.min(b).min(c).min(d);
        let hi = a.max(b).max(c).max(d);
        prop_assert!(theta >= lo && theta <= hi, "{theta} outside [{lo}, {hi}]");
        // The four-decimal weights sum to exactly 1.
        prop_assert_eq!(allocation_temperature(a, a, a, a), Some(a), "a constant week");
    }

    /// `Q = KW · h · F_WT` is an exact product, and the Kundenwert inverts it.
    #[test]
    fn the_daily_quantity_and_the_kundenwert_are_inverses(
        kw in (1i64..1_000_000).prop_map(|n| Decimal::new(n, 4)),
        h in (1i64..5_000_000).prop_map(|n| Decimal::new(n, 6)),
    ) {
        use metering::{slp::gas::gas_daily_quantity, slp::gas::kundenwert};

        let q = gas_daily_quantity(kw, h, Decimal::ONE).unwrap();
        prop_assert_eq!(q, kw * h);

        // Recovering the Kundenwert from the quantity and the profile sum
        // returns the same figure, to the Leitfaden's four places.
        let recovered = kundenwert(q, h).expect("a positive divisor");
        prop_assert_eq!(
            recovered,
            kw.round_dp_with_strategy(4, metering::precision::KUNDENWERT_STRATEGY)
        );

        prop_assert_eq!(kundenwert(q, Decimal::ZERO), None);
        prop_assert_eq!(kundenwert(q, -h), None);
    }

    /// The profile function is a consumption share: never negative, and finite
    /// across the whole temperature range a German winter and summer produce —
    /// and undefined, not a substitute value, at and above `ϑ₀ = 40 °C`.
    #[test]
    fn the_profile_function_is_a_non_negative_share(
        theta in (-30_000i64..38_999).prop_map(|n| Decimal::new(n, 3)),
        hot in (40_000i64..60_000).prop_map(|n| Decimal::new(n, 3)),
    ) {
        use metering::slp::gas::SigLinDe;
        prop_assert_eq!(SigLinDe::DE_HEF34.h_value(hot), None);
        let h = SigLinDe::DE_HEF34.h_value(theta).unwrap();
        prop_assert!(h >= Decimal::ZERO, "h({theta}) = {h}");
        prop_assert!(h <= Decimal::from(10u32), "h({theta}) = {h} is off the scale");

        // Warmer weather never means more heating gas.
        let warmer = SigLinDe::DE_HEF34.h_value(theta + Decimal::ONE).unwrap();
        prop_assert!(warmer <= h, "h({}) = {warmer} > h({theta}) = {h}", theta + Decimal::ONE);
    }

    /// The standard week sums to 7,0000 or the factors are refused — the
    /// Leitfaden's own consistency rule, which a rescaled set would break
    /// silently.
    #[test]
    fn weekday_factors_must_sum_to_seven(
        raw in prop::array::uniform7((1i64..20_000).prop_map(|n| Decimal::new(n, 4))),
    ) {
        use metering::slp::gas::WeekdayFactors;

        let sum: Decimal = raw.iter().copied().sum();
        prop_assert_eq!(
            WeekdayFactors::new(raw).is_some(),
            sum == Decimal::from(7u32),
        );

        // A set built to sum to seven is accepted and reports what it was given.
        let mut balanced = raw;
        balanced[6] = Decimal::from(7u32) - raw[..6].iter().copied().sum::<Decimal>();
        if let Some(factors) = WeekdayFactors::new(balanced) {
            for (i, weekday) in [
                time::Weekday::Monday,
                time::Weekday::Tuesday,
                time::Weekday::Wednesday,
                time::Weekday::Thursday,
                time::Weekday::Friday,
                time::Weekday::Saturday,
                time::Weekday::Sunday,
            ]
            .into_iter()
            .enumerate()
            {
                prop_assert_eq!(factors.factor(weekday), balanced[i]);
            }
        }
    }
}

// ── reading ──────────────────────────────────────────────────────────────────

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// Differencing conserves the register: a clean Zählerstandsgang sums to
    /// the difference of its outer readings, and every pair yields either an
    /// interval or an anomaly — never both, never neither.
    #[test]
    fn differencing_conserves_the_register(
        steps in prop::collection::vec(arb_kwh(), 2..60),
        start in (0i64..900_000).prop_map(|n| Decimal::new(n, 2)),
    ) {
        use metering::series::reading::{LastgangConfig, MeterReading, to_lastgang};

        let mut register = start;
        let readings: Vec<MeterReading> = std::iter::once(start)
            .chain(steps.iter().map(|s| {
                register += *s;
                register
            }))
            .enumerate()
            .map(|(i, value)| {
                MeterReading::measured(BASE + Duration::minutes(15 * i as i64), value)
            })
            .collect();

        let out = to_lastgang(&readings, &LastgangConfig::strom()).remove(0);
        prop_assert!(out.is_clean(), "a monotone register has nothing to explain");
        prop_assert_eq!(out.intervals.len(), readings.len() - 1);
        prop_assert_eq!(
            out.total(),
            Some(readings[readings.len() - 1].value - readings[0].value),
            "the Lastgang sums to the register difference",
        );
        // Every pair is accounted for exactly once.
        prop_assert_eq!(
            out.intervals.len() + out.anomalies.len(),
            readings.len() - 1,
        );
        for pair in out.intervals.windows(2) {
            prop_assert_eq!(pair[0].to(), pair[1].from(), "derived intervals tile");
        }
        for iv in &out.intervals {
            prop_assert!(iv.value() >= Decimal::ZERO, "a forward step is non-negative");
        }
    }
}

// ── power quality ────────────────────────────────────────────────────────────

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// Every EN 50160 outcome is a partition of its own samples, judged by
    /// the integer share test, per phase and per week.
    #[test]
    fn an_en50160_outcome_partitions_its_samples(
        volts in prop::collection::vec(
            (180_000i64..280_000).prop_map(|n| Decimal::new(n, 3)),
            0..200,
        ),
    ) {
        use metering::grid::power_quality::{En50160Limits, PowerQualityInterval, assess_en50160};

        let series: Vec<PowerQualityInterval> = volts
            .iter()
            .enumerate()
            .map(|(i, v)| {
                let from = BASE + Duration::minutes(10 * i as i64);
                PowerQualityInterval {
                    voltage_l1_v: Some(*v),
                    ..PowerQualityInterval::empty(from, from + Duration::minutes(10))
                }
            })
            .collect();
        let report = assess_en50160(&series, &En50160Limits::LOW_VOLTAGE);

        let samples: u32 = report.weeks.iter().map(|w| w.phases[0].voltage_band.samples).sum();
        prop_assert_eq!(samples as usize, series.len());
        for w in &report.weeks {
            let band = &w.phases[0].voltage_band;
            let absolute = &w.phases[0].voltage_absolute;
            for o in [band, absolute] {
                prop_assert!(o.within <= o.samples);
                prop_assert_eq!(o.worst.is_some(), o.within < o.samples);
                prop_assert_eq!(
                    o.compliant(),
                    (o.samples > 0).then(|| u64::from(o.within) * 1000 >= u64::from(o.samples) * u64::from(o.required_permille)),
                );
            }
            // The ±10 % band sits inside the +10 %/−15 % absolute limits.
            prop_assert!(absolute.within >= band.within);
            prop_assert_eq!(w.phases[1].voltage_band.compliant(), None, "L2 never measured");
        }
    }

    /// The Unsymmetrieleistung is a spread: never negative, unchanged by
    /// relabelling the Außenleiter, and zero exactly when the three are equal.
    #[test]
    fn unbalance_is_a_permutation_invariant_spread(
        a in (0i64..30_000).prop_map(|n| Decimal::new(n, 3)),
        b in (0i64..30_000).prop_map(|n| Decimal::new(n, 3)),
        c in (0i64..30_000).prop_map(|n| Decimal::new(n, 3)),
    ) {
        use metering::grid::power_quality::{Phase, PhaseApparentPower};

        let build = |x, y, z| PhaseApparentPower::default()
            .plus(Phase::L1, x)
            .and_then(|p| p.plus(Phase::L2, y))
            .and_then(|p| p.plus(Phase::L3, z))
            .unwrap();

        let base = build(a, b, c).unbalance_kva();
        prop_assert!(base >= Decimal::ZERO);
        for (x, y, z) in [(a, c, b), (b, a, c), (b, c, a), (c, a, b), (c, b, a)] {
            prop_assert_eq!(build(x, y, z).unbalance_kva(), base, "phase order");
        }
        prop_assert_eq!(base.is_zero(), a == b && b == c);
        prop_assert_eq!(
            build(a, b, c).within_limit(None),
            base <= metering::grid::power_quality::UNSYMMETRIE_LIMIT_KVA,
        );
        prop_assert_eq!(
            build(a, b, c).excess_kva(None),
            (base - metering::grid::power_quality::UNSYMMETRIE_LIMIT_KVA).max(Decimal::ZERO),
        );
    }
}

// ── § 14a ────────────────────────────────────────────────────────────────────

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// The floor never drops below the single-steuVE floor, adding a steuVE
    /// never lowers it, and an Anlage that forms no steuVE changes nothing.
    #[test]
    fn the_mindestleistung_is_monotone_and_bounded_below(
        devices in prop::collection::vec(
            (
                (0usize..metering::grid::para14a::SteuVeFallgruppe::ALL.len()),
                (4_300i64..60_000).prop_map(|n| Decimal::new(n, 3)),
            ),
            1..12,
        ),
    ) {
        use metering::grid::para14a::{Anlage, Para14aConfig, SteuVeFallgruppe as F, mindestleistung_ems, steuerbare_verbrauchseinrichtungen};

        let cfg = Para14aConfig::VERMUTUNG;
        let set: Vec<Anlage> = devices
            .iter()
            .map(|(g, kw)| Anlage::new(F::ALL[*g], *kw))
            .collect();
        let floor = mindestleistung_ems(&set, &cfg).expect("every Anlage exceeds 4,2 kW");
        prop_assert!(floor >= metering::grid::para14a::MINDESTLEISTUNG_KW, "{floor}");

        // Grouping never yields more steuVE than Anlagen.
        prop_assert!(steuerbare_verbrauchseinrichtungen(&set).unwrap().len() <= set.len());

        let mut grown = set.clone();
        grown.push(Anlage::new(F::Ladepunkt, Decimal::from(11u32)));
        let bigger = mindestleistung_ems(&grown, &cfg).expect("still steuVE");
        prop_assert!(bigger > floor, "{bigger} <= {floor}");

        let mut small = set;
        small.push(Anlage::new(F::Stromspeicher, Decimal::from(3u32)));
        prop_assert_eq!(mindestleistung_ems(&small, &cfg), Some(floor));
    }

    /// The netzwirksamer Leistungsbezug is bounded by both the grid draw and
    /// the steuVE draw under either convention, and the conservative one is
    /// never the smaller.
    #[test]
    fn the_netzwirksam_share_is_bounded_by_both_sides(
        netz in arb_signed_kwh(),
        steuve in arb_kwh(),
        uebrige in arb_kwh(),
    ) {
        use metering::{grid::para14a::Verursachungsregel as R, grid::para14a::netzwirksamer_leistungsbezug};

        let floor = netz.max(Decimal::ZERO);
        let conservative = netzwirksamer_leistungsbezug(netz, steuve, None, R::SteuVeZuletzt)
            .expect("needs no other figure");
        let pro_rata = netzwirksamer_leistungsbezug(netz, steuve, Some(uebrige), R::Anteilig)
            .expect("given the rest of the installation");

        for share in [conservative, pro_rata] {
            prop_assert!(share >= Decimal::ZERO);
            prop_assert!(share <= steuve, "cannot cause more than it drew");
            prop_assert!(share <= floor, "cannot cause more than left the grid");
        }
        prop_assert!(
            conservative >= pro_rata,
            "the conservative convention must never understate: {conservative} < {pro_rata}",
        );
        // Pro rata refuses to guess the rest of the installation.
        prop_assert_eq!(
            netzwirksamer_leistungsbezug(netz, steuve, None, R::Anteilig),
            None,
        );
    }
}

// ── sharing ──────────────────────────────────────────────────────────────────

/// The § 42c decision table is total and self-consistent: every input
/// reaches a verdict or a named contradiction, a qualified point names its
/// statutory limb, and a disqualified one always says why.
///
/// The table is finite — each of the four inputs is a known value or unknown —
/// so it is enumerated, every row, rather than sampled.
#[test]
fn the_sharing_decision_table_is_total() {
    use metering::allocation::sharing::{
        Bilanzierungsmethode, Capability, Delivery, EligibilityBasis, MeteringCapabilityInput,
        SharingReadiness, Zaehlertyp, assess_capability, combine_readiness,
    };

    fn maybe<T: Copy>(all: &[T]) -> Vec<Option<T>> {
        std::iter::once(None)
            .chain(all.iter().copied().map(Some))
            .collect()
    }

    let mut rows = 0usize;
    let mut verdicts = 0usize;
    for zaehlertyp in maybe(&Zaehlertyp::ALL) {
        for ist_fernauslesbar in maybe(&[false, true]) {
            for bilanzierungsmethode in maybe(&Bilanzierungsmethode::ALL) {
                for smgw_operational in maybe(&[false, true]) {
                    rows += 1;
                    let input = MeteringCapabilityInput {
                        zaehlertyp,
                        ist_fernauslesbar,
                        bilanzierungsmethode,
                        smgw_operational,
                    };
                    let Ok((capability, findings)) = assess_capability(&input) else {
                        continue;
                    };
                    verdicts += 1;

                    match capability {
                        Capability::Qualified(basis) => {
                            assert!(
                                findings.is_empty(),
                                "{input:?}: a clean verdict carries no complaint"
                            );
                            assert!(!basis.legal_basis().is_empty(), "{input:?}");
                        }
                        Capability::Disqualified | Capability::Unknown => {
                            assert!(!findings.is_empty(), "{input:?}: a refusal must say why");
                        }
                    }
                    assert_eq!(
                        capability.basis().is_some(),
                        matches!(capability, Capability::Qualified(_)),
                        "{input:?}"
                    );

                    // RLM qualifies on its own limb and needs no gateway — the
                    // `oder` of § 42c Abs. 1 that a naive iMSys-only reading drops.
                    if bilanzierungsmethode == Some(Bilanzierungsmethode::Rlm) {
                        assert!(matches!(capability, Capability::Qualified(_)), "{input:?}");
                    }
                    // A gateway reported down never qualifies the
                    // Zählerstandsgang limb.
                    if smgw_operational == Some(false) {
                        assert!(
                            !matches!(
                                capability,
                                Capability::Qualified(EligibilityBasis::Zaehlerstandsgangmessung)
                            ),
                            "{input:?}"
                        );
                    }

                    // Combining is total, and delivery alone never establishes
                    // eligibility.
                    for delivery in Delivery::ALL {
                        let verdict = combine_readiness(capability, delivery);
                        assert_eq!(
                            verdict == SharingReadiness::Ready,
                            matches!(capability, Capability::Qualified(_))
                                && delivery == Delivery::Delivering,
                            "{input:?} / {delivery:?}"
                        );
                        assert!(!verdict.required_action().is_empty());
                    }
                }
            }
        }
    }
    assert_eq!(
        rows,
        (Zaehlertyp::ALL.len() + 1) * 3 * (Bilanzierungsmethode::ALL.len() + 1) * 3
    );
    assert!(
        verdicts > rows / 2,
        "only {verdicts} of {rows} rows reached a verdict"
    );
}

// ── session ──────────────────────────────────────────────────────────────────

/// Register readings of one session: a monotone walk from an off-grid start,
/// so the first and last reading bound the span and every other one lies
/// inside it.
fn arb_session() -> impl Strategy<Value = Vec<MeterSample>> {
    (
        0i64..900,                                               // offset off the grid
        prop::collection::vec((1i64..2_000, 0i64..1000), 1..24), // steps
    )
        .prop_map(|(offset, rows)| {
            let mut at = BASE + Duration::seconds(offset);
            let mut reading = Decimal::new(1_000_000, 3);
            let mut samples = vec![MeterSample::new(at, reading)];
            for (step, delta) in rows {
                at += Duration::seconds(step);
                reading += Decimal::new(delta, 3);
                samples.push(MeterSample::new(at, reading));
            }
            samples
        })
}

/// The session total the readings state: last minus first.
fn total_of(samples: &[MeterSample]) -> Decimal {
    samples[samples.len() - 1].reading - samples[0].reading
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// A session's energy arrives on the grid whole: the slots sum to the
    /// readings' difference, none is negative, and they tile the span.
    #[test]
    fn a_session_split_conserves_its_total(samples in arb_session()) {
        let series = split_session(&samples, &SessionSplitConfig::quarter_hourly())
            .expect("the generated readings are consistent");
        let slots = series.as_slice();

        prop_assert_eq!(slots.iter().map(|s| s.value()).sum::<Decimal>(), total_of(&samples));
        prop_assert!(slots.iter().all(|s| s.value() >= Decimal::ZERO));
        prop_assert!(!slots.is_empty());

        for pair in slots.windows(2) {
            prop_assert_eq!(pair[0].to(), pair[1].from());
        }
        prop_assert!(slots[0].from() <= samples[0].at);
        prop_assert!(slots[slots.len() - 1].to() >= samples[samples.len() - 1].at);
        prop_assert!(slots.iter().all(|s| s.value().scale() <= metering::precision::ALLOCATION_DP));
    }

    /// A coarser grid is a partition of the same energy.
    #[test]
    fn the_grid_does_not_change_the_energy(samples in arb_session()) {
        let cfg = SessionSplitConfig::quarter_hourly();
        let quarters = split_session(&samples, &cfg).unwrap();
        let hours = split_session(&samples, &cfg.at(Resolution::Hour)).unwrap();
        prop_assert_eq!(quarters.billable_total(), hours.billable_total());
    }
}

// ── directional balance ──────────────────────────────────────────────────────

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// The three buckets partition the input: nothing is counted twice and
    /// nothing falls out.
    #[test]
    fn a_directional_split_partitions_the_series(series in arb_series(0..96)) {
        let codes = ["1-0:1.8.0", "1-0:2.8.0", "1-0:3.8.0"];
        let tagged: Vec<MeterInterval> = series
            .iter()
            .enumerate()
            .map(|(i, iv)| iv.clone().with_obis(codes[i % 3].parse().unwrap()))
            .collect();

        let split = sum_by_direction(&tagged);
        prop_assert_eq!(split.total(), tagged.iter().map(|iv| iv.value()).sum::<Decimal>());
        prop_assert_eq!(split.net(), split.import - split.export);

        // Untagged intervals are undirected, never silently dropped.
        let untagged = sum_by_direction(&series);
        prop_assert_eq!(untagged.import, Decimal::ZERO);
        prop_assert_eq!(untagged.export, Decimal::ZERO);
        prop_assert_eq!(untagged.undirected, series.iter().map(|iv| iv.value()).sum::<Decimal>());
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// A slot that comes back `Measured` carries exactly the energy the
    /// readings inside it say: it holds no segment that a slot boundary cut.
    #[test]
    fn a_measured_slot_is_the_register_difference(samples in arb_session()) {
        let series = split_session(&samples, &SessionSplitConfig::quarter_hourly())
            .expect("the generated readings are consistent");
        let first = samples[0].at;
        let last = samples[samples.len() - 1].at;

        for slot in series.iter() {
            if slot.quality() != QualityFlag::Measured {
                continue;
            }
            let lo = slot.from().max(first);
            let hi = slot.to().min(last);
            let at = |t| samples.iter().find(|s| s.at == t).map(|s| s.reading);
            let (Some(open), Some(close)) = (at(lo), at(hi)) else {
                return Err(TestCaseError::fail("a measured slot is bounded by readings"));
            };
            prop_assert_eq!(slot.value(), close - open);
        }
    }

    /// Merging is a union: every slot any session touched appears exactly
    /// once, and the sum is the sum of the totals.
    #[test]
    fn merging_sessions_conserves_every_total(
        samples in arb_session(),
        offset in 0i64..3_600,
        second in (0i64..40_000).prop_map(|milli| Decimal::new(milli, 3)),
    ) {
        let cfg = SessionSplitConfig::quarter_hourly();
        let a = split_session(&samples, &cfg).unwrap();
        let start = samples[0].at + Duration::seconds(offset);
        let b = split_session(
            &[
                MeterSample::new(start, Decimal::ZERO),
                MeterSample::new(start + Duration::hours(2), second),
            ],
            &cfg,
        )
        .unwrap();

        let merged = merge_sessions(&cfg, &[a.clone(), b.clone()]).unwrap();
        prop_assert_eq!(merged.billable_total(), Some(total_of(&samples) + second));

        let mut slots: Vec<_> = a.iter().chain(b.iter()).map(|iv| iv.from()).collect();
        slots.sort_unstable();
        slots.dedup();
        prop_assert_eq!(merged.len(), slots.len());
    }
}
