//! Mehr-/Mindermengensaldo (imbalance) calculation.
//!
//! ## Legal basis
//!
//! - **GPKE (BK6-24-174) Teil 1, Kap. 8.4** — Jahresmehr- und Jahresmindermengen
//!   (Strom). Historically §13 Abs. 3 StromNZV, repealed with effect from the end
//!   of 31.12.2025.
//! - **GaBi Gas 2.1 (BK7-24-01-008), Tenorziffer 3a** — SLP-Mehr- und
//!   Mindermengen Gas. Historically § 25 GasNZV, repealed on the same date.
//!   **Scoped to Standardlastprofile**: the Beschluss places the RLM case in
//!   Ziff. 3 of GaBi Gas 2.0, so both exist in two different clauses. It names
//!   the same comparison this module computes — *"Abweichungen zwischen
//!   allokierten Mengen und der tatsächlichen Ausspeisung beim
//!   Letztverbraucher"* — and settles it *"mindestens jährlich"*, which is why
//!   both commodities' quantities are annual.
//!
//! ## Definition
//!
//! Both quantities are named from the **network operator's** side, which inverts
//! the intuitive reading: a Mehrmenge is one the operator *receives* and pays
//! for, a Mindermenge one it *delivers* and invoices. GPKE Kap. 8.4 Nr. 3:
//!
//! > Unterschreitet die Summe der in einem Zeitraum ermittelten elektrischen
//! > Arbeit die Summe der Arbeit, die den bilanzierten Profilen zu Grunde gelegt
//! > wurde (ungewollte Mehrmenge), so vergütet der Netzbetreiber dem Lieferanten
//! > oder dem Kunden diese Differenzmenge.
//!
//! ```text
//! Mehr-Menge   = max(0, profiled_kwh − actual_kwh)   [NB vergütet → NB owes LF]
//! Minder-Menge = max(0, actual_kwh − profiled_kwh)   [NB stellt in Rechnung → LF owes NB]
//! ```
//!
//! The customer consuming *less* than the profile leaves surplus energy the
//! network operator absorbed — that surplus is the Mehrmenge, and it is credited.
//!
//! Only one of `mehr_kwh` or `minder_kwh` is positive in any period.
//!
//! ## The second quantity is the **bilanzierte** one, not a contracted one
//!
//! GPKE Kap. 8.4 compares the metered work against *"die Summe der Arbeit, die
//! den bilanzierten Profilen zu Grunde gelegt wurde"* — what the balance group
//! was allocated. For an SLP delivery point that is the profile the
//! Netzbetreiber allocated; it is not the quantity anybody contracted for, and
//! naming it `contracted` invites exactly the substitution that produces a
//! wrong Mehrmenge.
//!
//! The gas side names the same two quantities independently — *"Abweichungen
//! zwischen allokierten Mengen und der tatsächlichen Ausspeisung beim
//! Letztverbraucher"* — so two Festlegungen, written a decade apart for two
//! commodities, agree that the comparison is **allocated against measured**.
//!
//! It is a parameter because the allocation is the Netzbetreiber's output, not
//! a measurement this crate can derive: the caller supplies it alongside the
//! metered total, and this module owns the arithmetic and the sign convention.

use rust_decimal::Decimal;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// Result of a Mehr-/Mindermengensaldo calculation.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct ImbalanceSaldo {
    /// Actual metered energy in kWh.
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::decimal"))]
    pub actual_kwh: Decimal,
    /// Bilanzierte energy in kWh — what the balance group was allocated.
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::decimal"))]
    pub bilanziert_kwh: Decimal,
    /// Mehr-Menge: `max(0, bilanziert − actual)`. NB vergütet, so NB owes LF.
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::decimal"))]
    pub mehr_kwh: Decimal,
    /// Minder-Menge: `max(0, actual − bilanziert)`. NB invoices, so LF owes NB.
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::decimal"))]
    pub minder_kwh: Decimal,
    /// Signed delta: `actual − bilanziert`. Positive is a **Minder**menge.
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::decimal"))]
    pub delta_kwh: Decimal,
}

impl ImbalanceSaldo {
    /// `true` when there is a Mehrmengen position (NB owes LF, a credit).
    #[must_use]
    pub fn is_mehr(&self) -> bool {
        self.mehr_kwh > Decimal::ZERO
    }

    /// `true` when there is a Mindermengen position (LF owes NB, a charge).
    #[must_use]
    pub fn is_minder(&self) -> bool {
        self.minder_kwh > Decimal::ZERO
    }

    /// `true` when actual == bilanziert (balanced period).
    #[must_use]
    pub fn is_balanced(&self) -> bool {
        self.delta_kwh.is_zero()
    }

    /// Absolute imbalance magnitude in kWh.
    #[must_use]
    pub fn magnitude_kwh(&self) -> Decimal {
        self.delta_kwh.abs()
    }

    /// Imbalance as a percentage of the bilanzierte quantity, to
    /// [`PERCENT_DP`](crate::PERCENT_DP) places.
    ///
    /// Cut, because this is a figure someone reads: the quotient
    /// `delta ÷ bilanziert` does not generally terminate, and a percentage
    /// carrying twenty-eight significant digits is not a percentage. The width
    /// is [`PERCENT_DP`](crate::PERCENT_DP), the one every percentage this
    /// crate reports shares with
    /// [`NetworkLosses::verlust_prozent`](crate::losses::NetworkLosses::verlust_prozent).
    ///
    /// `None` when `bilanziert_kwh` is zero — a share of nothing is not zero
    /// percent, it is undefined.
    #[must_use]
    pub fn delta_pct(&self) -> Option<Decimal> {
        if self.bilanziert_kwh.is_zero() {
            None
        } else {
            Some(
                // `delta × 100 ÷ bilanziert`, not `delta ÷ bilanziert × 100`:
                // one rounding, at the end. See the crate-level rule.
                (self.delta_kwh * Decimal::ONE_HUNDRED / self.bilanziert_kwh)
                    .round_dp_with_strategy(
                        crate::PERCENT_DP,
                        rust_decimal::RoundingStrategy::MidpointAwayFromZero,
                    ),
            )
        }
    }
}

/// Compute the Mehr-/Mindermengensaldo for a billing period.
///
/// # Example
/// ```rust
/// use metering::compute_imbalance;
/// use rust_decimal::Decimal;
///
/// // 1050 kWh measured against a 1000 kWh profile → Mindermenge 50 kWh,
/// // which the network operator invoices.
/// let saldo = compute_imbalance(
///     Decimal::from(1050u32),
///     Decimal::from(1000u32),
/// );
/// assert_eq!(saldo.minder_kwh, Decimal::from(50u32));
/// assert!(saldo.is_minder());
/// assert!(!saldo.is_mehr());
/// ```
#[must_use]
pub fn compute_imbalance(actual_kwh: Decimal, bilanziert_kwh: Decimal) -> ImbalanceSaldo {
    let delta = actual_kwh - bilanziert_kwh;
    // Under-consumption is the Mehrmenge; over-consumption the Mindermenge.
    let mehr = (-delta).max(Decimal::ZERO);
    let minder = delta.max(Decimal::ZERO);
    ImbalanceSaldo {
        actual_kwh,
        bilanziert_kwh,
        mehr_kwh: mehr,
        minder_kwh: minder,
        delta_kwh: delta,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal::dec;

    /// Consuming above the profile is an ungewollte **Minder**menge: the NB
    /// supplied the shortfall and invoices it.
    #[test]
    fn over_consumption_is_a_mindermenge() {
        let s = compute_imbalance(dec!(1050), dec!(1000));
        assert_eq!(s.minder_kwh, dec!(50));
        assert_eq!(s.mehr_kwh, Decimal::ZERO);
        assert_eq!(s.delta_kwh, dec!(50));
        assert!(s.is_minder());
        assert!(!s.is_mehr());
    }

    /// Consuming below the profile is an ungewollte **Mehr**menge: the NB took
    /// the surplus and reimburses it.
    #[test]
    fn under_consumption_is_a_mehrmenge() {
        let s = compute_imbalance(dec!(950), dec!(1000));
        assert_eq!(s.mehr_kwh, dec!(50));
        assert_eq!(s.minder_kwh, Decimal::ZERO);
        assert_eq!(s.delta_kwh, dec!(-50));
        assert!(s.is_mehr());
        assert!(!s.is_minder());
    }

    #[test]
    fn balanced_period() {
        let s = compute_imbalance(dec!(1000), dec!(1000));
        assert!(s.is_balanced());
        assert_eq!(s.magnitude_kwh(), Decimal::ZERO);
        assert_eq!(s.delta_pct(), Some(Decimal::ZERO));
    }

    #[test]
    fn delta_pct_calculation() {
        // 50 kWh excess on 1000 bilanziert = 5%
        let s = compute_imbalance(dec!(1050), dec!(1000));
        assert_eq!(s.delta_pct(), Some(dec!(5)));
    }

    #[test]
    fn delta_pct_zero_bilanziert() {
        let s = compute_imbalance(dec!(100), Decimal::ZERO);
        assert_eq!(s.delta_pct(), None);
    }

    #[test]
    fn mehr_and_minder_are_mutually_exclusive() {
        // Mehr and Minder are mutually exclusive by construction.
        for (actual, bilanziert) in [
            (dec!(900), dec!(1000)),
            (dec!(1100), dec!(1000)),
            (dec!(1000), dec!(1000)),
        ] {
            let s = compute_imbalance(actual, bilanziert);
            // Never both mehr and minder simultaneously
            assert!(
                !(s.is_mehr() && s.is_minder()),
                "mehr and minder cannot both be true"
            );
        }
    }
}
