//! EEG and EnFG quantities: negative-price quarter-hours, Zeitgleichheit, and
//! the BNetzA MiSpeL Festlegung for storage and charge points.
//!
//! | Module | Rule | What leaves it |
//! |---|---|---|
//! | [`negative_prices`] | EEG § 51, § 51a | zero-value quarter-hours, the extension of the Vergütungszeitraum, the § 51 Abs. 3 energy |
//! | [`zeitgleichheit`] | EnFG § 46 Abs. 3 and 5 | the per-interval coincident quantity, the worst-case estimate |
//! | [`mispel`] | MiSpeL Anlage 1 and 2 | the Abgrenzungs- and Pauschaloption quantities, formula by formula |
//!
//! Quantities only: where a rule turns on the sign of a price (AW¼ > 0,
//! SP¼ ≥ 0, a negative spot price), the caller passes which quarter-hours
//! pass the test, never the price.
//!
//! EnFG § 46 Abs. 5 counts electricity
//! *"höchstens bis zu der Höhe der tatsächlichen Netzentnahme, bezogen auf
//! jedes 15-Minuten-Intervall"*; MiSpeL's (1)¼ and (2)¼ are the same
//! per-interval minimum, and both use one function: minimum per interval
//! first, sum afterwards.
//!
//! Input series must be quarter-hourly ([`Resolution::QUARTER_HOUR`]) and hold
//! the same quarter-hours; otherwise the first instant at which they part is
//! reported.

use rust_decimal::Decimal;
use time::OffsetDateTime;

use crate::series::Series;
use crate::series::interval::QualityFlag;
use crate::time::resolution::Resolution;

pub mod mispel;
pub mod negative_prices;
pub mod zeitgleichheit;

/// Why an EEG, EnFG or MiSpeL quantity could not be computed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum EegError {
    /// A series is not quarter-hourly.
    #[error("series {series} has resolution {found}, not PT15M")]
    Resolution {
        /// The series, by the source's symbol (`Z1NB`, …).
        series: &'static str,
        /// Its resolution.
        found: Resolution,
    },
    /// A quarter-hour is present in one series and absent from another.
    #[error("series {series} is not on the grid of the first series at {at}")]
    Misaligned {
        /// The series that parts from the first.
        series: &'static str,
        /// The first instant at which the grids differ.
        at: OffsetDateTime,
    },
    /// A value is [`Faulty`](QualityFlag::Faulty) or
    /// [`Unknown`](QualityFlag::Unknown) and may not enter a quantity.
    #[error("series {series} holds a non-billable value at {at}")]
    NotBillable {
        /// The series.
        series: &'static str,
        /// The interval start.
        at: OffsetDateTime,
    },
    /// A value is negative; inputs are single-direction registers, never net.
    #[error("series {series} holds a negative value at {at}")]
    Negative {
        /// The series.
        series: &'static str,
        /// The interval start.
        at: OffsetDateTime,
    },
    /// The inputs hold no interval.
    #[error("no interval to evaluate")]
    Empty,
    /// An interval lies outside the evaluated month, year or Rumpfjahr.
    #[error("the interval starting {at} lies outside the evaluated period")]
    OutsidePeriod {
        /// The first offending interval start.
        at: OffsetDateTime,
    },
    /// A weight or capacity that divides is zero or negative.
    #[error("{formula} divides by a zero or negative {what}")]
    ZeroWeight {
        /// The formula, by its number in the source.
        formula: &'static str,
        /// What is zero.
        what: &'static str,
    },
    /// An instant is not the start of a quarter-hour of the Berlin grid.
    #[error("{at} is not the start of a quarter-hour")]
    NotQuarterHour {
        /// The instant.
        at: OffsetDateTime,
    },
    /// A quarter-hour the computation needs is missing from a series.
    #[error("series {series} has no value for the quarter-hour starting {at}")]
    Missing {
        /// The series.
        series: &'static str,
        /// The quarter-hour start.
        at: OffsetDateTime,
    },
    /// A date or instant lies outside the supported calendar years, or a
    /// date range is reversed or spans two years.
    #[error("the date range is outside the supported calendar")]
    Calendar,
    /// Inputs that must agree in shape do not (e.g. overlapping months).
    #[error("the inputs do not agree: {what}")]
    Inconsistent {
        /// What disagrees.
        what: &'static str,
    },
    /// A sum or product left the `Decimal` range.
    #[error("arithmetic overflow")]
    Overflow,
}

/// The coincident share of two quantities in one interval: the lesser.
///
/// EnFG § 46 Abs. 5 and MiSpeL Anl. 1 (1)¼ = MIN \[ Z1NB¼ ; Z2V¼ \],
/// (2)¼ = MIN \[ Z1NE¼ ; Z2E¼ \] (pp. 33–34). Applied per interval, never to
/// sums.
pub(crate) fn coincident(a: Decimal, b: Decimal) -> Decimal {
    a.min(b)
}

/// One quarter-hour of several aligned series.
#[derive(Debug, Clone)]
pub(crate) struct Row {
    pub(crate) from: OffsetDateTime,
    pub(crate) to: OffsetDateTime,
    pub(crate) values: Vec<Decimal>,
    pub(crate) quality: QualityFlag,
}

/// Check that every series is quarter-hourly, holds the same quarter-hours,
/// and carries only non-negative billable values; return the rows.
pub(crate) fn align(inputs: &[(&'static str, &Series)]) -> Result<Vec<Row>, EegError> {
    let Some(&(_, first)) = inputs.first() else {
        return Err(EegError::Empty);
    };
    for &(name, s) in inputs {
        if s.resolution() != Resolution::QUARTER_HOUR {
            return Err(EegError::Resolution {
                series: name,
                found: s.resolution(),
            });
        }
    }
    let mut rows = Vec::with_capacity(first.len());
    for (i, head) in first.iter().enumerate() {
        let mut values = Vec::with_capacity(inputs.len());
        let mut quality = QualityFlag::Measured;
        for &(name, s) in inputs {
            let Some(iv) = s.as_slice().get(i) else {
                return Err(EegError::Misaligned {
                    series: name,
                    at: head.from(),
                });
            };
            if iv.from() != head.from() {
                return Err(EegError::Misaligned {
                    series: name,
                    at: iv.from().min(head.from()),
                });
            }
            if !iv.quality().is_billable() {
                return Err(EegError::NotBillable {
                    series: name,
                    at: iv.from(),
                });
            }
            if iv.value() < Decimal::ZERO {
                return Err(EegError::Negative {
                    series: name,
                    at: iv.from(),
                });
            }
            quality = quality.worse_of(iv.quality());
            values.push(iv.value());
        }
        rows.push(Row {
            from: head.from(),
            to: head.to(),
            values,
            quality,
        });
    }
    for &(name, s) in inputs {
        if let Some(extra) = s.as_slice().get(first.len()) {
            return Err(EegError::Misaligned {
                series: name,
                at: extra.from(),
            });
        }
    }
    Ok(rows)
}

/// `Σ`, checked.
pub(crate) fn sum(values: impl IntoIterator<Item = Decimal>) -> Result<Decimal, EegError> {
    values
        .into_iter()
        .try_fold(Decimal::ZERO, |acc, v| acc.checked_add(v))
        .ok_or(EegError::Overflow)
}

/// `a · b / c`, product first, one division last; `None` when `c` is zero.
pub(crate) fn share(a: Decimal, b: Decimal, c: Decimal) -> Result<Option<Decimal>, EegError> {
    if c.is_zero() {
        return Ok(None);
    }
    a.checked_mul(b)
        .ok_or(EegError::Overflow)?
        .checked_div(c)
        .map(Some)
        .ok_or(EegError::Overflow)
}

/// `a − b`, checked.
pub(crate) fn sub(a: Decimal, b: Decimal) -> Result<Decimal, EegError> {
    a.checked_sub(b).ok_or(EegError::Overflow)
}

/// `a + b`, checked.
pub(crate) fn add(a: Decimal, b: Decimal) -> Result<Decimal, EegError> {
    a.checked_add(b).ok_or(EegError::Overflow)
}

/// `a · b`, checked.
pub(crate) fn mul(a: Decimal, b: Decimal) -> Result<Decimal, EegError> {
    a.checked_mul(b).ok_or(EegError::Overflow)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::series::interval::MeterInterval;
    use crate::time::calendar::DayBoundary;
    use rust_decimal::dec;
    use time::Duration;
    use time::macros::datetime;

    fn qh(values: &[(i64, Decimal)]) -> Series {
        let t = datetime!(2026-06-01 0:00 UTC);
        let ivs = values
            .iter()
            .map(|&(i, v)| {
                MeterInterval::quarter_hour(t + Duration::minutes(15 * i), v, QualityFlag::Measured)
                    .unwrap()
            })
            .collect();
        Series::new(Resolution::QUARTER_HOUR, DayBoundary::Strom, ivs).unwrap()
    }

    #[test]
    fn alignment_refuses_a_parting_grid_with_the_instant() {
        let a = qh(&[(0, dec!(1)), (1, dec!(1))]);
        let b = qh(&[(0, dec!(1)), (2, dec!(1))]);
        assert_eq!(
            align(&[("a", &a), ("b", &b)]).unwrap_err(),
            EegError::Misaligned {
                series: "b",
                at: datetime!(2026-06-01 0:15 UTC)
            }
        );
        let short = qh(&[(0, dec!(1))]);
        assert!(matches!(
            align(&[("a", &a), ("short", &short)]),
            Err(EegError::Misaligned {
                series: "short",
                ..
            })
        ));
        assert!(matches!(
            align(&[("short", &short), ("a", &a)]),
            Err(EegError::Misaligned { series: "a", .. })
        ));
        let neg = qh(&[(0, dec!(-1)), (1, dec!(1))]);
        assert!(matches!(
            align(&[("a", &a), ("neg", &neg)]),
            Err(EegError::Negative { series: "neg", .. })
        ));
    }

    #[test]
    fn alignment_refuses_other_resolutions_and_faulty_values() {
        let t = datetime!(2026-06-01 0:00 UTC);
        let hour = Series::new(
            Resolution::Hour,
            DayBoundary::Strom,
            vec![MeterInterval::hour(t, dec!(1), QualityFlag::Measured).unwrap()],
        )
        .unwrap();
        assert!(matches!(
            align(&[("h", &hour)]),
            Err(EegError::Resolution { series: "h", .. })
        ));
        let faulty = Series::new(
            Resolution::QUARTER_HOUR,
            DayBoundary::Strom,
            vec![MeterInterval::quarter_hour(t, dec!(1), QualityFlag::Faulty).unwrap()],
        )
        .unwrap();
        assert_eq!(
            align(&[("f", &faulty)]).unwrap_err(),
            EegError::NotBillable { series: "f", at: t }
        );
    }
}
