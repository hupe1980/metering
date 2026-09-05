//! Blindarbeit — the reactive energy a Netzbetreiber charges for.
//!
//! A Netznutzer draws real energy (Wirkarbeit, kWh) and reactive energy
//! (Blindarbeit, kvarh). The reactive part performs no work but loads the
//! network, so the Netzbetreiber grants a **Freigrenze** proportional to the
//! Wirkarbeit and charges the excess:
//!
//! ```text
//! Blindmehrarbeit = max(0, Blindarbeit − ratio × Wirkarbeit)
//! ```
//!
//! A quantity in kvarh, which is why it is here; what it costs is the
//! Preisblatt's business.
//!
//! ## The ratio is the Netzbetreiber's
//!
//! No national rule fixes it — § 17 Abs. 1 StromNEV leaves the condition to the
//! Ergänzende Bedingungen and the Preisblatt — and published Preisblätter state
//! it two ways: **50 % der Wirkarbeit** ([`RATIO_HALF`], a `cos φ` of about
//! 0,894) or **cos φ = 0,9** ([`RATIO_COS_PHI_0_9`], the stricter 0,4843). Both
//! are constants because they are published practice, not because either is the
//! rule; neither is quoted from a document this crate can cite, since a
//! Preisblatt is per Netzbetreiber. A third value is passed to
//! [`ReactiveLimit::new`].
//!
//! ```text
//! ratio = tan(arccos(cos φ)) = √(1 − cos²φ) ÷ cos φ
//! ```
//!
//! Stated rather than computed: a square root has no exact decimal, and no
//! float touches a number that multiplies a billed quantity.
//!
//! ## Which registers
//!
//! Wirkarbeit is the Bezug register (OBIS `1-0:1.8.x`), Blindarbeit the
//! reactive one — `1-0:3.8.0`, or the quadrant registers `1-0:5.8.0`…`1-0:8.8.0`
//! ([`ObisCode::is_reactive`](crate::ObisCode::is_reactive)). Passing an export
//! register as Wirkarbeit inflates the Freigrenze, so both totals are arguments
//! rather than guesses from a mixed series.

use rust_decimal::{Decimal, dec};

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// The **50 % der Wirkarbeit** rule most Preisblätter state, as a ratio.
///
/// A power factor of about 0,894 — the looser of the two conventions, and the
/// one written in prose rather than as a `cos φ`.
pub const RATIO_HALF: Decimal = dec!(0.5);

/// `cos φ = 0,9` expressed as a ratio: `tan(arccos 0,9)`, to four places.
///
/// The exact value is irrational — `√(1 − 0,81) ÷ 0,9 = 0,4843221…` — so this
/// is a **rounding**, and it is stated as one rather than presented as the
/// number itself. Four places is what the Preisblätter that spell the ratio
/// out print; a Netzbetreiber quoting more takes [`ReactiveLimit::new`].
pub const RATIO_COS_PHI_0_9: Decimal = dec!(0.4843);

/// How much Blindarbeit is free of charge, per unit of Wirkarbeit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct ReactiveLimit {
    /// kvarh admitted free per kWh of Wirkarbeit.
    ///
    /// Negative values are meaningless and are treated as zero by
    /// [`blindmehrarbeit`], which then charges every kvarh.
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::decimal"))]
    pub ratio: Decimal,
}

impl ReactiveLimit {
    /// The Netzbetreiber's own ratio, from its Preisblatt.
    #[must_use]
    pub const fn new(ratio: Decimal) -> Self {
        Self { ratio }
    }

    /// **50 % der Wirkarbeit** — [`RATIO_HALF`].
    #[must_use]
    pub const fn half() -> Self {
        Self::new(RATIO_HALF)
    }

    /// `cos φ = 0,9` as [`RATIO_COS_PHI_0_9`].
    #[must_use]
    pub const fn cos_phi_0_9() -> Self {
        Self::new(RATIO_COS_PHI_0_9)
    }
}

impl Default for ReactiveLimit {
    /// [`half`](Self::half) — the formulation most Preisblätter use.
    ///
    /// A default that is *a* published convention rather than *the* rule; it is
    /// stated on every [`ReactiveBalance`] so a report can show which was
    /// applied.
    fn default() -> Self {
        Self::half()
    }
}

/// The Blindarbeit balance for one billing period.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct ReactiveBalance {
    /// Wirkarbeit drawn in the period (kWh).
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::decimal"))]
    pub wirkarbeit_kwh: Decimal,
    /// Blindarbeit drawn in the period (kvarh).
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::decimal"))]
    pub blindarbeit_kvarh: Decimal,
    /// The ratio applied, carried so a report states its own basis.
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::decimal"))]
    pub ratio: Decimal,
    /// Blindarbeit admitted free of charge: `ratio × Wirkarbeit` (kvarh).
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::decimal"))]
    pub freigrenze_kvarh: Decimal,
    /// The chargeable excess: `max(0, Blindarbeit − Freigrenze)` (kvarh).
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::decimal"))]
    pub blindmehrarbeit_kvarh: Decimal,
}

impl ReactiveBalance {
    /// `true` when there is a chargeable excess.
    #[must_use]
    pub fn is_chargeable(&self) -> bool {
        self.blindmehrarbeit_kvarh > Decimal::ZERO
    }

    /// How much of the Freigrenze went unused (kvarh), never below zero.
    ///
    /// The headroom a load could still take before it starts paying — the
    /// figure a Blindleistungskompensation is sized against.
    #[must_use]
    pub fn headroom_kvarh(&self) -> Decimal {
        (self.freigrenze_kvarh - self.blindarbeit_kvarh).max(Decimal::ZERO)
    }
}

/// The Blindmehrarbeit for a period, from its two register totals.
///
/// Exact decimal arithmetic: one product and one difference, no division and no
/// rounding, so the balance reconstructs digit for digit from the totals it was
/// given.
///
/// A negative ratio is read as zero — the Netzbetreiber that admits nothing
/// free charges every kvarh — rather than as a negative Freigrenze, which would
/// charge *more* than the meter recorded.
///
/// ```rust
/// use metering::reactive::{ReactiveLimit, blindmehrarbeit};
/// use rust_decimal::dec;
///
/// // 100 000 kWh with 62 000 kvarh against the 50 % rule: 12 000 kvarh over.
/// let balance = blindmehrarbeit(dec!(100000), dec!(62000), ReactiveLimit::half());
/// assert_eq!(balance.freigrenze_kvarh, dec!(50000.0));
/// assert_eq!(balance.blindmehrarbeit_kvarh, dec!(12000.0));
/// assert!(balance.is_chargeable());
///
/// // The same period under a cos φ of 0,9 admits less, so more is charged.
/// let stricter = blindmehrarbeit(dec!(100000), dec!(62000), ReactiveLimit::cos_phi_0_9());
/// assert_eq!(stricter.blindmehrarbeit_kvarh, dec!(13570.0000));
/// assert!(stricter.blindmehrarbeit_kvarh > balance.blindmehrarbeit_kvarh);
/// ```
#[must_use]
pub fn blindmehrarbeit(
    wirkarbeit_kwh: Decimal,
    blindarbeit_kvarh: Decimal,
    limit: ReactiveLimit,
) -> ReactiveBalance {
    let ratio = limit.ratio.max(Decimal::ZERO);
    let freigrenze_kvarh = (ratio * wirkarbeit_kwh).max(Decimal::ZERO);
    ReactiveBalance {
        wirkarbeit_kwh,
        blindarbeit_kvarh,
        ratio,
        freigrenze_kvarh,
        blindmehrarbeit_kvarh: (blindarbeit_kvarh - freigrenze_kvarh).max(Decimal::ZERO),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_excess_is_what_the_freigrenze_does_not_cover() {
        let b = blindmehrarbeit(dec!(1000), dec!(600), ReactiveLimit::half());
        assert_eq!(b.freigrenze_kvarh, dec!(500.0));
        assert_eq!(b.blindmehrarbeit_kvarh, dec!(100.0));
        assert!(b.is_chargeable());
        assert_eq!(b.headroom_kvarh(), Decimal::ZERO);
    }

    #[test]
    fn a_compensated_load_pays_nothing_and_keeps_headroom() {
        let b = blindmehrarbeit(dec!(1000), dec!(200), ReactiveLimit::half());
        assert_eq!(b.blindmehrarbeit_kvarh, Decimal::ZERO);
        assert!(!b.is_chargeable());
        assert_eq!(b.headroom_kvarh(), dec!(300.0));
    }

    /// The stricter convention charges more for the same registers — the
    /// direction that matters when a caller picks the wrong one.
    #[test]
    fn cos_phi_zero_nine_is_stricter_than_the_half_rule() {
        let half = blindmehrarbeit(dec!(1000), dec!(600), ReactiveLimit::half());
        let strict = blindmehrarbeit(dec!(1000), dec!(600), ReactiveLimit::cos_phi_0_9());
        assert!(strict.blindmehrarbeit_kvarh > half.blindmehrarbeit_kvarh);
        assert_eq!(strict.freigrenze_kvarh, dec!(484.3000));
    }

    /// No division, so the balance is exact whatever the totals look like.
    #[test]
    fn the_balance_is_exact() {
        let b = blindmehrarbeit(
            dec!(1234.567),
            dec!(1000.001),
            ReactiveLimit::new(dec!(0.3333)),
        );
        assert_eq!(b.freigrenze_kvarh, dec!(1234.567) * dec!(0.3333));
        assert_eq!(
            b.blindmehrarbeit_kvarh + b.freigrenze_kvarh,
            b.blindarbeit_kvarh
        );
    }

    /// A ratio below zero would make the Freigrenze negative and charge more
    /// than the meter recorded. It is read as "nothing is free" instead.
    #[test]
    fn a_negative_ratio_admits_nothing_rather_than_charging_extra() {
        let b = blindmehrarbeit(dec!(1000), dec!(600), ReactiveLimit::new(dec!(-0.5)));
        assert_eq!(b.ratio, Decimal::ZERO);
        assert_eq!(b.freigrenze_kvarh, Decimal::ZERO);
        assert_eq!(b.blindmehrarbeit_kvarh, dec!(600));
    }

    /// The published ratio is a rounding of an irrational, and says so; this
    /// pins the value the docs quote.
    #[test]
    fn the_cos_phi_constant_is_the_stated_rounding() {
        assert_eq!(RATIO_COS_PHI_0_9, dec!(0.4843));
        assert_eq!(RATIO_HALF, dec!(0.5));
        assert_eq!(ReactiveLimit::default(), ReactiveLimit::half());
    }
}
