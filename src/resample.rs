//! Time-series resampling — down-sample high-resolution intervals to coarser buckets.
//!
//! ## Use cases
//!
//! | Use case | Target resolution |
//! |---|---|
//! | API summaries (client dashboards) | Hourly or daily |
//! | MMM billing (GPKE BK6-22-024 §3) | Monthly totals |
//! | Mehr-/Mindermengensaldo (§ 13 StromNZV) | Monthly |
//! | MABIS Summenzeitreihe | Monthly |
//! | SLP compatibility (daily totals) | Daily |
//!
//! ## Invariants
//!
//! - `resample()` only **aggregates** — it never interpolates.
//! - Partial buckets (missing intervals) are flagged via `has_missing_data`.
//! - Peak demand (`peak_kw`) per bucket = maximum `kWh / interval_h` across contributors.
//! - Bucket quality = worst [`QualityFlag`] among contributing intervals.
//!
//! ## Calendar and DST handling
//!
//! Day, month and year buckets are **Europe/Berlin calendar periods**, resolved
//! through [`crate::calendar`]. A German Liefertag starts at 00:00 Berlin —
//! 23:00 UTC the previous day in winter, 22:00 UTC in summer — so bucketing on
//! the UTC date would book the first hour of every day, and the first hours of
//! every month, into the preceding period. For a §13 StromNZV monthly
//! Mehr-/Mindermengensaldo that is a billing error on every single settlement.
//!
//! Sub-daily buckets (quarter-hour, half-hour, hour, `Custom`) are snapped in
//! UTC, which is equivalent: every Europe/Berlin offset is a whole number of
//! hours, so local and UTC boundaries coincide at that granularity.
//!
//! `expected_count` follows from the bucket's real duration, so the
//! spring-forward day expects **92** quarter-hours and the fall-back day
//! **100** — not a flat 96 that would hide four missing intervals every autumn.
//!
//! `from` and `to` on a bucket remain UTC instants; convert with
//! [`crate::calendar::to_berlin`] for display.
//!
//! ## Regulatory basis
//!
//! - **§ 2 MsbG**: RLM = 15-min interval metering.
//! - **GPKE BK6-22-024 §3**: MMM billing uses monthly arbeitsmenge totals.
//! - **§ 13 StromNZV**: Mehr-/Mindermengen use calendar-month totals.

use std::collections::BTreeMap;

use rust_decimal::Decimal;
use time::{Duration, OffsetDateTime};

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use crate::calendar;
use crate::interval::{MeterInterval, QualityFlag};
use crate::resolution::IntervalResolution;

// ── ResampledBucket ───────────────────────────────────────────────────────────

/// A resampled bucket: one or more source intervals aggregated into a coarser window.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct ResampledBucket {
    /// Bucket start (UTC, inclusive).
    pub from: OffsetDateTime,
    /// Bucket end (UTC, exclusive).
    pub to: OffsetDateTime,
    /// Sum of all `value_kwh` from contributing intervals.
    pub total_kwh: Decimal,
    /// Peak demand in kW across contributing intervals.
    ///
    /// Computed as `max(interval.value_kwh / interval_duration_h)`.
    /// `None` only when no intervals contributed (should not normally occur).
    pub peak_kw: Option<Decimal>,
    /// Number of intervals that contributed to this bucket.
    pub interval_count: u32,
    /// Expected number of intervals for full coverage at the source resolution.
    ///
    /// When `interval_count < expected_count`, the bucket has missing data.
    pub expected_count: u32,
    /// Worst quality flag among all contributing intervals.
    pub quality: QualityFlag,
    /// `true` when some source intervals are missing (gap in the time series).
    pub has_missing_data: bool,
}

impl ResampledBucket {
    /// Coverage percentage (0.0–100.0).
    #[must_use]
    pub fn coverage_pct(&self) -> f64 {
        if self.expected_count == 0 {
            100.0
        } else {
            f64::from(self.interval_count) / f64::from(self.expected_count) * 100.0
        }
    }

    /// `true` when this bucket has complete, uninterrupted coverage.
    #[must_use]
    pub fn is_complete(&self) -> bool {
        !self.has_missing_data && self.interval_count >= self.expected_count
    }
}

// ── ResampleConfig ────────────────────────────────────────────────────────────

/// Configuration for [`resample`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResampleConfig {
    /// Target resolution to down-sample to.
    pub target_resolution: IntervalResolution,
    /// Resolution of the source intervals.
    ///
    /// Used to calculate `expected_count` per bucket, so it must be a fixed
    /// resolution ([`IntervalResolution::fixed_seconds`]); a calendar source
    /// resolution leaves `expected_count` at 0 and `has_missing_data` false,
    /// since no count can be derived. Default: [`IntervalResolution::QuarterHour`].
    pub source_resolution: IntervalResolution,
}

impl ResampleConfig {
    /// Resample from `source` to `target`.
    #[must_use]
    pub const fn new(source: IntervalResolution, target: IntervalResolution) -> Self {
        Self {
            target_resolution: target,
            source_resolution: source,
        }
    }

    /// Standard: resample 15-min RLM data to hourly buckets.
    #[must_use]
    pub const fn to_hourly() -> Self {
        Self::new(IntervalResolution::QuarterHour, IntervalResolution::Hour)
    }

    /// Standard: resample 15-min RLM data to Berlin calendar days.
    #[must_use]
    pub const fn to_daily() -> Self {
        Self::new(IntervalResolution::QuarterHour, IntervalResolution::Day)
    }

    /// Berlin calendar month totals — MMM billing and Mehr-/Mindermengensaldo
    /// (§ 13 StromNZV).
    #[must_use]
    pub const fn to_monthly() -> Self {
        Self::new(IntervalResolution::QuarterHour, IntervalResolution::Month)
    }

    /// Berlin calendar year totals — Jahresabrechnung.
    #[must_use]
    pub const fn to_yearly() -> Self {
        Self::new(IntervalResolution::QuarterHour, IntervalResolution::Year)
    }
}

impl Default for ResampleConfig {
    fn default() -> Self {
        Self::to_hourly()
    }
}

// ── resample ──────────────────────────────────────────────────────────────────

/// Down-sample a slice of meter intervals to the target resolution.
///
/// Input intervals do **not** need to be contiguous — gaps reduce `interval_count`
/// relative to `expected_count` and set `has_missing_data = true`.
///
/// Output is sorted ascending by `from`. Empty input returns an empty vec.
///
/// ## Panics
///
/// Does not panic.
#[must_use]
pub fn resample(intervals: &[MeterInterval], config: &ResampleConfig) -> Vec<ResampledBucket> {
    if intervals.is_empty() {
        return Vec::new();
    }

    // BTreeMap: bucket_start_unix → ResampledBucket (sorted automatically)
    let mut buckets: BTreeMap<i64, ResampledBucket> = BTreeMap::new();

    for iv in intervals {
        let (bucket_start, bucket_end) = bucket_bounds_for(iv.from, config.target_resolution);

        let entry = buckets
            .entry(bucket_start.unix_timestamp())
            .or_insert_with(|| {
                let expected = crate::calendar::intervals_between(
                    bucket_start,
                    bucket_end,
                    config.source_resolution,
                )
                .unwrap_or(0);
                ResampledBucket {
                    from: bucket_start,
                    to: bucket_end,
                    total_kwh: Decimal::ZERO,
                    peak_kw: None,
                    interval_count: 0,
                    expected_count: expected,
                    quality: QualityFlag::Measured,
                    has_missing_data: false,
                }
            });

        entry.total_kwh += iv.value_kwh;
        entry.interval_count += 1;

        // Peak demand = energy / duration_h
        let duration_secs = (iv.to - iv.from).whole_seconds().max(1);
        let duration_h = Decimal::from(duration_secs) / Decimal::from(3_600u32);
        if duration_h > Decimal::ZERO {
            let kw = iv.value_kwh / duration_h;
            entry.peak_kw = Some(entry.peak_kw.map_or(kw, |prev| prev.max(kw)));
        }

        // Quality: keep worst
        if quality_rank(iv.quality) > quality_rank(entry.quality) {
            entry.quality = iv.quality;
        }
    }

    buckets
        .into_values()
        .map(|mut b| {
            if b.interval_count < b.expected_count {
                b.has_missing_data = true;
            }
            b
        })
        .collect()
}

// ── helpers ───────────────────────────────────────────────────────────────────

fn quality_rank(q: QualityFlag) -> u8 {
    match q {
        QualityFlag::Faulty | QualityFlag::Unknown => 5,
        QualityFlag::Preliminary => 4,
        QualityFlag::Estimated => 3,
        QualityFlag::Corrected | QualityFlag::Substituted => 2,
        QualityFlag::Calculated => 1,
        QualityFlag::Measured => 0,
    }
}

/// The half-open bucket `[start, end)` containing `ts`.
///
/// Day, month and year buckets are Europe/Berlin calendar periods, so their
/// duration is whatever the calendar says — 23, 24 or 25 hours for a day; 28 to
/// 31 days ±1 hour for a month. Sub-daily buckets snap in UTC, which coincides
/// with local snapping because every Berlin offset is a whole hour.
fn bucket_bounds_for(
    ts: OffsetDateTime,
    res: IntervalResolution,
) -> (OffsetDateTime, OffsetDateTime) {
    match res {
        IntervalResolution::Day => {
            let day = calendar::local_day(ts);
            (calendar::day_start_utc(day), calendar::day_end_utc(day))
        }
        IntervalResolution::Month => {
            let day = calendar::local_day(ts);
            (calendar::month_start_utc(day), calendar::month_end_utc(day))
        }
        IntervalResolution::Year => {
            let year = calendar::local_year(ts);
            (calendar::year_start_utc(year), calendar::year_end_utc(year))
        }
        // Fixed-length resolutions: snap the Unix timestamp onto the grid.
        // `fixed_seconds` is `None` only for the calendar arms above and for
        // `Custom(0)`, which degenerates to a single bucket holding everything.
        fixed => {
            let Some(secs) = fixed.fixed_seconds() else {
                return (ts, ts);
            };
            let s = i64::from(secs);
            let snapped = ts.unix_timestamp().div_euclid(s) * s;
            let start = OffsetDateTime::from_unix_timestamp(snapped).unwrap_or(ts);
            (start, start + Duration::seconds(s))
        }
    }
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal::dec;
    use time::macros::{date, datetime};

    fn make_iv(from: OffsetDateTime, value_kwh: Decimal) -> MeterInterval {
        MeterInterval {
            from,
            to: from + Duration::minutes(15),
            value_kwh,
            quality: QualityFlag::Measured,
            obis_code: None,
        }
    }

    #[test]
    fn four_quarters_sum_to_one_hour() {
        let base = datetime!(2026-01-01 00:00 UTC);
        let ivs: Vec<_> = (0..4)
            .map(|i| make_iv(base + Duration::minutes(15 * i), dec!(2.5)))
            .collect();
        let result = resample(&ivs, &ResampleConfig::to_hourly());
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].total_kwh, dec!(10.0));
        assert_eq!(result[0].interval_count, 4);
        assert_eq!(result[0].expected_count, 4);
        assert!(result[0].is_complete());
    }

    #[test]
    fn gap_in_bucket_sets_has_missing_data() {
        let base = datetime!(2026-01-01 00:00 UTC);
        let ivs: Vec<_> =
            (0..3) // only 3 of 4 expected
                .map(|i| make_iv(base + Duration::minutes(15 * i), dec!(1.0)))
                .collect();
        let result = resample(&ivs, &ResampleConfig::to_hourly());
        assert_eq!(result.len(), 1);
        assert!(result[0].has_missing_data);
        assert!(!result[0].is_complete());
    }

    /// A German day runs 00:00–00:00 Berlin, so a full day's 96 intervals
    /// starting at 23:00 UTC (= 00:00 CET) form exactly one complete bucket.
    #[test]
    fn daily_aggregation_96_intervals() {
        let base = crate::calendar::day_start_utc(date!(2026 - 03 - 15));
        let ivs: Vec<_> = (0..96)
            .map(|i| make_iv(base + Duration::minutes(15 * i), dec!(1.0)))
            .collect();
        let result = resample(&ivs, &ResampleConfig::to_daily());
        assert_eq!(result.len(), 1, "one Berlin calendar day");
        assert_eq!(result[0].total_kwh, dec!(96.0));
        assert_eq!(result[0].expected_count, 96);
        assert!(result[0].is_complete());
    }

    /// The bug that motivated calendar bucketing: a UTC-day grouping would split
    /// these 96 intervals across two buckets and report both as incomplete.
    #[test]
    fn a_utc_day_is_not_a_german_day() {
        let base = datetime!(2026-03-15 00:00 UTC); // 01:00 CET — one hour late
        let ivs: Vec<_> = (0..96)
            .map(|i| make_iv(base + Duration::minutes(15 * i), dec!(1.0)))
            .collect();
        let result = resample(&ivs, &ResampleConfig::to_daily());
        assert_eq!(result.len(), 2, "a UTC day straddles two German days");
        assert_eq!(result[0].interval_count, 92, "01:00 CET to midnight");
        assert_eq!(result[1].interval_count, 4, "midnight to 01:00 CET");
        assert!(result.iter().all(|b| !b.is_complete()));
        assert_eq!(
            result[0].from,
            crate::calendar::day_start_utc(date!(2026 - 03 - 15))
        );
    }

    /// Spring forward: the day holds 92 quarter-hours, and a complete day must
    /// not be reported as short. Autumn: 100, and 96 must not read as complete.
    #[test]
    fn dst_days_expect_92_and_100_intervals() {
        for (day, expected) in [
            (date!(2026 - 03 - 29), 92u32),
            (date!(2026 - 10 - 25), 100u32),
        ] {
            let base = crate::calendar::day_start_utc(day);
            let full: Vec<_> = (0..i64::from(expected))
                .map(|i| make_iv(base + Duration::minutes(15 * i), dec!(1.0)))
                .collect();
            let result = resample(&full, &ResampleConfig::to_daily());
            assert_eq!(result.len(), 1, "{day}");
            assert_eq!(result[0].expected_count, expected, "{day} expected_count");
            assert!(result[0].is_complete(), "{day} must be complete");
            assert!(!result[0].has_missing_data, "{day}");
        }

        // 96 intervals on the 25-hour day is a four-interval gap, not a full day.
        let base = crate::calendar::day_start_utc(date!(2026 - 10 - 25));
        let short: Vec<_> = (0..96)
            .map(|i| make_iv(base + Duration::minutes(15 * i), dec!(1.0)))
            .collect();
        let result = resample(&short, &ResampleConfig::to_daily());
        assert!(
            result[0].has_missing_data,
            "96 of 100 intervals must not read as a complete autumn day"
        );
        assert_eq!(result[0].expected_count, 100);
    }

    /// Month boundaries are Berlin-local: 23:30 UTC on 31 January is already
    /// February, so all four intervals belong to the February bucket.
    #[test]
    fn month_buckets_follow_the_german_calendar() {
        let base = datetime!(2026-01-31 23:30 UTC); // 00:30 CET on 1 February
        let ivs: Vec<_> = (0..4)
            .map(|i| make_iv(base + Duration::minutes(15 * i), dec!(1.0)))
            .collect();
        let result = resample(&ivs, &ResampleConfig::to_monthly());
        assert_eq!(result.len(), 1, "all four intervals are February");
        assert_eq!(
            result[0].from,
            crate::calendar::month_start_utc(date!(2026 - 02 - 01))
        );
        assert_eq!(result[0].expected_count, 28 * 96);

        // One interval on either side of the real boundary splits the buckets.
        let straddling = vec![
            make_iv(datetime!(2026-01-31 22:45 UTC), dec!(1.0)), // 23:45 CET, January
            make_iv(datetime!(2026-01-31 23:00 UTC), dec!(1.0)), // 00:00 CET, February
        ];
        let result = resample(&straddling, &ResampleConfig::to_monthly());
        assert_eq!(result.len(), 2);
    }

    /// March 2026 is 2 972 quarter-hours, not 31 × 96 — the lost DST hour is
    /// four intervals the completeness check must not expect.
    #[test]
    fn month_expected_count_absorbs_the_dst_hour() {
        let march = resample(
            &[make_iv(datetime!(2026-03-15 12:00 UTC), dec!(1.0))],
            &ResampleConfig::to_monthly(),
        );
        assert_eq!(march[0].expected_count, 2_972);

        let october = resample(
            &[make_iv(datetime!(2026-10-15 12:00 UTC), dec!(1.0))],
            &ResampleConfig::to_monthly(),
        );
        assert_eq!(october[0].expected_count, 2_980);
    }

    #[test]
    fn yearly_buckets_account_for_leap_days() {
        let leap = resample(
            &[make_iv(datetime!(2028-06-15 12:00 UTC), dec!(1.0))],
            &ResampleConfig::to_yearly(),
        );
        assert_eq!(leap[0].expected_count, 366 * 96);
        assert_eq!(leap[0].from, crate::calendar::year_start_utc(2028));
    }

    #[test]
    fn peak_kw_is_maximum_across_intervals() {
        let base = datetime!(2026-06-01 10:00 UTC);
        let ivs = vec![
            make_iv(base, dec!(5.0)),                         // 5 kWh / 0.25 h = 20 kW
            make_iv(base + Duration::minutes(15), dec!(2.5)), // 2.5 / 0.25 = 10 kW
        ];
        let result = resample(&ivs, &ResampleConfig::to_hourly());
        assert_eq!(result[0].peak_kw, Some(dec!(20.0)));
    }

    #[test]
    fn worst_quality_propagates() {
        let base = datetime!(2026-01-01 00:00 UTC);
        let mut ivs: Vec<_> = (0..4)
            .map(|i| make_iv(base + Duration::minutes(15 * i), dec!(1.0)))
            .collect();
        ivs[2].quality = QualityFlag::Estimated;
        let result = resample(&ivs, &ResampleConfig::to_hourly());
        assert_eq!(result[0].quality, QualityFlag::Estimated);
    }

    #[test]
    fn empty_input_returns_empty() {
        assert!(resample(&[], &ResampleConfig::to_hourly()).is_empty());
    }

    #[test]
    fn coverage_pct_partial_bucket() {
        let base = datetime!(2026-01-01 00:00 UTC);
        let ivs = vec![make_iv(base, dec!(1.0))]; // 1 of 4
        let result = resample(&ivs, &ResampleConfig::to_hourly());
        assert!((result[0].coverage_pct() - 25.0).abs() < 0.01);
    }
}
