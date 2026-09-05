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
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::decimal"))]
    pub arbeitsmenge: Decimal,

    /// Spitzenleistung in kW: the highest average power over any single
    /// billable interval, `max(kWh / duration_h)`.
    ///
    /// `None` when the config disables it or nothing billable was supplied.
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::decimal_option"))]
    pub spitzenleistung_kw: Option<Decimal>,

    /// The interval the Spitzenleistung was **first** reached in.
    ///
    /// The Leistungspreis is the single most disputed line on an RLM invoice
    /// and "48 kW" is not an answer to "when?". `None` whenever
    /// [`spitzenleistung_kw`](Self::spitzenleistung_kw) is.
    ///
    /// A flat load reaches its maximum in many intervals, so the tie is broken
    /// by the **earliest** `from` rather than by whichever the caller listed
    /// first — see [`aggregate`].
    ///
    /// Only comparable across one resolution — see
    /// [`uniform_resolution`](Self::uniform_resolution).
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::rfc3339_option"))]
    pub spitzenleistung_at: Option<OffsetDateTime>,

    /// Number of intervals that contributed to the Arbeitsmenge.
    ///
    /// Billable intervals only, so it always matches the sum.
    pub billable_count: usize,

    /// Number of intervals supplied but excluded as non-billable.
    pub excluded_count: usize,

    /// `true` when every billable interval was the same length.
    ///
    /// The Jahreshöchstleistung of § 17 Abs. 2 StromNEV is defined on the
    /// metered Viertelstunde, and an average power over an hour is not
    /// comparable with one over a quarter-hour: the hour has already averaged
    /// away the peak the quarter-hour would have shown. A maximum taken over a
    /// series that mixes the two is not a Spitzenleistung of anything.
    ///
    /// This crate does not guess which resolution was meant, and it does not
    /// silently drop the answer either — it reports that the question was
    /// mixed, so a caller can refuse the figure, resample first, or record the
    /// caveat. `false` here makes
    /// [`spitzenleistung_kw`](Self::spitzenleistung_kw) an upper bound rather
    /// than a measurement.
    ///
    /// Vacuously `true` for a series with fewer than two billable intervals.
    pub uniform_resolution: bool,

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
    ///
    /// Only **billable** intervals contribute, because this figure answers
    /// *"can this period be invoiced"*.
    /// [`QualityReport::coverage_pct`](crate::QualityReport::coverage_pct)
    /// counts every delivered interval, because it answers *"did the data
    /// arrive"*. A day of `Faulty` quarter-hours is 100 % covered there and
    /// 0 % here, and that divergence is the point rather than a discrepancy.
    pub coverage_pct: f64,
}

/// Decimal places [`BillingPeriod::benutzungsdauer_h`] is cut to: **2**.
///
/// The quotient `kWh ÷ kW` rarely terminates, and the number it produces is
/// read against a threshold — Anlage 4 zu § 17 Abs. 2 StromNEV puts the kink of
/// the Gleichzeitigkeitsgrad at 2 500 Stunden, and Netzentgelt price sheets
/// split on the same figure. A hundredth of an hour is thirty-six seconds:
/// finer than any published threshold, coarse enough to be a number on a page.
pub const BENUTZUNGSDAUER_DP: u32 = 2;

/// Aggregate meter intervals into a [`BillingPeriod`].
///
/// **Order-independent, and no sort happens.** Every quantity here — a sum, a
/// maximum, two counts, a covered duration — is order-independent by
/// construction, so shuffled input gives an identical result in a single pass
/// with no allocation.
///
/// A maximum can be reached in several intervals, so
/// [`BillingPeriod::spitzenleistung_at`] breaks the tie by the **earliest**
/// interval start: a flat load reports the first quarter-hour it hit its peak
/// in, whatever order the series arrived in.
///
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
    let mut length: Option<i64> = None;
    let mut uniform_resolution = true;

    for iv in intervals {
        if !iv.quality.is_billable() {
            excluded_count += 1;
            continue;
        }
        billable_count += 1;
        arbeitsmenge += iv.value;
        let secs = iv.duration_secs();
        covered_secs += secs.max(0);
        match length {
            Some(first) if first != secs => uniform_resolution = false,
            Some(_) => {}
            None => length = Some(secs),
        }
        earliest = Some(earliest.map_or(iv.from, |e: OffsetDateTime| e.min(iv.from)));
        latest = Some(latest.map_or(iv.to, |l: OffsetDateTime| l.max(iv.to)));

        // A higher power wins; an equal one wins only if it happened earlier,
        // so the answer does not depend on the order the slice arrived in.
        if config.include_spitzenleistung
            && let Some(kw) = iv.demand_kw()
            && peak.is_none_or(|(best, at)| kw > best || (kw == best && iv.from < at))
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
        uniform_resolution,
        coverage_pct,
    }
}

// ── directional balance ───────────────────────────────────────────────────────

/// The energy that crossed a measurement point, split by
/// [`Direction`](crate::Direction).
///
/// Three buckets, not two: an interval whose OBIS code has no direction — a
/// reactive register, a gas volume, or no code at all — is counted in
/// [`undirected`](Self::undirected) rather than silently dropped, so
/// `import + export + undirected` is always the plain sum of the input and no
/// energy disappears between the call and the result.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct DirectionalEnergy {
    /// Bezug — the sum over intervals whose code counts C = 1.
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::decimal"))]
    pub import: Decimal,
    /// Einspeisung — the sum over intervals whose code counts C = 2.
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::decimal"))]
    pub export: Decimal,
    /// Everything with no direction to read: no OBIS code, or a code that
    /// counts something other than a directed active energy.
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::decimal"))]
    pub undirected: Decimal,
}

impl BillingPeriod {
    /// Benutzungsstundenzahl — `Arbeitsmenge ÷ Spitzenleistung`, in hours.
    ///
    /// § 17 Abs. 1 StromNEV makes the Netzentgelt depend on *"der jeweiligen
    /// **Benutzungsstundenzahl** der Entnahmestelle"*, and Anlage 4 zu § 17
    /// Abs. 2 builds the Gleichzeitigkeitsgrad on the Jahresbenutzungsdauer,
    /// with its two straight lines meeting *"durch die Jahresbenutzungsdauer
    /// 2 500 Stunden"* and reaching 1 at 8 760 Stunden. It is the figure a
    /// price sheet's two tariff bands are separated by, so it decides which
    /// column an RLM Entnahmestelle is billed from — while itself being a pure
    /// quantity, which is why it is here and its price is not.
    ///
    /// Cut to [`BENUTZUNGSDAUER_DP`] places.
    ///
    /// `None` when there is no Spitzenleistung to divide by — the config
    /// switched it off, nothing billable arrived, or the peak is zero. A
    /// **year** is the period the 2 500 h threshold is stated for; over a month
    /// the same arithmetic answers a different question, and over a series that
    /// mixes resolutions ([`uniform_resolution`](Self::uniform_resolution)) it
    /// answers none.
    ///
    /// ```rust
    /// use metering::{AggregationConfig, MeterInterval, QualityFlag, aggregate};
    /// use rust_decimal::dec;
    /// use time::{Duration, macros::datetime};
    ///
    /// // A flat 4 kW draw for a day: 96 kWh against a 4 kW peak is 24 hours.
    /// let day: Vec<MeterInterval> = (0..96).map(|i| MeterInterval {
    ///     from: datetime!(2026-06-01 0:00 UTC) + Duration::minutes(15 * i),
    ///     to:   datetime!(2026-06-01 0:00 UTC) + Duration::minutes(15 * i + 15),
    ///     value: dec!(1),
    ///     quality: QualityFlag::Measured,
    ///     obis_code: None,
    /// }).collect();
    ///
    /// let period = aggregate(&day, &AggregationConfig::rlm());
    /// assert_eq!(period.spitzenleistung_kw, Some(dec!(4)));
    /// assert_eq!(period.benutzungsdauer_h(), Some(dec!(24)));
    /// ```
    #[must_use]
    pub fn benutzungsdauer_h(&self) -> Option<Decimal> {
        let peak = self.spitzenleistung_kw?;
        if peak <= Decimal::ZERO {
            return None;
        }
        Some((self.arbeitsmenge / peak).round_dp(BENUTZUNGSDAUER_DP))
    }
}

impl DirectionalEnergy {
    /// `import − export` — the net flow across the point.
    ///
    /// Positive when the point drew more than it fed back. Excludes
    /// [`undirected`](Self::undirected), which by definition has no side to
    /// fall on.
    #[must_use]
    pub fn net(&self) -> Decimal {
        self.import - self.export
    }

    /// `import + export + undirected` — the plain sum of everything counted.
    #[must_use]
    pub fn total(&self) -> Decimal {
        self.import + self.export + self.undirected
    }
}

/// Sum a series by flow direction.
///
/// The conservation check for a bidirectional measurement point. A charge
/// point that supports V2G, a battery, a PV roof behind the grid meter — each
/// delivers a Bezug *and* an Einspeisung series for the same quarter-hour, and
/// an allocation of that point's energy is only correct if both sides balance:
///
/// ```rust
/// use metering::{MeterInterval, aggregation::sum_by_direction};
/// use rust_decimal::dec;
/// use time::macros::datetime;
///
/// let iv = |code: &str, kwh| MeterInterval::quarter_hour(datetime!(2026-06-01 12:00 UTC), kwh)
///     .with_obis(code.parse().unwrap());
///
/// let grid = [iv("1-0:1.8.0", dec!(9)), iv("1-0:2.8.0", dec!(4))];
/// let allocated = [
///     iv("1-0:1.8.0", dec!(5)), iv("1-0:1.8.0", dec!(4)),   // two sessions
///     iv("1-0:2.8.0", dec!(4)),                             // one discharge
/// ];
///
/// let measured = sum_by_direction(&grid);
/// let split = sum_by_direction(&allocated);
///
/// assert_eq!(measured.import - split.import, dec!(0));
/// assert_eq!(measured.export - split.export, dec!(0));
/// assert_eq!(measured.net(), dec!(5));
/// ```
///
/// ## Every interval counts, billable or not
///
/// [`aggregate`] sums only the billable ones, because it answers *"can this
/// period be invoiced"*. This is a **physical** balance — an allocation that
/// drops a `Faulty` quarter-hour has still lost that energy — so it counts
/// everything it is given. The divergence is the same one
/// [`BillingPeriod::coverage_pct`] documents against
/// [`QualityReport::coverage_pct`](crate::QualityReport::coverage_pct).
///
/// Order-independent: three sums, one pass, no allocation.
#[must_use]
pub fn sum_by_direction(intervals: &[MeterInterval]) -> DirectionalEnergy {
    let mut out = DirectionalEnergy::default();
    for iv in intervals {
        match iv.direction() {
            Some(crate::interval::Direction::Import) => out.import += iv.value,
            Some(crate::interval::Direction::Export) => out.export += iv.value,
            None => out.undirected += iv.value,
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interval::QualityFlag;
    use rust_decimal::dec;
    use time::OffsetDateTime;
    use time::macros::datetime;

    use time::Duration;

    /// A series that mixes an hour with a quarter-hour has no single
    /// Spitzenleistung, and the result says so instead of quietly reporting the
    /// larger of two incomparable numbers.
    #[test]
    fn a_mixed_resolution_series_is_reported_as_mixed() {
        let base = datetime!(2026-06-01 0:00 UTC);
        let quarter = MeterInterval {
            from: base,
            to: base + Duration::minutes(15),
            value: dec!(1),
            quality: QualityFlag::Measured,
            obis_code: None,
        };
        let hour = MeterInterval {
            from: base + Duration::minutes(15),
            to: base + Duration::minutes(75),
            value: dec!(2),
            quality: QualityFlag::Measured,
            obis_code: None,
        };

        let uniform = aggregate(std::slice::from_ref(&quarter), &AggregationConfig::rlm());
        assert!(uniform.uniform_resolution);

        let mixed = aggregate(&[quarter, hour], &AggregationConfig::rlm());
        assert!(!mixed.uniform_resolution);
        // The peak is still reported — as an upper bound the caller now knows
        // to qualify: 4 kW over a quarter-hour against 2 kW over an hour.
        assert_eq!(mixed.spitzenleistung_kw, Some(dec!(4)));
        assert_eq!(mixed.benutzungsdauer_h(), Some(dec!(0.75)));
    }

    /// Nothing billable means no peak, so no utilisation hours either — rather
    /// than a division by zero or a plausible-looking zero.
    #[test]
    fn utilisation_hours_need_a_peak() {
        let base = datetime!(2026-06-01 0:00 UTC);
        let faulty = MeterInterval {
            from: base,
            to: base + Duration::minutes(15),
            value: dec!(1),
            quality: QualityFlag::Faulty,
            obis_code: None,
        };
        let period = aggregate(&[faulty], &AggregationConfig::rlm());
        assert_eq!(period.spitzenleistung_kw, None);
        assert_eq!(period.benutzungsdauer_h(), None);
        assert!(
            period.uniform_resolution,
            "vacuously, with nothing billable"
        );
    }

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

#[cfg(test)]
mod order_tests {
    use super::*;
    use crate::interval::QualityFlag;
    use rust_decimal::dec;
    use time::macros::datetime;

    /// The documented property, asserted rather than asserted-in-prose: no sort
    /// is performed because none is needed.
    #[test]
    fn aggregation_is_order_independent() {
        let base = datetime!(2026-01-01 0:00 UTC);
        let ordered: Vec<MeterInterval> = [dec!(2.5), dec!(5.0), dec!(1.0), dec!(3.25)]
            .into_iter()
            .enumerate()
            .map(|(i, value)| MeterInterval {
                from: base + time::Duration::minutes(i as i64 * 15),
                to: base + time::Duration::minutes(i as i64 * 15 + 15),
                value,
                quality: QualityFlag::Measured,
                obis_code: None,
            })
            .collect();
        let mut shuffled = ordered.clone();
        shuffled.reverse();

        let a = aggregate(&ordered, &AggregationConfig::rlm());
        let b = aggregate(&shuffled, &AggregationConfig::rlm());
        assert_eq!(a, b);
        assert_eq!(
            a.spitzenleistung_at,
            Some(base + time::Duration::minutes(15))
        );
    }

    /// A flat load reaches its maximum in every interval, and
    /// `spitzenleistung_at` is the one quantity here a tie can make depend on
    /// the slice order.
    #[test]
    fn a_tied_peak_reports_the_earliest_interval() {
        let base = datetime!(2026-01-01 0:00 UTC);
        let flat: Vec<MeterInterval> = (0..4)
            .map(|i| MeterInterval {
                from: base + time::Duration::minutes(i * 15),
                to: base + time::Duration::minutes(i * 15 + 15),
                value: dec!(5),
                quality: QualityFlag::Measured,
                obis_code: None,
            })
            .collect();
        let mut reversed = flat.clone();
        reversed.reverse();

        let forward = aggregate(&flat, &AggregationConfig::rlm());
        let backward = aggregate(&reversed, &AggregationConfig::rlm());
        assert_eq!(forward, backward);
        assert_eq!(
            forward.spitzenleistung_at,
            Some(base),
            "the first quarter-hour the peak was reached in, not the first listed"
        );
        assert_eq!(forward.spitzenleistung_kw, Some(dec!(20)));
    }
}
