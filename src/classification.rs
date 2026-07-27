//! SLP/RLM/iMSys classification and interval length detection.
//!
//! ## Legal basis
//!
//! - **§ 2 MsbG**: RLM = registrierende Lastgangmessung (15-min or 60-min intervals).
//! - **§ 12 StromNZV**: SLP = Standardlastprofil (daily, monthly, or annual totals).
//! - **§41a Abs. 2 EnWG**: iMSys (intelligente Messsysteme) require 15-min
//!   interval resolution for dynamic tariff billing.
//! - **BNetzA MaBiS-Beschluss BK6-12-200**: RLM threshold ≥ 100 000 kWh/year.
//!
//! ## Classification rules
//!
//! | Messtyp | Interval | Threshold |
//! |---|---|---|
//! | `Slp` | any (daily/monthly aggregates) | < 100 000 kWh/year |
//! | `Rlm` | 15 min or 60 min | ≥ 100 000 kWh/year |
//! | `IMsys` | 15 min (from SMGW direct push) | mandatory for §41a |

use crate::interval::MeterInterval;
use crate::resolution::IntervalResolution;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// Metering type (Messtyp).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "SCREAMING_SNAKE_CASE"))]
pub enum Messtyp {
    /// Standard load profile — daily or monthly aggregates.
    /// No 15-min resolution. § 12 StromNZV.
    Slp,
    /// Registrierende Lastgangmessung — 15-min or 60-min intervals.
    /// Required for ≥100 000 kWh/year. § 2 MsbG.
    Rlm,
    /// iMSys / SMGW direct push — always 15-min.
    /// Required for §41a EnWG dynamic tariff billing.
    IMsys,
}

impl Messtyp {
    /// `true` when the Messtyp supports Spitzenleistung (peak demand) billing.
    #[must_use]
    pub fn supports_spitzenleistung(&self) -> bool {
        matches!(self, Messtyp::Rlm | Messtyp::IMsys)
    }

    /// `true` when the Messtyp supports §41a EnWG dynamic tariff billing.
    #[must_use]
    pub fn supports_dynamic_tariff(&self) -> bool {
        matches!(self, Messtyp::IMsys)
    }
}

/// Detect the dominant interval length in a set of meter intervals.
///
/// Uses the median interval duration, so it is robust against gaps and against
/// the odd short or long interval at a DST transition.
///
/// The daily band is deliberately **23–25 hours**: a Berlin calendar day is 23
/// hours each spring and 25 each autumn, so a daily series legitimately contains
/// intervals of all three lengths. A median in that band is
/// [`IntervalResolution::Day`] — the calendar day — rather than a fixed
/// `Custom(86_400)` window.
///
/// # Returns
///
/// `None` when `intervals` is empty or every interval is zero-length.
#[must_use]
pub fn detect_interval_length(intervals: &[MeterInterval]) -> Option<IntervalResolution> {
    if intervals.is_empty() {
        return None;
    }
    let mut durations: Vec<i64> = intervals
        .iter()
        .map(|iv| iv.duration_secs())
        .filter(|&d| d > 0)
        .collect();
    if durations.is_empty() {
        return None;
    }
    durations.sort_unstable();
    let median = durations[durations.len() / 2];
    Some(match median {
        750..=1050 => IntervalResolution::QuarterHour, // 900 s ± 150 s
        1650..=1950 => IntervalResolution::HalfHour,   // 1800 s
        3300..=3900 => IntervalResolution::Hour,       // 3600 s ± 300 s
        // 23 h … 25 h — a Berlin calendar day at either DST transition.
        82_800..=90_000 => IntervalResolution::Day,
        other => IntervalResolution::Custom(u32::try_from(other).unwrap_or(u32::MAX)),
    })
}

/// Classify the metering type based on interval length and source hint.
///
/// - `source = Some("SMGW")` or `Some("CLS_GATEWAY")` → always `IMsys`
/// - 15-min intervals with non-SMGW source → `Rlm`
/// - 60-min intervals → `Rlm`
/// - Daily/monthly or fewer intervals → `Slp`
///
/// # Example
/// ```rust
/// use metering::{classify_messtyp, Messtyp};
/// use metering::interval::MeterInterval;
/// use metering::interval::QualityFlag;
/// use rust_decimal::Decimal;
/// use time::macros::datetime;
///
/// // 15-min intervals without SMGW source → RLM
/// let iv = MeterInterval {
///     from: datetime!(2026-01-01 0:00 UTC),
///     to:   datetime!(2026-01-01 0:15 UTC),
///     value_kwh: Decimal::from(2u32),
///     quality: QualityFlag::Measured,
///     obis_code: None,
/// };
/// assert_eq!(classify_messtyp(&[iv], None), Messtyp::Rlm);
/// ```
#[must_use]
pub fn classify_messtyp(intervals: &[MeterInterval], source: Option<&str>) -> Messtyp {
    // SMGW/CLS direct push is always iMSys
    if let Some(src) = source {
        let src_upper = src.to_uppercase();
        if src_upper.contains("SMGW") || src_upper.contains("CLS") || src_upper.contains("IMSYS") {
            return Messtyp::IMsys;
        }
    }

    match detect_interval_length(intervals) {
        Some(
            IntervalResolution::QuarterHour
            | IntervalResolution::HalfHour
            | IntervalResolution::Hour,
        ) => Messtyp::Rlm,
        _ => Messtyp::Slp,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interval::QualityFlag;
    use rust_decimal::dec;
    use time::macros::datetime;

    fn iv_15min(i: u8) -> MeterInterval {
        let base = datetime!(2026-01-01 0:00 UTC);
        MeterInterval {
            from: base + time::Duration::minutes(i as i64 * 15),
            to: base + time::Duration::minutes(i as i64 * 15 + 15),
            value_kwh: dec!(2.0),
            quality: QualityFlag::Measured,
            obis_code: None,
        }
    }

    #[test]
    fn classify_rlm_from_15min_intervals() {
        let intervals: Vec<_> = (0..4).map(iv_15min).collect();
        assert_eq!(classify_messtyp(&intervals, None), Messtyp::Rlm);
    }

    #[test]
    fn classify_imsys_from_smgw_source() {
        let intervals: Vec<_> = (0..4).map(iv_15min).collect();
        assert_eq!(classify_messtyp(&intervals, Some("SMGW")), Messtyp::IMsys);
        assert_eq!(
            classify_messtyp(&intervals, Some("CLS_GATEWAY")),
            Messtyp::IMsys
        );
    }

    #[test]
    fn classify_slp_from_daily_intervals() {
        let base = datetime!(2026-01-01 0:00 UTC);
        let intervals = vec![MeterInterval {
            from: base,
            to: base + time::Duration::days(1),
            value_kwh: dec!(24.0),
            quality: QualityFlag::Measured,
            obis_code: None,
        }];
        assert_eq!(classify_messtyp(&intervals, None), Messtyp::Slp);
    }

    #[test]
    fn detect_15min_length() {
        let intervals: Vec<_> = (0..4).map(iv_15min).collect();
        assert_eq!(
            detect_interval_length(&intervals),
            Some(IntervalResolution::QuarterHour)
        );
    }

    #[test]
    fn imsys_supports_dynamic_tariff_enwg_41a() {
        assert!(Messtyp::IMsys.supports_dynamic_tariff());
        assert!(!Messtyp::Rlm.supports_dynamic_tariff());
        assert!(!Messtyp::Slp.supports_dynamic_tariff());
    }
}
