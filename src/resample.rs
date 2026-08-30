//! Time-series resampling — down-sample high-resolution intervals to coarser buckets.
//!
//! ## Use cases
//!
//! | Use case | Target resolution |
//! |---|---|
//! | API summaries (client dashboards) | Hourly or daily |
//! | Jahresmehr-/-mindermengen (GPKE Kap. 8.4) | Yearly |
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
//! every month, into the preceding period — a systematic error on every daily
//! and monthly figure the market exchanges.
//!
//! Sub-daily buckets (quarter-hour, half-hour, hour, `Custom`) are snapped in
//! UTC, which is equivalent: every Europe/Berlin offset is a whole number of
//! hours, so local and UTC boundaries coincide at that granularity.
//!
//! `expected_count` follows from the bucket's real duration, so the
//! spring-forward day expects **92** quarter-hours and the fall-back day
//! **100** — not a flat 96 that would hide four missing intervals every autumn.
//!
//! Day, month and year buckets are cut at **00:00 by default and at 06:00 on
//! request** — [`ResampleConfig::on`] with
//! [`crate::calendar::DayBoundary::Gastag`]. The German
//! gas market balances on the Gastag, so summing a gas Lastgang over the
//! calendar day books the 00:00–06:00 draw into the wrong Bilanzierungstag,
//! every day.
//!
//! `from` and `to` on a bucket remain UTC instants; convert with
//! [`crate::calendar::to_berlin`] for display.
//!
//! ## Regulatory basis
//!
//! - **§ 2 MsbG** — RLM, the 15-minute interval metering these buckets start from.
//! - **GPKE Kap. 8.4** (BNetzA **BK6-24-174**) — Jahresmehr- und
//!   Jahresmindermengen, settled **annually**, not monthly. The older citation
//!   *§ 13 StromNZV* is dead: repealed with effect from the end of
//!   31 December 2025.

use std::collections::BTreeMap;

use rust_decimal::Decimal;
use time::OffsetDateTime;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use crate::calendar::DayBoundary;
use crate::interval::{MeterInterval, QualityFlag};
use crate::resolution::IntervalResolution;

// ── ResampledBucket ───────────────────────────────────────────────────────────

/// A resampled bucket: one or more source intervals aggregated into a coarser window.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct ResampledBucket {
    /// Bucket start (UTC, inclusive).
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::rfc3339"))]
    pub from: OffsetDateTime,
    /// Bucket end (UTC, exclusive).
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::rfc3339"))]
    pub to: OffsetDateTime,
    /// Sum of all `value` from contributing intervals.
    pub total: Decimal,
    /// Peak demand in kW across contributing intervals.
    ///
    /// Computed as `max(interval.value / interval_duration_h)`.
    /// `None` only when no intervals contributed (should not normally occur).
    ///
    /// Meaningful only where the interval unit is an **energy** — the same
    /// caveat as [`MeterInterval::demand_kw`](crate::MeterInterval::demand_kw).
    /// On a `Sparte::Wasser` series, whose values are cubic metres, this is a
    /// flow rate in m³/h wearing a `_kw` name.
    pub peak_kw: Option<Decimal>,
    /// Number of intervals that contributed to this bucket.
    pub interval_count: u32,
    /// How many intervals a complete bucket holds, when that is knowable.
    ///
    /// `None` when [`ResampleConfig::source_resolution`] is a calendar period,
    /// which has no fixed count to divide by — not `0`, which would make
    /// [`coverage_pct`](Self::coverage_pct) report 100 % and
    /// [`is_complete`](Self::is_complete) `true` for a bucket nobody can
    /// assess.
    pub expected_count: Option<u32>,
    /// Worst quality flag among all contributing intervals.
    pub quality: QualityFlag,
}

impl ResampledBucket {
    /// Coverage as a percentage, or `None` when the expected count is unknown.
    ///
    /// Not capped at 100: more intervals than expected means duplicates, and
    /// hiding that behind a cap would turn a data fault into a clean bucket.
    #[must_use]
    pub fn coverage_pct(&self) -> Option<f64> {
        let expected = self.expected_count?;
        if expected == 0 {
            return None;
        }
        Some(f64::from(self.interval_count) / f64::from(expected) * 100.0)
    }

    /// `true` when the bucket holds exactly the intervals it should, `false`
    /// when it does not, and `None` when that cannot be determined.
    ///
    /// Three states rather than two, because "unknown" is not "complete".
    #[must_use]
    pub fn is_complete(&self) -> Option<bool> {
        Some(self.interval_count == self.expected_count?)
    }

    /// `true` when the bucket is short of its expected count.
    ///
    /// `false` when it is complete, over-full, or unknown — use
    /// [`is_complete`](Self::is_complete) to tell the last case apart.
    #[must_use]
    pub fn has_missing_data(&self) -> bool {
        self.expected_count
            .is_some_and(|expected| self.interval_count < expected)
    }

    /// `true` when more intervals landed here than the bucket can hold —
    /// duplicates, or a source resolution finer than the one configured.
    #[must_use]
    pub fn has_surplus_data(&self) -> bool {
        self.expected_count
            .is_some_and(|expected| self.interval_count > expected)
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
    /// Used to derive [`ResampledBucket::expected_count`], so a calendar source
    /// resolution leaves that `None` — there is no fixed count to divide a
    /// bucket by. Default: [`IntervalResolution::QuarterHour`].
    pub source_resolution: IntervalResolution,
    /// Where a day, month and year are cut.
    ///
    /// [`DayBoundary::Midnight`] by default — the electricity market's
    /// Liefertag. [`DayBoundary::Gastag`] moves every daily, monthly and yearly
    /// bucket onto the 06:00 boundary the gas market balances on. Sub-daily
    /// buckets are unaffected: they snap on the UTC grid either way.
    pub day_boundary: DayBoundary,
}

impl ResampleConfig {
    /// Resample from `source` to `target`, on calendar days.
    #[must_use]
    pub const fn new(source: IntervalResolution, target: IntervalResolution) -> Self {
        Self {
            target_resolution: target,
            source_resolution: source,
            day_boundary: DayBoundary::Midnight,
        }
    }

    /// Cut days, months and years on `boundary` (builder style).
    ///
    /// ```rust
    /// use metering::{IntervalResolution, ResampleConfig, calendar::DayBoundary};
    ///
    /// // Daily gas totals per Gastag rather than per calendar day.
    /// let cfg = ResampleConfig::to_daily().on(DayBoundary::Gastag);
    /// assert_eq!(cfg.day_boundary, DayBoundary::Gastag);
    /// # let _ = IntervalResolution::Day;
    /// ```
    #[must_use]
    pub const fn on(mut self, boundary: DayBoundary) -> Self {
        self.day_boundary = boundary;
        self
    }

    /// Gas day totals — the 06:00-to-06:00 Gastag, from hourly gas intervals.
    ///
    /// Summing a gas Lastgang over the *calendar* day books the 00:00–06:00
    /// draw into the wrong Bilanzierungstag, every day.
    #[must_use]
    pub const fn to_gas_daily() -> Self {
        Self::new(IntervalResolution::Hour, IntervalResolution::Day).on(DayBoundary::Gastag)
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

    /// Berlin calendar month totals — monthly settlement and reporting.
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
/// Input intervals need not be contiguous or sorted: gaps show up as an
/// `interval_count` below [`ResampledBucket::expected_count`], and the output
/// is always ascending by `from`.
///
/// ## An interval is assigned by its start
///
/// A source interval that straddles a bucket boundary is booked **whole** into
/// the bucket it begins in; it is not split pro rata. Splitting would require
/// assuming the energy is spread evenly inside the interval, which is exactly
/// the assumption interval metering exists to avoid making.
///
/// At the resolutions this is used for the case does not arise — quarter-hours
/// nest inside hours, days and months without remainder. It arises for a
/// `Custom` source resolution that does not divide the target, and there the
/// caller should resample in two steps instead.
#[must_use]
pub fn resample(intervals: &[MeterInterval], config: &ResampleConfig) -> Vec<ResampledBucket> {
    if intervals.is_empty() {
        return Vec::new();
    }

    // BTreeMap: bucket_start_unix → ResampledBucket (sorted automatically)
    let mut buckets: BTreeMap<i64, ResampledBucket> = BTreeMap::new();

    for iv in intervals {
        let (bucket_start, bucket_end) = config
            .day_boundary
            .bucket_bounds(iv.from, config.target_resolution);

        let entry = buckets
            .entry(bucket_start.unix_timestamp())
            .or_insert_with(|| ResampledBucket {
                from: bucket_start,
                to: bucket_end,
                total: Decimal::ZERO,
                peak_kw: None,
                interval_count: 0,
                expected_count: crate::calendar::intervals_between(
                    bucket_start,
                    bucket_end,
                    config.source_resolution,
                ),
                quality: QualityFlag::Measured,
            });

        entry.total += iv.value;
        entry.interval_count += 1;

        // Peak demand = energy / duration_h
        let duration_secs = (iv.to - iv.from).whole_seconds().max(1);
        let duration_h = Decimal::from(duration_secs) / Decimal::from(3_600u32);
        if duration_h > Decimal::ZERO {
            let kw = iv.value / duration_h;
            entry.peak_kw = Some(entry.peak_kw.map_or(kw, |prev| prev.max(kw)));
        }

        // Quality: keep the worst contributor — see `QualityFlag::severity_rank`,
        // whose ranks are distinct so this fold is order-independent.
        entry.quality = entry.quality.worse_of(iv.quality);
    }

    buckets.into_values().collect()
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal::dec;
    use time::Duration;
    use time::macros::{date, datetime};

    fn make_iv(from: OffsetDateTime, value: Decimal) -> MeterInterval {
        MeterInterval {
            from,
            to: from + Duration::minutes(15),
            value,
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
        assert_eq!(result[0].total, dec!(10.0));
        assert_eq!(result[0].interval_count, 4);
        assert_eq!(result[0].expected_count, Some(4));
        assert_eq!(result[0].is_complete(), Some(true));
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
        assert!(result[0].has_missing_data());
        assert_eq!(result[0].is_complete(), Some(false));
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
        assert_eq!(result[0].total, dec!(96.0));
        assert_eq!(result[0].expected_count, Some(96));
        assert_eq!(result[0].is_complete(), Some(true));
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
        assert!(result.iter().all(|b| b.is_complete() == Some(false)));
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
            assert_eq!(
                result[0].expected_count,
                Some(expected),
                "{day} expected_count"
            );
            assert_eq!(
                result[0].is_complete(),
                Some(true),
                "{day} must be complete"
            );
            assert!(!result[0].has_missing_data(), "{day}");
        }

        // 96 intervals on the 25-hour day is a four-interval gap, not a full day.
        let base = crate::calendar::day_start_utc(date!(2026 - 10 - 25));
        let short: Vec<_> = (0..96)
            .map(|i| make_iv(base + Duration::minutes(15 * i), dec!(1.0)))
            .collect();
        let result = resample(&short, &ResampleConfig::to_daily());
        assert!(
            result[0].has_missing_data(),
            "96 of 100 intervals must not read as a complete autumn day"
        );
        assert_eq!(result[0].expected_count, Some(100));
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
        assert_eq!(result[0].expected_count, Some(28 * 96));

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
        assert_eq!(march[0].expected_count, Some(2_972));

        let october = resample(
            &[make_iv(datetime!(2026-10-15 12:00 UTC), dec!(1.0))],
            &ResampleConfig::to_monthly(),
        );
        assert_eq!(october[0].expected_count, Some(2_980));
    }

    #[test]
    fn yearly_buckets_account_for_leap_days() {
        let leap = resample(
            &[make_iv(datetime!(2028-06-15 12:00 UTC), dec!(1.0))],
            &ResampleConfig::to_yearly(),
        );
        assert_eq!(leap[0].expected_count, Some(366 * 96));
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

    /// "Unknown" must not read as "complete". A calendar source resolution has
    /// no fixed count, so the bucket cannot be assessed at all — and an
    /// unassessable bucket is not a perfect one.
    #[test]
    fn an_unknown_expected_count_is_not_a_complete_bucket() {
        let cfg = ResampleConfig::new(IntervalResolution::Day, IntervalResolution::Month);
        let daily = vec![MeterInterval {
            from: crate::calendar::day_start_utc(date!(2026 - 03 - 15)),
            to: crate::calendar::day_end_utc(date!(2026 - 03 - 15)),
            value: dec!(100),
            quality: QualityFlag::Measured,
            obis_code: None,
        }];
        let result = resample(&daily, &cfg);
        assert_eq!(result[0].expected_count, None);
        assert_eq!(result[0].is_complete(), None, "unknown, not complete");
        assert_eq!(result[0].coverage_pct(), None, "unknown, not 100 %");
        assert!(!result[0].has_missing_data());
        assert!(!result[0].has_surplus_data());
    }

    /// More intervals than a bucket can hold is a data fault — duplicates, or a
    /// finer source than configured — and is reported rather than capped away.
    #[test]
    fn a_surplus_is_visible_rather_than_capped() {
        let base = datetime!(2026-01-01 00:00 UTC);
        // Eight quarter-hours claiming to be in one hour: four are duplicates.
        let mut ivs: Vec<_> = (0..4)
            .map(|i| make_iv(base + Duration::minutes(15 * i), dec!(1.0)))
            .collect();
        ivs.extend(ivs.clone());

        let result = resample(&ivs, &ResampleConfig::to_hourly());
        assert_eq!(result[0].interval_count, 8);
        assert_eq!(result[0].expected_count, Some(4));
        assert!(result[0].has_surplus_data());
        assert!(!result[0].has_missing_data());
        assert_eq!(result[0].is_complete(), Some(false));
        assert!(
            result[0].coverage_pct().unwrap() > 100.0,
            "coverage is not capped: {:?}",
            result[0].coverage_pct()
        );
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
        assert!((result[0].coverage_pct().unwrap() - 25.0).abs() < 0.01);
    }
}

#[cfg(test)]
mod gas_day_tests {
    use super::*;
    use crate::calendar;
    use crate::interval::QualityFlag;
    use rust_decimal::dec;
    use time::Duration;
    use time::macros::{date, datetime};

    /// Hourly gas intervals over `n` hours from `start`, 1 kWh each.
    fn hourly(start: OffsetDateTime, n: i64) -> Vec<MeterInterval> {
        (0..n)
            .map(|i| {
                let from = start + Duration::hours(i);
                MeterInterval {
                    from,
                    to: from + Duration::hours(1),
                    value: dec!(1),
                    quality: QualityFlag::Measured,
                    obis_code: None,
                }
            })
            .collect()
    }

    /// The failure the boundary exists to prevent: the German gas market
    /// balances 06:00 to 06:00, so a calendar-day total books the first six
    /// hours of every Gastag into the previous one.
    #[test]
    fn gas_days_are_cut_at_0600_local() {
        // Two whole Gastage: 06:00 local on 15 January to 06:00 on the 17th.
        let start = calendar::gas_day_start_utc(date!(2026 - 01 - 15));
        let buckets = resample(&hourly(start, 48), &ResampleConfig::to_gas_daily());

        assert_eq!(buckets.len(), 2, "two gas days, not three");
        assert_eq!(buckets[0].from, datetime!(2026-01-15 5:00 UTC));
        assert_eq!(buckets[0].to, datetime!(2026-01-16 5:00 UTC));
        assert_eq!(buckets[0].total, dec!(24));
        assert_eq!(buckets[0].expected_count, Some(24));
        assert_eq!(buckets[0].is_complete(), Some(true));

        // The same data on calendar days splits into three partial buckets.
        let calendar_days = resample(
            &hourly(start, 48),
            &ResampleConfig::new(IntervalResolution::Hour, IntervalResolution::Day),
        );
        assert_eq!(calendar_days.len(), 3);
        assert!(calendar_days[0].has_missing_data(), "18:00–24:00 only");
    }

    /// A Gastag inherits the DST length of the day it *contains*, and the
    /// transition happens at 03:00 local — inside the gas day that began on
    /// Saturday, not the one named after the Sunday.
    #[test]
    fn the_long_gas_day_is_saturdays() {
        let start = calendar::gas_day_start_utc(date!(2026 - 10 - 24));
        let buckets = resample(&hourly(start, 49), &ResampleConfig::to_gas_daily());

        assert_eq!(
            buckets[0].from,
            calendar::gas_day_start_utc(date!(2026 - 10 - 24))
        );
        assert_eq!(
            buckets[0].expected_count,
            Some(25),
            "Saturday's Gastag holds the extra hour"
        );
        assert_eq!(buckets[0].interval_count, 25);
        assert_eq!(buckets[0].is_complete(), Some(true));
        assert_eq!(buckets[1].expected_count, Some(24), "Sunday's is ordinary");
    }

    /// The boundary carries up to months and years: a gas month is a whole
    /// number of Gastage, not a calendar month shifted six hours.
    #[test]
    fn gas_months_are_whole_gas_days() {
        let cfg = ResampleConfig::new(IntervalResolution::Hour, IntervalResolution::Month)
            .on(DayBoundary::Gastag);
        let start = calendar::gas_day_start_utc(date!(2026 - 01 - 31));
        let buckets = resample(&hourly(start, 48), &cfg);

        assert_eq!(buckets.len(), 2, "the month ends at 06:00 on 1 February");
        assert_eq!(
            buckets[0].to,
            calendar::gas_day_start_utc(date!(2026 - 02 - 01))
        );
        assert_eq!(
            buckets[1].from,
            calendar::gas_day_start_utc(date!(2026 - 02 - 01))
        );
    }
}
