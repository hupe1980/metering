//! Mehr-/Mindermengensaldo (imbalance): metered against bilanzierte energy.
//!
//! | Commodity | Source |
//! |---|---|
//! | Strom | GPKE (BK6-24-174) Teil 1, Kap. 8.4 — Jahresmehr- und Jahresmindermengen |
//! | Gas (SLP only; RLM is Ziff. 3 of GaBi Gas 2.0) | GaBi Gas 2.1 (BK7-24-01-008), Tenorziffer 3a — *"Abweichungen zwischen allokierten Mengen und der tatsächlichen Ausspeisung beim Letztverbraucher"*, settled *"mindestens jährlich"* |
//!
//! Both quantities are named from the **network operator's** side
//! (GPKE Kap. 8.4 Nr. 3):
//!
//! > Unterschreitet die Summe der in einem Zeitraum ermittelten elektrischen
//! > Arbeit die Summe der Arbeit, die den bilanzierten Profilen zu Grunde gelegt
//! > wurde (ungewollte Mehrmenge), so vergütet der Netzbetreiber dem Lieferanten
//! > oder dem Kunden diese Differenzmenge.
//!
//! ```text
//! Mehr-Menge   = max(0, bilanziert_kwh − actual_kwh)   [NB vergütet → NB owes LF]
//! Minder-Menge = max(0, actual_kwh − bilanziert_kwh)   [NB stellt in Rechnung → LF owes NB]
//! ```
//!
//! At most one of the two is positive. The second input is the **bilanzierte**
//! (allocated) quantity, not a contracted one; it is the Netzbetreiber's output,
//! so the caller supplies it. Start at [`ImbalanceSaldo`].

use rust_decimal::Decimal;

#[cfg(feature = "serde")]
use serde::Serialize;

/// A Mehr-/Mindermengensaldo: metered and bilanzierte energy, every derived
/// figure a method.
///
/// ```rust
/// use metering::billing::imbalance::ImbalanceSaldo;
/// use rust_decimal::dec;
///
/// // 1050 kWh measured against a 1000 kWh profile → Mindermenge 50 kWh,
/// // which the network operator invoices.
/// let saldo = ImbalanceSaldo::new(dec!(1050), dec!(1000)).unwrap();
/// assert_eq!(saldo.minder_kwh(), dec!(50));
/// assert!(saldo.is_minder());
/// assert!(!saldo.is_mehr());
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize))]
#[non_exhaustive]
pub struct ImbalanceSaldo {
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::decimal"))]
    actual_kwh: Decimal,
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::decimal"))]
    bilanziert_kwh: Decimal,
}

impl ImbalanceSaldo {
    /// The saldo of a period from its metered and its bilanzierte energy (kWh).
    /// `None` when `actual − bilanziert` does not fit a `Decimal`.
    #[must_use]
    pub fn new(actual_kwh: Decimal, bilanziert_kwh: Decimal) -> Option<Self> {
        actual_kwh.checked_sub(bilanziert_kwh)?;
        Some(Self {
            actual_kwh,
            bilanziert_kwh,
        })
    }

    /// Metered energy (kWh).
    #[must_use]
    pub const fn actual_kwh(&self) -> Decimal {
        self.actual_kwh
    }

    /// Bilanzierte energy (kWh) — what the balance group was allocated.
    #[must_use]
    pub const fn bilanziert_kwh(&self) -> Decimal {
        self.bilanziert_kwh
    }

    /// Signed delta `actual − bilanziert`. Positive is a **Minder**menge.
    #[must_use]
    pub fn delta_kwh(&self) -> Decimal {
        // Checked in `new`.
        self.actual_kwh - self.bilanziert_kwh
    }

    /// Mehrmenge `max(0, bilanziert − actual)`: NB vergütet, so NB owes LF.
    #[must_use]
    pub fn mehr_kwh(&self) -> Decimal {
        (-self.delta_kwh()).max(Decimal::ZERO)
    }

    /// Mindermenge `max(0, actual − bilanziert)`: NB invoices, so LF owes NB.
    #[must_use]
    pub fn minder_kwh(&self) -> Decimal {
        self.delta_kwh().max(Decimal::ZERO)
    }

    /// `true` when there is a Mehrmengen position (NB owes LF, a credit).
    #[must_use]
    pub fn is_mehr(&self) -> bool {
        self.mehr_kwh() > Decimal::ZERO
    }

    /// `true` when there is a Mindermengen position (LF owes NB, a charge).
    #[must_use]
    pub fn is_minder(&self) -> bool {
        self.minder_kwh() > Decimal::ZERO
    }

    /// `true` when actual == bilanziert.
    #[must_use]
    pub fn is_balanced(&self) -> bool {
        self.delta_kwh().is_zero()
    }

    /// `delta × 100 ÷ bilanziert`, rounded once to
    /// [`PERCENT_DP`](crate::precision::PERCENT_DP) places. `None` when
    /// `bilanziert_kwh` is zero or on overflow.
    #[must_use]
    pub fn delta_pct(&self) -> Option<Decimal> {
        if self.bilanziert_kwh.is_zero() {
            return None;
        }
        Some(
            self.delta_kwh()
                .checked_mul(Decimal::ONE_HUNDRED)?
                .checked_div(self.bilanziert_kwh)?
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

    fn saldo(actual: Decimal, bilanziert: Decimal) -> ImbalanceSaldo {
        ImbalanceSaldo::new(actual, bilanziert).unwrap()
    }

    /// Consuming above the profile is an ungewollte **Minder**menge.
    #[test]
    fn over_consumption_is_a_mindermenge() {
        let s = saldo(dec!(1050), dec!(1000));
        assert_eq!(s.minder_kwh(), dec!(50));
        assert_eq!(s.mehr_kwh(), Decimal::ZERO);
        assert_eq!(s.delta_kwh(), dec!(50));
        assert_eq!(s.delta_pct(), Some(dec!(5)));
    }

    /// Consuming below the profile is an ungewollte **Mehr**menge.
    #[test]
    fn under_consumption_is_a_mehrmenge() {
        let s = saldo(dec!(950), dec!(1000));
        assert_eq!(s.mehr_kwh(), dec!(50));
        assert_eq!(s.minder_kwh(), Decimal::ZERO);
        assert!(s.is_mehr() && !s.is_minder());
    }

    #[test]
    fn a_balanced_period_and_a_zero_profile() {
        let s = saldo(dec!(1000), dec!(1000));
        assert!(s.is_balanced());
        assert_eq!(s.delta_pct(), Some(Decimal::ZERO));
        assert_eq!(saldo(dec!(100), Decimal::ZERO).delta_pct(), None);
    }

    /// Derived, not stored: the parts always reconstruct the inputs.
    #[test]
    fn the_saldo_reconstructs_its_inputs() {
        for (a, b) in [
            (dec!(900), dec!(1000)),
            (dec!(1100), dec!(1000)),
            (dec!(5), dec!(5)),
        ] {
            let s = saldo(a, b);
            assert_eq!(
                s.bilanziert_kwh() + s.minder_kwh() - s.mehr_kwh(),
                s.actual_kwh()
            );
            assert!(!(s.is_mehr() && s.is_minder()));
        }
        assert_eq!(ImbalanceSaldo::new(Decimal::MIN, Decimal::MAX), None);
    }
}
