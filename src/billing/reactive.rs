//! Blindarbeit — the reactive energy (kvarh) a Netzbetreiber charges for.
//!
//! The Netzbetreiber grants a **Freigrenze** proportional to the Wirkarbeit
//! and charges the excess:
//!
//! ```text
//! Blindmehrarbeit = max(0, Blindarbeit − ratio × Wirkarbeit)
//! ratio           = tan(arccos(cos φ)) = √(1 − cos²φ) ÷ cos φ
//! ```
//!
//! No national rule fixes the ratio — § 17 Abs. 1 StromNEV leaves it to the
//! Ergänzende Bedingungen and the Preisblatt. Published Preisblätter use
//! **50 % der Wirkarbeit** ([`RATIO_HALF`]) or **cos φ = 0,9**
//! ([`RATIO_COS_PHI_0_9`]); any other value goes through
//! [`ReactiveLimit::new`]. Neither is a default.
//!
//! Wirkarbeit is the Bezug register (OBIS `1-0:1.8.x`), Blindarbeit `1-0:3.8.0`
//! or the quadrant registers `1-0:5.8.0`…`1-0:8.8.0`
//! ([`ObisCode::is_reactive`](crate::ObisCode::is_reactive)). An export
//! register passed as Wirkarbeit inflates the Freigrenze. Pricing is the
//! Preisblatt's business. Start at [`ReactiveBalance`].

use rust_decimal::{Decimal, dec};

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// The **50 % der Wirkarbeit** rule as a ratio (a `cos φ` of about 0,894).
pub const RATIO_HALF: Decimal = dec!(0.5);

/// `cos φ = 0,9` as a ratio: `tan(arccos 0,9) = 0,4843221…`, **rounded** to
/// the four places Preisblätter print; more places go through
/// [`ReactiveLimit::new`].
pub const RATIO_COS_PHI_0_9: Decimal = dec!(0.4843);

/// How much Blindarbeit is free of charge, per unit of Wirkarbeit.
///
/// No `Default`: the ratio is the Netzbetreiber's. On the wire it is
/// `{ "ratio": "<decimal>" }`, read through [`new`](Self::new).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ReactiveLimit {
    ratio: Decimal,
}

impl ReactiveLimit {
    /// The Netzbetreiber's own ratio (kvarh free per kWh). `None` when
    /// negative.
    #[must_use]
    pub fn new(ratio: Decimal) -> Option<Self> {
        (ratio >= Decimal::ZERO).then_some(Self { ratio })
    }

    /// **50 % der Wirkarbeit** — [`RATIO_HALF`].
    pub const HALF: Self = Self { ratio: RATIO_HALF };

    /// `cos φ = 0,9` as [`RATIO_COS_PHI_0_9`].
    pub const COS_PHI_0_9: Self = Self {
        ratio: RATIO_COS_PHI_0_9,
    };

    /// The ratio.
    #[must_use]
    pub const fn ratio(&self) -> Decimal {
        self.ratio
    }
}

#[cfg(feature = "serde")]
#[derive(Serialize, Deserialize)]
struct RawLimit {
    #[serde(with = "crate::wire::decimal")]
    ratio: Decimal,
}

#[cfg(feature = "serde")]
impl Serialize for ReactiveLimit {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        RawLimit { ratio: self.ratio }.serialize(serializer)
    }
}

#[cfg(feature = "serde")]
impl<'de> Deserialize<'de> for ReactiveLimit {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = RawLimit::deserialize(deserializer)?;
        Self::new(raw.ratio)
            .ok_or_else(|| serde::de::Error::custom("a reactive ratio is not negative"))
    }
}

/// The Blindarbeit balance for one billing period: the two register totals
/// and the limit, every derived figure a method. Exact, no rounding.
///
/// ```rust
/// use metering::billing::reactive::{ReactiveBalance, ReactiveLimit};
/// use rust_decimal::dec;
///
/// // 100 000 kWh with 62 000 kvarh against the 50 % rule: 12 000 kvarh over.
/// let b = ReactiveBalance::new(dec!(100000), dec!(62000), ReactiveLimit::HALF).unwrap();
/// assert_eq!(b.freigrenze_kvarh(), dec!(50000.0));
/// assert_eq!(b.blindmehrarbeit_kvarh(), dec!(12000.0));
/// assert!(b.is_chargeable());
///
/// // Under cos φ = 0,9 less is free, so more is charged.
/// let strict = ReactiveBalance::new(dec!(100000), dec!(62000), ReactiveLimit::COS_PHI_0_9).unwrap();
/// assert_eq!(strict.blindmehrarbeit_kvarh(), dec!(13570.0000));
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize))]
#[non_exhaustive]
pub struct ReactiveBalance {
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::decimal"))]
    wirkarbeit_kwh: Decimal,
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::decimal"))]
    blindarbeit_kvarh: Decimal,
    limit: ReactiveLimit,
}

impl ReactiveBalance {
    /// The balance of a period from its Wirkarbeit (kWh) and Blindarbeit
    /// (kvarh) totals. `None` when a total is negative or `ratio × Wirkarbeit`
    /// does not fit a `Decimal`.
    #[must_use]
    pub fn new(
        wirkarbeit_kwh: Decimal,
        blindarbeit_kvarh: Decimal,
        limit: ReactiveLimit,
    ) -> Option<Self> {
        if wirkarbeit_kwh < Decimal::ZERO || blindarbeit_kvarh < Decimal::ZERO {
            return None;
        }
        limit.ratio.checked_mul(wirkarbeit_kwh)?;
        Some(Self {
            wirkarbeit_kwh,
            blindarbeit_kvarh,
            limit,
        })
    }

    /// Wirkarbeit drawn in the period (kWh).
    #[must_use]
    pub const fn wirkarbeit_kwh(&self) -> Decimal {
        self.wirkarbeit_kwh
    }

    /// Blindarbeit drawn in the period (kvarh).
    #[must_use]
    pub const fn blindarbeit_kvarh(&self) -> Decimal {
        self.blindarbeit_kvarh
    }

    /// The limit applied.
    #[must_use]
    pub const fn limit(&self) -> ReactiveLimit {
        self.limit
    }

    /// Blindarbeit admitted free of charge: `ratio × Wirkarbeit` (kvarh).
    #[must_use]
    pub fn freigrenze_kvarh(&self) -> Decimal {
        // Checked in `new`; both factors are non-negative.
        self.limit.ratio * self.wirkarbeit_kwh
    }

    /// The chargeable excess: `max(0, Blindarbeit − Freigrenze)` (kvarh).
    #[must_use]
    pub fn blindmehrarbeit_kvarh(&self) -> Decimal {
        (self.blindarbeit_kvarh - self.freigrenze_kvarh()).max(Decimal::ZERO)
    }

    /// `true` when there is a chargeable excess.
    #[must_use]
    pub fn is_chargeable(&self) -> bool {
        self.blindmehrarbeit_kvarh() > Decimal::ZERO
    }

    /// How much of the Freigrenze went unused (kvarh), never below zero — the
    /// figure a Blindleistungskompensation is sized against.
    #[must_use]
    pub fn headroom_kvarh(&self) -> Decimal {
        (self.freigrenze_kvarh() - self.blindarbeit_kvarh).max(Decimal::ZERO)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn balance(w: Decimal, b: Decimal, limit: ReactiveLimit) -> ReactiveBalance {
        ReactiveBalance::new(w, b, limit).unwrap()
    }

    #[test]
    fn the_excess_is_what_the_freigrenze_does_not_cover() {
        let b = balance(dec!(1000), dec!(600), ReactiveLimit::HALF);
        assert_eq!(b.freigrenze_kvarh(), dec!(500.0));
        assert_eq!(b.blindmehrarbeit_kvarh(), dec!(100.0));
        assert!(b.is_chargeable());
        assert_eq!(b.headroom_kvarh(), Decimal::ZERO);
    }

    #[test]
    fn a_compensated_load_pays_nothing_and_keeps_headroom() {
        let b = balance(dec!(1000), dec!(200), ReactiveLimit::HALF);
        assert_eq!(b.blindmehrarbeit_kvarh(), Decimal::ZERO);
        assert!(!b.is_chargeable());
        assert_eq!(b.headroom_kvarh(), dec!(300.0));
    }

    #[test]
    fn cos_phi_zero_nine_is_stricter_than_the_half_rule() {
        let half = balance(dec!(1000), dec!(600), ReactiveLimit::HALF);
        let strict = balance(dec!(1000), dec!(600), ReactiveLimit::COS_PHI_0_9);
        assert!(strict.blindmehrarbeit_kvarh() > half.blindmehrarbeit_kvarh());
        assert_eq!(strict.freigrenze_kvarh(), dec!(484.3000));
    }

    /// The outputs are derived, so they reconstruct the inputs exactly.
    #[test]
    fn derived_figures_cannot_contradict_the_inputs() {
        let b = balance(
            dec!(1234.567),
            dec!(1000.001),
            ReactiveLimit::new(dec!(0.3333)).unwrap(),
        );
        assert_eq!(b.freigrenze_kvarh(), dec!(1234.567) * dec!(0.3333));
        assert_eq!(
            b.blindmehrarbeit_kvarh() + b.freigrenze_kvarh(),
            b.blindarbeit_kvarh()
        );
        assert_eq!(b.limit().ratio(), dec!(0.3333));
    }

    /// No negative ratio and no negative register total: refused, not
    /// clamped.
    #[test]
    fn negative_inputs_are_refused() {
        assert_eq!(ReactiveLimit::new(dec!(-0.5)), None);
        assert_eq!(
            ReactiveBalance::new(dec!(-1), dec!(1), ReactiveLimit::HALF),
            None
        );
        assert_eq!(
            ReactiveBalance::new(dec!(1), dec!(-1), ReactiveLimit::HALF),
            None
        );
        assert_eq!(
            ReactiveBalance::new(Decimal::MAX, dec!(1), ReactiveLimit::new(dec!(2)).unwrap()),
            None
        );
    }

    #[test]
    fn the_published_conventions_are_the_stated_values() {
        assert_eq!(ReactiveLimit::COS_PHI_0_9.ratio(), dec!(0.4843));
        assert_eq!(ReactiveLimit::HALF.ratio(), dec!(0.5));
    }
}
