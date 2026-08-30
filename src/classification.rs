//! SLP/RLM/iMSys classification and interval length detection.
//!
//! ## Legal basis
//!
//! - **§ 2 MsbG**: RLM = registrierende Lastgangmessung (15-min or 60-min intervals).
//! - **§ 2 MsbG** — registrierende Lastgangmessung and intelligentes Messsystem.
//! - **§ 41a Abs. 2 EnWG** — suppliers with more than 100 000 Letztverbraucher
//!   must offer a dynamic tariff to customers who *have* an iMSys. Note this is
//!   an obligation on the **supplier**, not a resolution mandate on the
//!   meter.
//!
//! ## What this classifies, and what it cannot
//!
//! Classification here is **from the observed series alone**: the interval
//! length, plus an optional statement about where the data came from.
//!
//! | Observed | Messtyp |
//! |---|---|
//! | source says SMGW / iMSys | `IMsys` |
//! | 15, 30 or 60 minute intervals | `Rlm` |
//! | anything coarser | `Slp` |
//!
//! The **consumption thresholds do not appear here** — the SLP/RLM boundary at
//! 100 000 kWh/a is a property of the Marktlokation's master data and annual
//! quantity, neither of which is in a `MeterInterval`.

use crate::interval::MeterInterval;
use crate::resolution::IntervalResolution;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// Metering type (Messtyp).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "SCREAMING_SNAKE_CASE"))]
pub enum Messtyp {
    /// Standard load profile — daily or coarser aggregates, with no interval
    /// series to read a Viertelstundenleistung out of.
    Slp,
    /// Registrierende Lastgangmessung — an equidistant interval series,
    /// 15 to 60 minutes (§ 2 MsbG).
    Rlm,
    /// Intelligentes Messsystem — a metering system behind a Smart-Meter-
    /// Gateway (§ 2 Satz 1 Nr. 7 MsbG), delivering quarter-hour values.
    IMsys,
}

impl Messtyp {
    /// Every Messtyp, in declaration order.
    pub const ALL: [Self; 3] = [Self::Slp, Self::Rlm, Self::IMsys];

    /// Stable DB/wire label. Matches the `serde` tag and
    /// [`FromStr`](std::str::FromStr) input.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Slp => "SLP",
            Self::Rlm => "RLM",
            Self::IMsys => "I_MSYS",
        }
    }

    /// `true` when the Messtyp supports Spitzenleistung (peak demand) billing.
    #[must_use]
    pub fn supports_spitzenleistung(&self) -> bool {
        matches!(self, Messtyp::Rlm | Messtyp::IMsys)
    }

    /// `true` when the Messtyp can serve a § 41a EnWG dynamic tariff, which
    /// needs the quarter-hour values a Smart-Meter-Gateway delivers.
    #[must_use]
    pub fn supports_dynamic_tariff(&self) -> bool {
        matches!(self, Messtyp::IMsys)
    }
}

/// Detect the dominant interval length in a set of meter intervals.
///
/// Uses the median interval duration, so it is robust against gaps and against
/// the odd short or long interval at a DST transition, and maps it through
/// [`IntervalResolution::from_observed_seconds`] — the one tolerance table,
/// shared with [`crate::reading::detect_reading_cadence`], so the two cannot
/// disagree about what a daily series looks like.
///
/// # Returns
///
/// `None` when `intervals` is empty, every interval is zero-length, or the
/// median is too long to be a resolution at all.
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
    IntervalResolution::from_observed_seconds(durations[durations.len() / 2])
}

/// How a series reached the system, where that settles the Messtyp on its own.
///
/// A series that arrived through a Smart-Meter-Gateway is from an
/// intelligentes Messsystem by definition (§ 2 Satz 1 Nr. 7 MsbG), whatever its
/// interval length looks like. Nothing else about the transport is decisive, so
/// this enum has exactly the two answers that matter.
///
/// A closed enum rather than a free-text label: substring-matching a
/// caller-supplied string would classify `"LEGACY_NON_SMGW_IMPORT"` as iMSys
/// and a feed labelled `"Gateway"` as not.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "SCREAMING_SNAKE_CASE"))]
pub enum SeriesOrigin {
    /// Delivered by a Smart-Meter-Gateway — directly, or over a CLS channel.
    SmartMeterGateway,
    /// Anything else: an MSCONS delivery, a manual entry, a file import.
    ///
    /// Says nothing about the Messtyp on its own, so classification falls back
    /// to the observed interval length.
    Other,
}

impl SeriesOrigin {
    /// Every variant, in declaration order.
    pub const ALL: [Self; 2] = [Self::SmartMeterGateway, Self::Other];

    /// Stable DB/wire label. Matches the `serde` tag and
    /// [`FromStr`](std::str::FromStr) input.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::SmartMeterGateway => "SMART_METER_GATEWAY",
            Self::Other => "OTHER",
        }
    }
}

crate::codes::string_codes! {
    Messtyp;
    SeriesOrigin;
}

/// Classify the metering type from the observed series and, optionally, how it
/// arrived.
///
/// | Evidence | Messtyp |
/// |---|---|
/// | [`SeriesOrigin::SmartMeterGateway`] | `IMsys` |
/// | intervals of 15, 30 or 60 minutes | `Rlm` |
/// | anything coarser, or no usable series | `Slp` |
///
/// The gateway wins over the interval length: an iMSys delivering hourly values
/// is still an iMSys.
///
/// # Example
/// ```rust
/// use metering::{classify_messtyp, Messtyp};
/// use metering::classification::SeriesOrigin;
/// use metering::interval::{MeterInterval, QualityFlag};
/// use rust_decimal::dec;
/// use time::macros::datetime;
///
/// let iv = MeterInterval {
///     from: datetime!(2026-01-01 0:00 UTC),
///     to:   datetime!(2026-01-01 0:15 UTC),
///     value: dec!(2),
///     quality: QualityFlag::Measured,
///     obis_code: None,
/// };
///
/// // Quarter-hours with no gateway claim → RLM.
/// assert_eq!(classify_messtyp(&[iv.clone()], None), Messtyp::Rlm);
/// // ...the same series from a gateway → iMSys.
/// assert_eq!(
///     classify_messtyp(&[iv], Some(SeriesOrigin::SmartMeterGateway)),
///     Messtyp::IMsys,
/// );
/// ```
#[must_use]
pub fn classify_messtyp(intervals: &[MeterInterval], origin: Option<SeriesOrigin>) -> Messtyp {
    if origin == Some(SeriesOrigin::SmartMeterGateway) {
        return Messtyp::IMsys;
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
            value: dec!(2.0),
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
    fn a_gateway_origin_outranks_the_interval_length() {
        let intervals: Vec<_> = (0..4).map(iv_15min).collect();
        assert_eq!(
            classify_messtyp(&intervals, Some(SeriesOrigin::SmartMeterGateway)),
            Messtyp::IMsys
        );
        assert_eq!(
            classify_messtyp(&intervals, Some(SeriesOrigin::Other)),
            Messtyp::Rlm
        );

        // Even a daily series from a gateway is an iMSys.
        let base = datetime!(2026-01-01 0:00 UTC);
        let daily = vec![MeterInterval {
            from: base,
            to: base + time::Duration::days(1),
            value: dec!(24.0),
            quality: QualityFlag::Measured,
            obis_code: None,
        }];
        assert_eq!(
            classify_messtyp(&daily, Some(SeriesOrigin::SmartMeterGateway)),
            Messtyp::IMsys
        );
        assert_eq!(classify_messtyp(&daily, None), Messtyp::Slp);
    }

    #[test]
    fn classify_slp_from_daily_intervals() {
        let base = datetime!(2026-01-01 0:00 UTC);
        let intervals = vec![MeterInterval {
            from: base,
            to: base + time::Duration::days(1),
            value: dec!(24.0),
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
