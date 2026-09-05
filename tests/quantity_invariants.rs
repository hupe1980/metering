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

use metering::session::{MeterSample, SessionSplitConfig, merge_sessions, split_session};
use metering::{
    AggregationConfig, AllocationBasis, AllocationPart, FillGapsConfig, G685FinalRounding,
    G685Rounding, GasConversionParams, IntervalResolution, MeasurementUnit, MeterInterval,
    QualityFlag, ResampleConfig, SubstituteMethod, Zaehlzeitdefinition, ZustandszahlParams,
    aggregate, allocate, compute_imbalance, fill_gaps, gas_m3_to_kwh_hs, gas_m3_to_kwh_hs_rounded,
    hoehenzonen_luftdruck_mbar, network_losses, normalize_to_kwh, project_annual_consumption,
    resample, sum_by_direction, zustandszahl,
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
                MeterInterval {
                    from,
                    to: from + Duration::minutes(15),
                    value,
                    quality,
                    obis_code: None,
                }
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
                MeterInterval {
                    from,
                    to: from + Duration::minutes(15),
                    value,
                    quality,
                    obis_code: None,
                }
            })
            .collect()
    })
}

// ── substitute ───────────────────────────────────────────────────────────────

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// `fill_gaps` completes a grid: the output is every slot of the period,
    /// once each, ascending and tiling without gap or overlap.
    #[test]
    fn a_filled_series_is_exactly_the_grid(
        measured in arb_sparse_series(),
        threshold in 0usize..6,
    ) {
        let period = (BASE, BASE + Duration::hours(24));
        let cfg = FillGapsConfig::new(IntervalResolution::QuarterHour, period.0, period.1)
            .short_gap_threshold(threshold);
        let filled = fill_gaps(&measured, &cfg);

        prop_assert_eq!(filled.intervals.len(), 96, "a day of quarter-hours");
        prop_assert_eq!(filled.intervals[0].from, period.0);
        prop_assert_eq!(filled.intervals[95].to, period.1);
        for pair in filled.intervals.windows(2) {
            prop_assert_eq!(pair[0].to, pair[1].from, "slots must tile");
            prop_assert!(pair[0].from < pair[1].from, "and ascend");
        }
    }

    /// A measured value is never overwritten, and every value that is not a
    /// measured one is accounted for in the audit trail — exactly once.
    #[test]
    fn every_invented_value_is_in_the_audit_trail(
        measured in arb_sparse_series(),
        threshold in 0usize..6,
    ) {
        let cfg = FillGapsConfig::new(
            IntervalResolution::QuarterHour,
            BASE,
            BASE + Duration::hours(24),
        )
        .short_gap_threshold(threshold);
        let filled = fill_gaps(&measured, &cfg);

        // Each supplied interval survives untouched at its own slot.
        for iv in &measured {
            let found = filled
                .intervals
                .iter()
                .find(|out| out.from == iv.from)
                .expect("its slot is in the period");
            prop_assert_eq!(found, iv, "a delivered value must not be rewritten");
        }

        // The audit trail lines up with the output, one entry per slot.
        prop_assert_eq!(
            filled.substitutions.len(),
            filled.intervals.len() - measured.len(),
        );
        for entry in &filled.substitutions {
            prop_assert_eq!(entry.interval.quality, QualityFlag::Substituted);
            let out = filled
                .intervals
                .iter()
                .find(|iv| iv.from == entry.interval.from)
                .expect("the entry names an output slot");
            prop_assert_eq!(out, &entry.interval, "the trail must match the series");
            // A method that rests on no evidence says so.
            let expected_refs = match entry.method {
                SubstituteMethod::ZeroFill => 0,
                SubstituteMethod::LastValueCarryForward => 1,
                SubstituteMethod::LinearInterpolation => 2,
                SubstituteMethod::PriorPeriodAverage => entry.reference_count,
            };
            prop_assert_eq!(entry.reference_count, expected_refs, "{:?}", entry.method);
            if entry.reference_count == 0 {
                prop_assert_eq!(entry.interval.value, Decimal::ZERO);
            }
        }
        prop_assert!((0.0..=100.0).contains(&filled.measured_pct()));
    }

    /// An interpolated value lies between the two billable values it was
    /// interpolated from — a straight line does not leave its own endpoints.
    #[test]
    fn an_interpolated_value_stays_between_its_anchors(
        measured in arb_sparse_series(),
    ) {
        let cfg = FillGapsConfig::new(
            IntervalResolution::QuarterHour,
            BASE,
            BASE + Duration::hours(24),
        )
        .short_gap_threshold(96);
        let filled = fill_gaps(&measured, &cfg);

        let billable: Vec<Decimal> = filled
            .intervals
            .iter()
            .filter(|iv| iv.quality.is_billable())
            .map(|iv| iv.value)
            .collect();
        let (Some(lo), Some(hi)) = (
            billable.iter().copied().reduce(Decimal::min),
            billable.iter().copied().reduce(Decimal::max),
        ) else {
            return Ok(());
        };
        for entry in &filled.substitutions {
            if entry.method == SubstituteMethod::LinearInterpolation {
                prop_assert!(
                    entry.interval.value >= lo && entry.interval.value <= hi,
                    "{} outside [{lo}, {hi}]",
                    entry.interval.value,
                );
            }
        }
    }
}

// ── resample ─────────────────────────────────────────────────────────────────

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// Down-sampling conserves energy and loses no interval: the buckets sum to
    /// the input and count it, whatever the target grid.
    #[test]
    fn resampling_conserves_energy_and_counts(series in arb_series(0..80)) {
        for cfg in [
            ResampleConfig::to_hourly(),
            ResampleConfig::to_daily(),
            ResampleConfig::to_daily().on(metering::DayBoundary::Gastag),
            ResampleConfig::to_monthly(),
            ResampleConfig::to_yearly(),
        ] {
            let buckets = resample(&series, &cfg);
            let total: Decimal = buckets.iter().map(|b| b.total).sum();
            let counted: u32 = buckets.iter().map(|b| b.interval_count).sum();
            prop_assert_eq!(total, series.iter().map(|iv| iv.value).sum::<Decimal>());
            prop_assert_eq!(counted as usize, series.len());

            // Buckets are ascending and disjoint, and each contains its own start.
            for pair in buckets.windows(2) {
                prop_assert!(pair[0].to <= pair[1].from, "buckets must not overlap");
                prop_assert!(pair[0].from < pair[1].from);
            }
            for b in &buckets {
                prop_assert!(b.from < b.to);
                prop_assert!(b.interval_count > 0, "an empty bucket is not emitted");
            }
        }
    }
}

// ── zaehlzeit ────────────────────────────────────────────────────────────────

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// A register split reconstructs the Arbeitsmenge exactly, and books into
    /// nothing the definition does not declare.
    #[test]
    fn the_register_split_reconstructs_the_arbeitsmenge(series in arb_series(0..96)) {
        let zzd = Zaehlzeitdefinition::modul_3(
            "NB-14A-3",
            date!(2026 - 01 - 01),
            (17 * 60, 20 * 60),
            (22 * 60, 6 * 60),
        );
        let split = zzd.split_energy(&series);
        let period = aggregate(&series, &AggregationConfig::rlm());

        prop_assert_eq!(split.values().sum::<Decimal>(), period.arbeitsmenge);
        let declared = zzd.registers();
        for key in split.keys() {
            match key {
                Some(id) => prop_assert!(declared.contains(id), "undeclared register {id}"),
                None => prop_assert!(true, "unassigned energy is a legitimate bucket"),
            }
        }
    }
}

// ── forecast ─────────────────────────────────────────────────────────────────

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// The projection is its own stated formula, and the prediction interval
    /// brackets it.
    #[test]
    fn a_projection_is_its_own_formula(series in arb_series(700..900)) {
        let Some(f) = project_annual_consumption(&series, None) else {
            return Ok(());
        };
        prop_assert_eq!(
            f.observed_days as i64,
            metering::calendar::days_between(f.observation_from, f.observation_to),
        );
        prop_assert_eq!(
            f.observed,
            series
                .iter()
                .filter(|iv| iv.quality.is_billable())
                .map(|iv| iv.value)
                .sum::<Decimal>(),
        );
        prop_assert_eq!(f.seasonal_factor, Decimal::ONE);
        prop_assert!(!f.seasonal_correction_applied);
        prop_assert_eq!(
            f.projected_annual,
            (f.daily_average_kwh() * Decimal::from(f.target_year_days))
                .round_dp(metering::FORECAST_DP),
        );
        prop_assert!(
            f.projected_annual.scale() <= metering::FORECAST_DP,
            "a projection must be a number an Abschlag can carry: {}",
            f.projected_annual,
        );

        if let (Some(lo), Some(hi)) = (f.confidence_lower, f.confidence_upper) {
            prop_assert!(lo >= Decimal::ZERO, "a consumption cannot be negative");
            prop_assert!(lo <= f.projected_annual, "{lo} > {}", f.projected_annual);
            prop_assert!(hi >= f.projected_annual, "{hi} < {}", f.projected_annual);
        }
    }

    /// Doubling every reading doubles the projection — **to the last reported
    /// place of the daily average it is built on**.
    ///
    /// Scaling is what an extrapolation *is*, so it must survive one. It
    /// survives it only up to a rounding boundary: the daily average is cut to
    /// `FORECAST_DP` before it is multiplied out, so that the projection can be
    /// re-derived from the figures the report itself states, and `round(2x)`
    /// and `2·round(x)` differ by up to one unit in that place — which the
    /// multiplication then scales by the length of the year.
    #[test]
    fn a_projection_scales_with_its_input(series in arb_series(700..900)) {
        let Some(base) = project_annual_consumption(&series, None) else {
            return Ok(());
        };
        let doubled: Vec<MeterInterval> = series
            .iter()
            .map(|iv| MeterInterval { value: iv.value * Decimal::TWO, ..iv.clone() })
            .collect();
        let scaled = project_annual_consumption(&doubled, None).expect("same window");

        prop_assert_eq!(scaled.observed, base.observed * Decimal::TWO);
        prop_assert_eq!(scaled.observed_days, base.observed_days);
        let drift = scaled.projected_annual - base.projected_annual * Decimal::TWO;
        // One unit in the average's last place, carried through a year, plus
        // the projection's own final rounding.
        let tolerance = Decimal::new(1, metering::FORECAST_DP)
            * Decimal::from(scaled.target_year_days)
            + Decimal::new(1, metering::FORECAST_DP);
        prop_assert!(
            drift.abs() <= tolerance,
            "{} vs 2 × {}",
            scaled.projected_annual,
            base.projected_annual,
        );

        // ...and the projection is exactly what the reported average says it
        // is, which is the property the cut exists for.
        prop_assert_eq!(
            scaled.projected_annual,
            (scaled.daily_average_kwh() * Decimal::from(scaled.target_year_days))
                .round_dp(metering::FORECAST_DP),
        );
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
        let limit = metering::ReactiveLimit::new(Decimal::new(ratio_millis, 3));
        let b = metering::blindmehrarbeit(wirk, blind, limit);

        prop_assert_eq!(b.freigrenze_kvarh, limit.ratio * wirk);
        prop_assert!(b.blindmehrarbeit_kvarh >= Decimal::ZERO);
        prop_assert!(b.headroom_kvarh() >= Decimal::ZERO);
        // Exactly one of the two is non-zero, and together they close the gap.
        prop_assert_eq!(
            b.freigrenze_kvarh + b.blindmehrarbeit_kvarh - b.headroom_kvarh(),
            b.blindarbeit_kvarh
        );
        prop_assert_eq!(b.is_chargeable(), blind > b.freigrenze_kvarh);
    }

    /// A stricter ratio never charges less.
    #[test]
    fn a_smaller_freigrenze_never_charges_less(
        kwh in 1i64..1_000_000,
        kvarh in 0i64..1_000_000,
    ) {
        let wirk = Decimal::from(kwh);
        let blind = Decimal::from(kvarh);
        let loose = metering::blindmehrarbeit(wirk, blind, metering::ReactiveLimit::half());
        let strict =
            metering::blindmehrarbeit(wirk, blind, metering::ReactiveLimit::cos_phi_0_9());
        prop_assert!(strict.blindmehrarbeit_kvarh >= loose.blindmehrarbeit_kvarh);
    }

    /// The Benutzungsstundenzahl is bounded by the period it is measured over:
    /// a load that never exceeds its own peak cannot run more hours than the
    /// period holds, and a flat load runs exactly all of them.
    #[test]
    fn the_utilisation_hours_are_bounded_by_the_period(
        values in prop::collection::vec(1i64..4_000, 96..=96),
    ) {
        let base = datetime!(2026-06-01 0:00 UTC);
        let day: Vec<MeterInterval> = values
            .iter()
            .enumerate()
            .map(|(i, v)| MeterInterval {
                from: base + Duration::minutes(15 * i as i64),
                to: base + Duration::minutes(15 * i as i64 + 15),
                value: Decimal::new(*v, 2),
                quality: QualityFlag::Measured,
                obis_code: None,
            })
            .collect();

        let period = aggregate(&day, &AggregationConfig::rlm());
        let hours = period.benutzungsdauer_h().expect("a positive peak");
        prop_assert!(hours > Decimal::ZERO);
        prop_assert!(hours <= Decimal::from(24u32), "{hours} h in a 24 h day");
        prop_assert!(period.uniform_resolution);

        // A flat day uses every hour of itself.
        let flat: Vec<MeterInterval> = day
            .iter()
            .map(|iv| MeterInterval { value: Decimal::ONE, ..iv.clone() })
            .collect();
        prop_assert_eq!(
            aggregate(&flat, &AggregationConfig::rlm()).benutzungsdauer_h(),
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
        prop_assert_eq!(gas_m3_to_kwh_hs(m3, hs, z), m3 * hs * z);

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
        let luftdruck = hoehenzonen_luftdruck_mbar(hoehe);
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
            luftdruck - Decimal::new(12, 2) * step,
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
        let exact = gas_m3_to_kwh_hs(m3, hs, z);
        // The inputs are already at the configured precision, so only the final
        // rounding can move anything.
        let unrounded = gas_m3_to_kwh_hs_rounded(m3, hs, z, G685Rounding::default());
        prop_assert_eq!(unrounded, exact, "the default rounds nothing");

        for (mode, granularity) in [
            (G685FinalRounding::WholeKwh, Decimal::new(5, 1)),
            (G685FinalRounding::TwoDecimals, Decimal::new(5, 3)),
        ] {
            let rounded = gas_m3_to_kwh_hs_rounded(
                m3, hs, z,
                G685Rounding { final_rounding: mode, ..G685Rounding::default() },
            );
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
        let scale = MeasurementUnit::parse_scaled(unit).expect("an accepted unit");
        prop_assert_eq!(scale.unit, MeasurementUnit::KiloWattHour);

        let converted = normalize_to_kwh(value, unit, None, None).unwrap();
        prop_assert_eq!(converted, scale.apply(value), "one path, one answer");
        prop_assert_eq!(
            converted * Decimal::from(scale.den),
            value * Decimal::from(scale.num),
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
        let scale = MeasurementUnit::parse_scaled(unit).expect("an accepted unit");
        let converted = normalize_to_kwh(value, unit, None, None).unwrap();
        prop_assert_eq!(converted, scale.apply(value), "one path, one answer");

        let residue = converted * Decimal::from(scale.den) - value * Decimal::from(scale.num);
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
        contracted in arb_signed_kwh(),
    ) {
        let s = compute_imbalance(actual, contracted);
        prop_assert_eq!(s.delta_kwh, actual - contracted);
        prop_assert_eq!(s.minder_kwh - s.mehr_kwh, s.delta_kwh);
        prop_assert!(s.mehr_kwh >= Decimal::ZERO && s.minder_kwh >= Decimal::ZERO);
        prop_assert!(!(s.is_mehr() && s.is_minder()), "both sides cannot be open");
        prop_assert_eq!(s.is_balanced(), s.delta_kwh.is_zero());
        prop_assert_eq!(s.magnitude_kwh(), s.delta_kwh.abs());
        prop_assert_eq!(s.delta_pct().is_some(), !contracted.is_zero());
    }

    /// The loss balance is a difference, and the share is that difference over
    /// the infeed.
    #[test]
    fn a_loss_balance_is_a_difference(
        einspeisung in arb_kwh(),
        entnahme in arb_kwh(),
    ) {
        let l = network_losses(einspeisung, entnahme);
        prop_assert_eq!(l.verlust_kwh, einspeisung - entnahme);
        prop_assert_eq!(l.verlust_prozent.is_some(), einspeisung > Decimal::ZERO);
        if let Some(pct) = l.verlust_prozent {
            prop_assert_eq!(pct.is_sign_negative(), l.verlust_kwh.is_sign_negative());
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
        use metering::allocation_temperature;
        let [a, b, c, d] = temps;
        let theta = allocation_temperature(a, b, c, d);
        let lo = a.min(b).min(c).min(d);
        let hi = a.max(b).max(c).max(d);
        prop_assert!(theta >= lo && theta <= hi, "{theta} outside [{lo}, {hi}]");
        prop_assert_eq!(allocation_temperature(a, a, a, a), a, "a constant week");

        // The published weights are exact integers; the division by 15 is not,
        // so the numerator comes back only to `Decimal`'s width. A tolerance
        // of one unit in the last reported place of a temperature is four
        // orders of magnitude tighter than any thermometer.
        let numerator =
            Decimal::from(8u32) * a + Decimal::from(4u32) * b + Decimal::from(2u32) * c + d;
        prop_assert!(
            (theta * Decimal::from(15u32) - numerator).abs() < Decimal::new(1, 6),
            "{theta} × 15 vs {numerator}",
        );
        // Where the quotient *does* terminate it is exact to the digit.
        let fifteen = Decimal::from(15u32);
        let clean = Decimal::from(30u32);
        prop_assert_eq!(
            allocation_temperature(clean, clean, clean, clean) * fifteen,
            clean * fifteen,
        );
    }

    /// `Q = KW · h · F_WT` is an exact product, and the Kundenwert inverts it.
    #[test]
    fn the_daily_quantity_and_the_kundenwert_are_inverses(
        kw in (1i64..1_000_000).prop_map(|n| Decimal::new(n, 4)),
        h in (1i64..5_000_000).prop_map(|n| Decimal::new(n, 6)),
    ) {
        use metering::{gas_daily_quantity, kundenwert};

        let q = gas_daily_quantity(kw, h, Decimal::ONE);
        prop_assert_eq!(q, kw * h);

        // Recovering the Kundenwert from the quantity and the profile sum
        // returns the same figure, to the Leitfaden's four places.
        let recovered = kundenwert(q, h).expect("a positive divisor");
        prop_assert_eq!(recovered, kw.round_dp(4));

        prop_assert_eq!(kundenwert(q, Decimal::ZERO), None);
        prop_assert_eq!(kundenwert(q, -h), None);
    }

    /// The profile function is a consumption share: never negative, and finite
    /// across the whole temperature range a German winter and summer produce.
    #[test]
    fn the_profile_function_is_a_non_negative_share(
        theta in (-30_000i64..45_000).prop_map(|n| Decimal::new(n, 3)),
    ) {
        use metering::SigLinDe;
        let h = SigLinDe::DE_HEF34.h_value(theta);
        prop_assert!(h >= Decimal::ZERO, "h({theta}) = {h}");
        prop_assert!(h <= Decimal::from(10u32), "h({theta}) = {h} is off the scale");

        // Warmer weather never means more heating gas.
        let warmer = SigLinDe::DE_HEF34.h_value(theta + Decimal::ONE);
        prop_assert!(warmer <= h, "h({}) = {warmer} > h({theta}) = {h}", theta + Decimal::ONE);
    }

    /// The standard week sums to 7,0000 or the factors are refused — the
    /// Leitfaden's own consistency rule, which a rescaled set would break
    /// silently.
    #[test]
    fn weekday_factors_must_sum_to_seven(
        raw in prop::array::uniform7((1i64..20_000).prop_map(|n| Decimal::new(n, 4))),
    ) {
        use metering::WeekdayFactors;

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
        use metering::reading::{LastgangConfig, MeterReading, to_lastgang};

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

        let out = to_lastgang(&readings, &LastgangConfig::strom());
        prop_assert!(out.is_clean(), "a monotone register has nothing to explain");
        prop_assert_eq!(out.intervals.len(), readings.len() - 1);
        prop_assert_eq!(
            out.total(),
            readings[readings.len() - 1].value - readings[0].value,
            "the Lastgang sums to the register difference",
        );
        // Every pair is accounted for exactly once.
        prop_assert_eq!(
            out.intervals.len() + out.anomalies.len(),
            readings.len() - 1,
        );
        for pair in out.intervals.windows(2) {
            prop_assert_eq!(pair[0].to, pair[1].from, "derived intervals tile");
        }
        for iv in &out.intervals {
            prop_assert!(iv.value >= Decimal::ZERO, "a forward step is non-negative");
        }
    }
}

// ── power quality ────────────────────────────────────────────────────────────

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// Every EN 50160 outcome is a partition of its own samples, and the share
    /// it reports is that partition as a percentage.
    #[test]
    fn an_en50160_outcome_partitions_its_samples(
        volts in prop::collection::vec(
            (180_000i64..280_000).prop_map(|n| Decimal::new(n, 3)),
            0..200,
        ),
    ) {
        use metering::power_quality::{En50160Limits, PowerQualityInterval, assess_en50160};

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

        prop_assert_eq!(report.intervals, series.len());
        for (name, outcome) in report.outcomes() {
            prop_assert!(outcome.within <= outcome.samples, "{name}");
            prop_assert!((0.0..=100.0).contains(&outcome.share_pct), "{name}");
            prop_assert_eq!(
                outcome.worst.is_some(),
                outcome.within < outcome.samples,
                "{}: a worst sample exists exactly when one is outside",
                name,
            );
            prop_assert!(
                (metering::exceedance_pct(outcome) + outcome.share_pct - 100.0).abs() < 1e-9,
                "{name}",
            );
        }
        // The ±10 % band sits **inside** the +10 %/−15 % absolute limits: same
        // ceiling, a lower floor. So every sample inside the band is inside the
        // absolute limits, and `within` can only grow in that direction. A
        // 195,5 V reading is the witness — outside the band, exactly on the
        // absolute floor.
        prop_assert!(
            report.voltage_absolute.within >= report.voltage_band.within,
            "absolute {} < band {}",
            report.voltage_absolute.within,
            report.voltage_band.within,
        );
        prop_assert_eq!(report.voltage_absolute.samples, report.voltage_band.samples);
    }

    /// The Unsymmetrieleistung is a spread: never negative, unchanged by
    /// relabelling the Außenleiter, and zero exactly when the three are equal.
    #[test]
    fn unbalance_is_a_permutation_invariant_spread(
        a in (0i64..30_000).prop_map(|n| Decimal::new(n, 3)),
        b in (0i64..30_000).prop_map(|n| Decimal::new(n, 3)),
        c in (0i64..30_000).prop_map(|n| Decimal::new(n, 3)),
    ) {
        use metering::power_quality::{Phase, PhaseApparentPower};

        let build = |x, y, z| PhaseApparentPower::default()
            .plus(Phase::L1, x)
            .plus(Phase::L2, y)
            .plus(Phase::L3, z);

        let base = build(a, b, c).unbalance_kva();
        prop_assert!(base >= Decimal::ZERO);
        for (x, y, z) in [(a, c, b), (b, a, c), (b, c, a), (c, a, b), (c, b, a)] {
            prop_assert_eq!(build(x, y, z).unbalance_kva(), base, "phase order");
        }
        prop_assert_eq!(base.is_zero(), a == b && b == c);
        prop_assert_eq!(
            build(a, b, c).within_limit(None),
            base <= metering::UNSYMMETRIE_LIMIT_KVA,
        );
        prop_assert_eq!(
            build(a, b, c).excess_kva(None),
            (base - metering::UNSYMMETRIE_LIMIT_KVA).max(Decimal::ZERO),
        );
    }
}

// ── § 14a ────────────────────────────────────────────────────────────────────

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// The floor rises with the device count and never drops below the floor
    /// one device alone would have.
    #[test]
    fn the_mindestleistung_is_monotone_and_bounded_below(
        devices in prop::collection::vec(
            (
                (0usize..metering::SteuVeFallgruppe::ALL.len()),
                (4_300i64..60_000).prop_map(|n| Decimal::new(n, 3)),
            ),
            1..12,
        ),
    ) {
        use metering::{Para14aConfig, SteuVe, SteuVeFallgruppe, mindestleistung_ems};

        let cfg = Para14aConfig::default();
        let set: Vec<SteuVe> = devices
            .iter()
            .map(|(g, kw)| SteuVe::new(SteuVeFallgruppe::ALL[*g], *kw))
            .collect();
        prop_assert!(set.iter().all(SteuVe::is_steuerbar));

        let floor = mindestleistung_ems(&set, &cfg).expect("a set of steuVE");
        prop_assert!(floor >= metering::MINDESTLEISTUNG_KW, "{floor}");

        // Adding a device never lowers the floor.
        let mut grown = set.clone();
        grown.push(SteuVe::new(SteuVeFallgruppe::Ladepunkt, Decimal::from(11u32)));
        let bigger = mindestleistung_ems(&grown, &cfg).expect("still steuVE");
        prop_assert!(bigger >= floor, "{bigger} < {floor}");

        // …and one that is not a steuVE refuses the whole set.
        let mut spoiled = set;
        spoiled.push(SteuVe::new(SteuVeFallgruppe::Ladepunkt, Decimal::from(3u32)));
        prop_assert_eq!(mindestleistung_ems(&spoiled, &cfg), None);
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
        use metering::{Verursachungsregel as R, netzwirksamer_leistungsbezug};

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

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    /// The § 42c decision table is total and self-consistent: every input
    /// reaches a verdict, a qualified point names its statutory limb, and a
    /// disqualified one always says why.
    #[test]
    fn the_sharing_decision_table_is_total(
        zaehlertyp in prop::option::of(0usize..metering::Zaehlertyp::ALL.len()),
        fernauslesbar in prop::option::of(any::<bool>()),
        methode in prop::option::of(0usize..metering::Bilanzierungsmethode::ALL.len()),
        smgw in prop::option::of(any::<bool>()),
    ) {
        use metering::{
            Bilanzierungsmethode, Capability, MeteringCapabilityInput, SharingReadiness,
            Zaehlertyp, assess_capability,
        };

        let input = MeteringCapabilityInput {
            zaehlertyp: zaehlertyp.map(|i| Zaehlertyp::ALL[i]),
            ist_fernauslesbar: fernauslesbar,
            bilanzierungsmethode: methode.map(|i| Bilanzierungsmethode::ALL[i]),
            smgw_operational: smgw,
        };
        let (capability, findings) = assess_capability(&input);

        match capability {
            Capability::Qualified(basis) => {
                prop_assert!(findings.is_empty(), "a clean verdict carries no complaint");
                prop_assert!(!basis.legal_basis().is_empty());
            }
            Capability::Disqualified | Capability::Unknown => {
                prop_assert!(!findings.is_empty(), "a refusal must say why");
            }
        }
        prop_assert_eq!(capability.basis().is_some(), matches!(capability, Capability::Qualified(_)));

        // RLM qualifies on its own limb and needs no gateway — the `oder` of
        // § 42c Abs. 1 that a naive iMSys-only reading drops.
        if input.bilanzierungsmethode == Some(Bilanzierungsmethode::Rlm) {
            prop_assert!(matches!(capability, Capability::Qualified(_)));
        }

        // Combining is total, and delivery alone never establishes eligibility.
        for delivery in metering::Delivery::ALL {
            let verdict = metering::combine_readiness(capability, delivery);
            prop_assert_eq!(
                verdict == SharingReadiness::Ready,
                matches!(capability, Capability::Qualified(_))
                    && delivery == metering::Delivery::Delivering,
            );
            prop_assert!(!verdict.required_action().is_empty());
        }
    }
}

// ── allocation ───────────────────────────────────────────────────────────────

/// A pool and a set of weighted, optionally ceilinged claims on it.
fn arb_pool() -> impl Strategy<Value = (Decimal, Vec<AllocationPart>, AllocationBasis)> {
    (
        arb_kwh(),
        prop::collection::vec((arb_kwh(), prop::option::of(arb_kwh())), 0..6),
        any::<bool>(),
    )
        .prop_map(|(total, rows, proportional)| {
            let basis = if proportional {
                AllocationBasis::Proportional
            } else {
                AllocationBasis::Fraction
            };
            let n = rows.len().max(1) as i64;
            let parts = rows
                .into_iter()
                .enumerate()
                .map(|(i, (weight, capacity))| {
                    // A `Fraction` key is only valid if the weights are
                    // positive and sum to at most 1, so build one that is —
                    // the property under test is the identity, not the guard.
                    let weight = match basis {
                        AllocationBasis::Fraction => Decimal::ONE / Decimal::from(n * 2),
                        AllocationBasis::Proportional => weight,
                    };
                    let mut part = AllocationPart::new(format!("P{i}"), weight);
                    if let Some(cap) = capacity {
                        part = part.capped_at(cap);
                    }
                    part
                })
                .collect();
            (total, parts, basis)
        })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// The identity the whole module exists for: nothing is created, and
    /// nothing is lost between the pool and the parts.
    #[test]
    fn an_allocation_conserves_its_total((total, parts, basis) in arb_pool()) {
        let row = allocate(total, parts, basis).expect("the generated key is valid");
        prop_assert_eq!(row.allocated() + row.residual, row.total);
        prop_assert_eq!(row.total, total);

        for part in &row.parts {
            prop_assert!(part.allocated >= Decimal::ZERO, "a part received a negative amount");
            prop_assert!(part.allocated <= part.share, "a part received more than its share");
            prop_assert!(part.forgone() >= Decimal::ZERO);
            prop_assert!(part.share.scale() <= metering::ALLOCATION_DP);
        }

        // With a non-negative pool, the parts can never take more than it holds.
        prop_assert!(row.allocated() <= total, "over-allocated");
        prop_assert!(row.residual >= Decimal::ZERO);
    }
}

// ── session ──────────────────────────────────────────────────────────────────

/// A session span, its total, and register readings inside it.
///
/// The readings are built from a monotone walk so they are always consistent
/// with the total: the last uncovered end absorbs whatever they do not
/// account for, which is exactly the contract `split_session` documents.
fn arb_session()
-> impl Strategy<Value = (OffsetDateTime, OffsetDateTime, Decimal, Vec<MeterSample>)> {
    (
        60i64..40_000,                                           // span, seconds
        0i64..900,                                               // offset off the grid
        arb_kwh(),                                               // session total
        prop::collection::vec((1i64..2_000, 0i64..1000), 0..24), // samples
    )
        .prop_map(|(span, offset, energy, rows)| {
            let from = BASE + Duration::seconds(offset);
            let to = from + Duration::seconds(span);

            // Sample instants strictly inside the span, and a register that
            // only ever climbs — never past the session total.
            let mut at = from;
            let mut reading = Decimal::new(1_000_000, 3);
            let mut samples = Vec::new();
            let mut placed = Decimal::ZERO;
            for (step, delta) in rows {
                at += Duration::seconds(step);
                if at >= to {
                    break;
                }
                let delta = Decimal::new(delta, 3).min(energy - placed);
                placed += delta;
                reading += delta;
                samples.push(MeterSample::new(at, reading));
            }
            (from, to, energy, samples)
        })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// A session's energy arrives on the grid whole: the slots sum to the
    /// total, none of them is negative, and they tile the span contiguously.
    #[test]
    fn a_session_split_conserves_its_total(
        (from, to, energy, samples) in arb_session(),
    ) {
        let slots = split_session(from, to, energy, &samples, &SessionSplitConfig::quarter_hourly())
            .expect("the generated session is consistent");

        prop_assert_eq!(slots.iter().map(|s| s.value).sum::<Decimal>(), energy);
        prop_assert!(slots.iter().all(|s| s.value >= Decimal::ZERO));
        prop_assert!(!slots.is_empty());

        // Contiguous, ascending, and covering the whole span.
        for pair in slots.windows(2) {
            prop_assert_eq!(pair[0].to, pair[1].from);
        }
        prop_assert!(slots[0].from <= from);
        prop_assert!(slots[slots.len() - 1].to >= to);

        // Every slot value is a quantity someone can write down.
        prop_assert!(slots.iter().all(|s| s.value.scale() <= metering::ALLOCATION_DP));
    }

    /// A coarser grid is a partition of the same energy: splitting onto hours
    /// and splitting onto quarter-hours give the same total.
    #[test]
    fn the_grid_does_not_change_the_energy(
        (from, to, energy, samples) in arb_session(),
    ) {
        let cfg = SessionSplitConfig::quarter_hourly();
        let quarters = split_session(from, to, energy, &samples, &cfg).unwrap();
        let hours = split_session(
            from, to, energy, &samples,
            &cfg.at(IntervalResolution::Hour),
        ).unwrap();
        prop_assert_eq!(
            quarters.iter().map(|s| s.value).sum::<Decimal>(),
            hours.iter().map(|s| s.value).sum::<Decimal>(),
        );
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
            .map(|(i, iv)| MeterInterval {
                obis_code: Some(codes[i % 3].parse().unwrap()),
                ..iv.clone()
            })
            .collect();

        let split = sum_by_direction(&tagged);
        prop_assert_eq!(split.total(), tagged.iter().map(|iv| iv.value).sum::<Decimal>());
        prop_assert_eq!(split.net(), split.import - split.export);

        // Untagged intervals are undirected, never silently dropped.
        let untagged = sum_by_direction(&series);
        prop_assert_eq!(untagged.import, Decimal::ZERO);
        prop_assert_eq!(untagged.export, Decimal::ZERO);
        prop_assert_eq!(untagged.undirected, series.iter().map(|iv| iv.value).sum::<Decimal>());
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// The exactness claim, tested directly rather than trusted: a slot that
    /// comes back `Measured` carries exactly the energy the register readings
    /// on its own boundaries say it does.
    #[test]
    fn a_measured_slot_is_the_register_difference(
        (from, to, energy, samples) in arb_session(),
    ) {
        let slots = split_session(from, to, energy, &samples, &SessionSplitConfig::quarter_hourly())
            .expect("the generated session is consistent");

        // A cumulative the samples state directly, with no interpolation.
        let reading_at = |t| samples.iter().find(|s| s.at == t).map(|s| s.reading);

        for slot in &slots {
            if slot.quality != QualityFlag::Measured {
                continue;
            }
            // A `Measured` slot is bounded by readings, unless the whole
            // session lies inside it — in which case its value is the total.
            if let (Some(open), Some(close)) = (reading_at(slot.from), reading_at(slot.to)) {
                prop_assert_eq!(slot.value, close - open);
            } else {
                prop_assert!(slot.from <= from && slot.to >= to);
                prop_assert_eq!(slot.value, energy);
            }
        }
    }

    /// Merging is a union: every slot any session touched appears exactly
    /// once, and the sum is the sum of the totals.
    #[test]
    fn merging_sessions_conserves_every_total(
        (from, to, energy, samples) in arb_session(),
        offset in 0i64..3_600,
        second in (0i64..40_000).prop_map(|milli| Decimal::new(milli, 3)),
    ) {
        let cfg = SessionSplitConfig::quarter_hourly();
        let a = split_session(from, to, energy, &samples, &cfg).unwrap();
        let b = split_session(
            from + Duration::seconds(offset),
            to + Duration::seconds(offset),
            second,
            &[],
            &cfg,
        )
        .unwrap();

        let merged = merge_sessions(&[a.clone(), b.clone()]);
        prop_assert_eq!(
            merged.iter().map(|s| s.value).sum::<Decimal>(),
            energy + second,
        );

        // One row per distinct slot, ascending. Not necessarily contiguous:
        // two sessions with a gap between them touch no slot in the gap, and
        // inventing zeros there is `fill_gaps`, not this.
        let mut slots: Vec<_> = a.iter().chain(b.iter()).map(|iv| (iv.from, iv.to)).collect();
        slots.sort_unstable();
        slots.dedup();
        prop_assert_eq!(merged.len(), slots.len());
        for pair in merged.windows(2) {
            prop_assert!(pair[0].from < pair[1].from);
        }
    }
}
