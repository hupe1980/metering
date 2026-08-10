//! Billing period aggregation: `arbeitsmenge`, `spitzenleistung_kw`, HT/NT split.
//!
//! ## Legal basis
//!
//! - **§ 17 Abs. 2 StromNEV** — the Jahresleistungspreissystem: *"Das
//!   Jahresleistungsentgelt ist das Produkt aus dem jeweiligen
//!   Jahresleistungspreis und der **Jahreshöchstleistung** in Kilowatt der
//!   jeweiligen Entnahme im Abrechnungsjahr."* That maximum is what
//!   [`BillingPeriod::spitzenleistung_kw`] computes.
//! - **§ 2 MsbG** — registrierende Lastgangmessung, the metering that makes a
//!   Viertelstundenleistung available at all.
//!
//! Earlier releases cited *§ 12 StromNZV* for the Spitzenleistung. That
//! provision is *"Standardisierte Lastprofile; Zählerstandsgangmessung"* — it
//! is about SLP, says nothing about peak demand, and was in any case repealed
//! with effect from the end of 31 December 2025.
//!
//! ## Spitzenleistung (peak demand)
//!
//! ```text
//! spitzenleistung_kw = max(interval_kwh / interval_duration_h)
//! ```
//!
//! For a 15-minute interval that is `kWh × 4`. The maximum is taken over
//! **billable** intervals only: a Faulty reading must not set the
//! Leistungspreis for a year.
//!
//! Mixing resolutions in one call makes the result meaningless — an hourly
//! interval's average power is not comparable to a quarter-hour's, and the
//! Jahreshöchstleistung is defined on the metered Viertelstunde. Aggregate one
//! resolution at a time.
//!
//! ## Tariff registers are not computed here
//!
//! `aggregate` returns one Arbeitsmenge. Splitting it across HT/NT, § 14a
//! Modul 3's three bands or any other Zählzeitdefinition is
//! [`Zaehlzeitdefinition::split_energy`](crate::Zaehlzeitdefinition::split_energy),
//! and the two compose:
//!
//! ```rust
//! # use metering::{AggregationConfig, MeterInterval, QualityFlag, Zaehlzeitdefinition, aggregate};
//! # use rust_decimal::dec;
//! # use time::macros::{date, datetime};
//! # let intervals: Vec<MeterInterval> = (0..96).map(|i| MeterInterval {
//! #     from: datetime!(2026-01-04 23:00 UTC) + time::Duration::minutes(i * 15),
//! #     to:   datetime!(2026-01-04 23:00 UTC) + time::Duration::minutes(i * 15 + 15),
//! #     value: dec!(2.5), quality: QualityFlag::Measured, obis_code: None }).collect();
//! let zzd = Zaehlzeitdefinition::modul_3(
//!     "NB-14A-3", date!(2026 - 01 - 01), (17 * 60, 20 * 60), (0, 6 * 60),
//! );
//!
//! let period = aggregate(&intervals, &AggregationConfig::rlm());
//! let registers = zzd.split_energy(&intervals);
//!
//! // The split reconstructs the total, always.
//! assert_eq!(registers.values().sum::<rust_decimal::Decimal>(), period.arbeitsmenge);
//! ```
//!
//! Earlier releases put a two-register `ht_nt` split on [`BillingPeriod`],
//! configured by an `include_ht_nt` flag and a `TariffWindow`. It could not
//! express Modul 3, which has three bands and which every Netzbetreiber has had
//! to offer since 1 April 2025 — so the split moved to the one mechanism that
//! can express any number of them.

use rust_decimal::Decimal;
use time::OffsetDateTime;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use crate::interval::MeterInterval;

/// Configuration for billing period aggregation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AggregationConfig {
    /// Compute the Spitzenleistung. Only meaningful for interval metering.
    pub include_spitzenleistung: bool,
    /// The period the series is supposed to cover, as a half-open UTC range.
    ///
    /// Sets what [`BillingPeriod::coverage_pct`] is measured against. Without
    /// it, coverage is measured against the extent of the data itself, which
    /// can only ever detect *interior* gaps — a month whose last week never
    /// arrived reports 100 %.
    pub period: Option<(OffsetDateTime, OffsetDateTime)>,
}

impl AggregationConfig {
    /// Arbeitsmenge **and** Spitzenleistung — interval metering (RLM, iMSys).
    ///
    /// Take the peak only where the series is an equidistant interval series of
    /// one resolution. The Jahreshöchstleistung of § 17 Abs. 2 StromNEV is
    /// defined on the metered Viertelstunde, and the maximum over a series that
    /// mixes quarter-hours with hours compares quantities that are not
    /// comparable.
    #[must_use]
    pub const fn rlm() -> Self {
        Self {
            include_spitzenleistung: true,
            period: None,
        }
    }

    /// Arbeitsmenge only — SLP, gas, and anything else with no Leistungspreis.
    ///
    /// This replaces the former `slp_strom()` and `gas()`, which had become
    /// byte-identical once the HT/NT knobs moved to
    /// [`Zaehlzeitdefinition`](crate::Zaehlzeitdefinition). Two constructors
    /// returning the same value document an intent the type does not act on.
    #[must_use]
    pub const fn arbeitsmenge_only() -> Self {
        Self {
            include_spitzenleistung: false,
            period: None,
        }
    }

    /// Declare the period the series must cover (builder style).
    #[must_use]
    pub const fn over_period(mut self, from: OffsetDateTime, to: OffsetDateTime) -> Self {
        self.period = Some((from, to));
        self
    }
}

impl Default for AggregationConfig {
    fn default() -> Self {
        Self::rlm()
    }
}

/// Result of a billing period aggregation.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct BillingPeriod {
    /// Arbeitsmenge in kWh — the sum of the **billable** intervals.
    pub arbeitsmenge: Decimal,

    /// Spitzenleistung in kW: the highest average power over any single
    /// billable interval, `max(kWh / duration_h)`.
    ///
    /// `None` when the config disables it or nothing billable was supplied.
    pub spitzenleistung_kw: Option<Decimal>,

    /// The interval the Spitzenleistung was reached in.
    ///
    /// The Leistungspreis is the single most disputed line on an RLM invoice
    /// and "48 kW" is not an answer to "when?". `None` whenever
    /// [`spitzenleistung_kw`](Self::spitzenleistung_kw) is.
    pub spitzenleistung_at: Option<OffsetDateTime>,

    /// Number of intervals that contributed to the Arbeitsmenge.
    ///
    /// Billable intervals only, so it always matches the sum. It previously
    /// counted every supplied interval, including the `Faulty` ones that were
    /// excluded from the total.
    pub billable_count: usize,

    /// Number of intervals supplied but excluded as non-billable.
    pub excluded_count: usize,

    /// Share of the period covered by billable intervals, 0–100.
    ///
    /// A **duration** ratio — covered seconds over period seconds — not a count
    /// ratio. A count needs an expected count, and the expected count for a
    /// German day is 92, 96 or 100 depending on the date; a duration ratio is
    /// right at every resolution and across both DST transitions without being
    /// told which day it is.
    ///
    /// Measured against [`AggregationConfig::period`] when set, and against the
    /// extent of the data otherwise — in which case it can only detect interior
    /// gaps. See that field.
    pub coverage_pct: f64,
}

/// Aggregate meter intervals into a [`BillingPeriod`].
///
/// Intervals are sorted by `from` internally.
/// Only intervals where `quality.is_billable()` contribute to the result.
///
/// # Example
/// ```rust
/// use metering::{MeterInterval, QualityFlag, aggregate, AggregationConfig};
/// use rust_decimal::Decimal;
/// use time::macros::datetime;
///
/// let iv = MeterInterval {
///     from: datetime!(2026-06-01 0:00 UTC),
///     to:   datetime!(2026-06-01 0:15 UTC),
///     value: Decimal::from_str_exact("2.5").unwrap(),
///     quality: QualityFlag::Measured,
///     obis_code: None,
/// };
/// let period = aggregate(&[iv], &AggregationConfig::rlm());
/// assert_eq!(period.arbeitsmenge, Decimal::from_str_exact("2.5").unwrap());
/// assert_eq!(period.spitzenleistung_kw, Some(Decimal::from(10u32))); // 2.5 × 4 = 10 kW
/// ```
#[must_use]
pub fn aggregate(intervals: &[MeterInterval], config: &AggregationConfig) -> BillingPeriod {
    // Only billable intervals contribute to anything. A single pass, no clone
    // and no sort: every quantity below is order-independent.
    let mut arbeitsmenge = Decimal::ZERO;
    let mut billable_count = 0usize;
    let mut excluded_count = 0usize;
    let mut peak: Option<(Decimal, OffsetDateTime)> = None;
    let mut covered_secs = 0i64;
    let mut earliest: Option<OffsetDateTime> = None;
    let mut latest: Option<OffsetDateTime> = None;

    for iv in intervals {
        if !iv.quality.is_billable() {
            excluded_count += 1;
            continue;
        }
        billable_count += 1;
        arbeitsmenge += iv.value;
        covered_secs += iv.duration_secs().max(0);
        earliest = Some(earliest.map_or(iv.from, |e: OffsetDateTime| e.min(iv.from)));
        latest = Some(latest.map_or(iv.to, |l: OffsetDateTime| l.max(iv.to)));

        if config.include_spitzenleistung
            && let Some(kw) = iv.demand_kw()
            && peak.is_none_or(|(best, _)| kw > best)
        {
            peak = Some((kw, iv.from));
        }
    }

    let period_secs = match config.period {
        Some((from, to)) => (to - from).whole_seconds(),
        None => match (earliest, latest) {
            (Some(f), Some(l)) => (l - f).whole_seconds(),
            _ => 0,
        },
    };
    let coverage_pct = if period_secs <= 0 {
        if covered_secs > 0 { 100.0 } else { 0.0 }
    } else {
        ((covered_secs as f64 / period_secs as f64) * 100.0).clamp(0.0, 100.0)
    };

    BillingPeriod {
        arbeitsmenge,
        spitzenleistung_kw: peak.map(|(kw, _)| kw),
        spitzenleistung_at: peak.map(|(_, at)| at),
        billable_count,
        excluded_count,
        coverage_pct,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interval::QualityFlag;
    use rust_decimal::dec;
    use time::OffsetDateTime;
    use time::macros::datetime;

    fn iv(from: OffsetDateTime, kwh: Decimal) -> MeterInterval {
        MeterInterval {
            from,
            to: from + time::Duration::minutes(15),
            value: kwh,
            quality: QualityFlag::Measured,
            obis_code: None,
        }
    }

    #[test]
    fn spitzenleistung_max_15min() {
        let base = datetime!(2026-01-01 0:00 UTC);
        let intervals = vec![
            iv(base, dec!(2.5)),                               // 10 kW
            iv(base + time::Duration::minutes(15), dec!(5.0)), // 20 kW — peak
            iv(base + time::Duration::minutes(30), dec!(1.0)), // 4 kW
        ];
        let period = aggregate(&intervals, &AggregationConfig::rlm());
        assert_eq!(period.spitzenleistung_kw, Some(dec!(20)));
    }

    #[test]
    fn arbeitsmenge_sum_only_billable() {
        let base = datetime!(2026-01-01 0:00 UTC);
        let mut intervals = vec![
            iv(base, dec!(2.0)),
            iv(base + time::Duration::minutes(15), dec!(3.0)),
        ];
        // Estimated is billable: a Prognosewert is what an Abschlag rests on.
        intervals[1].quality = QualityFlag::Estimated;
        let period = aggregate(&intervals, &AggregationConfig::rlm());
        // Both intervals contribute: 2.0 + 3.0 = 5.0 kWh
        assert_eq!(period.arbeitsmenge, dec!(5.0));

        // Only Faulty and Unknown are excluded
        let mut faulty_intervals = vec![
            iv(base, dec!(2.0)),
            iv(base + time::Duration::minutes(15), dec!(3.0)),
        ];
        faulty_intervals[1].quality = QualityFlag::Faulty;
        let faulty_period = aggregate(&faulty_intervals, &AggregationConfig::rlm());
        // Faulty interval excluded: only 2.0 kWh
        assert_eq!(faulty_period.arbeitsmenge, dec!(2.0));
    }

    #[test]
    fn slp_no_spitzenleistung() {
        let base = datetime!(2026-01-01 0:00 UTC);
        let intervals = vec![iv(base, dec!(24.0))]; // daily SLP read
        let period = aggregate(&intervals, &AggregationConfig::arbeitsmenge_only());
        assert_eq!(period.spitzenleistung_kw, None);
        assert_eq!(period.arbeitsmenge, dec!(24.0));
    }

    #[test]
    fn empty_intervals() {
        let period = aggregate(&[], &AggregationConfig::rlm());
        assert_eq!(period.arbeitsmenge, Decimal::ZERO);
        assert_eq!(period.spitzenleistung_kw, None);
        assert_eq!(period.billable_count, 0);
        assert_eq!(period.coverage_pct, 0.0);
    }

    #[test]
    fn spitzenleistung_mess_zv_definition() {
        // § 17 Abs. 2 StromNEV: the Jahreshöchstleistung is the highest
        // 3.5 kWh in 15 min = 14 kW
        // 1.0 kWh in 15 min = 4 kW
        // Peak = 14 kW
        let base = datetime!(2026-06-01 10:00 UTC);
        let intervals = vec![
            iv(base, dec!(3.5)),
            iv(base + time::Duration::minutes(15), dec!(1.0)),
        ];
        let period = aggregate(&intervals, &AggregationConfig::rlm());
        assert_eq!(period.spitzenleistung_kw, Some(dec!(14)));
    }
}
