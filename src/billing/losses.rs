//! Netzverlust — grid-loss balance over a grid area (§22 Abs. 1 EnWG).
//!
//! ```text
//! Verlust = Σ Einspeisung ins Netz − Σ Entnahme aus dem Netz
//! ```
//!
//! An **indicator**, not a settlement quantity: unmetered infeed or offtake
//! shows up as phantom loss or gain. Settlement-grade Verlustenergie uses the
//! DSO's Bilanzkreis data.

use rust_decimal::Decimal;

/// A grid-loss balance over one period: the two totals, every derived figure
/// a method. Callers aggregate the totals themselves (e.g. OBIS `2.8.x`/`2.29.x` series for infeed, `1.8.x`/`1.29.x` for
/// offtake).
///
/// ```rust
/// use metering::billing::losses::NetworkLosses;
/// use rust_decimal::dec;
///
/// let l = NetworkLosses::new(dec!(1_000_000), dec!(955_000)).unwrap();
/// assert_eq!(l.verlust_kwh(), dec!(45_000));
/// assert_eq!(l.verlust_prozent(), Some(dec!(4.50)));
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct NetworkLosses {
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::decimal"))]
    einspeisung_kwh: Decimal,
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::decimal"))]
    entnahme_kwh: Decimal,
}

impl NetworkLosses {
    /// The balance of `einspeisung_kwh` (generation feed-in plus imports over
    /// Übergabezählpunkte) against `entnahme_kwh` (customer offtake plus
    /// exports). `None` when the difference does not fit a `Decimal`.
    #[must_use]
    pub fn new(einspeisung_kwh: Decimal, entnahme_kwh: Decimal) -> Option<Self> {
        einspeisung_kwh.checked_sub(entnahme_kwh)?;
        Some(Self {
            einspeisung_kwh,
            entnahme_kwh,
        })
    }

    /// Total energy fed into the grid area (kWh).
    #[must_use]
    pub const fn einspeisung_kwh(&self) -> Decimal {
        self.einspeisung_kwh
    }

    /// Total energy taken out of the grid area (kWh).
    #[must_use]
    pub const fn entnahme_kwh(&self) -> Decimal {
        self.entnahme_kwh
    }

    /// `einspeisung − entnahme`. Positive = physical losses (plus any unmetered
    /// offtake); negative = a metering-coverage gap on the infeed side.
    #[must_use]
    pub fn verlust_kwh(&self) -> Decimal {
        // Checked in `new`.
        self.einspeisung_kwh - self.entnahme_kwh
    }

    /// Loss share of the infeed in percent, to
    /// [`PERCENT_DP`](crate::precision::PERCENT_DP) places. `None` when
    /// nothing was fed in or on overflow.
    #[must_use]
    pub fn verlust_prozent(&self) -> Option<Decimal> {
        if self.einspeisung_kwh <= Decimal::ZERO {
            return None;
        }
        Some(
            self.verlust_kwh()
                .checked_mul(Decimal::ONE_HUNDRED)?
                .checked_div(self.einspeisung_kwh)?
                .round_dp_with_strategy(
                    crate::precision::PERCENT_DP,
                    crate::precision::PERCENT_STRATEGY,
                ),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal::dec;

    #[test]
    fn a_typical_distribution_grid_shows_single_digit_losses() {
        let l = NetworkLosses::new(dec!(1_000_000), dec!(955_000)).unwrap();
        assert_eq!(l.verlust_kwh(), dec!(45_000));
        assert_eq!(l.verlust_prozent(), Some(dec!(4.50)));
    }

    #[test]
    fn negative_balance_signals_a_metering_coverage_gap() {
        let l = NetworkLosses::new(dec!(100), dec!(120)).unwrap();
        assert_eq!(l.verlust_kwh(), dec!(-20));
        assert_eq!(l.verlust_prozent(), Some(dec!(-20.00)));
    }

    #[test]
    fn zero_infeed_yields_no_percentage() {
        assert_eq!(
            NetworkLosses::new(Decimal::ZERO, dec!(10))
                .unwrap()
                .verlust_prozent(),
            None
        );
        assert_eq!(NetworkLosses::new(Decimal::MIN, Decimal::MAX), None);
    }
}
