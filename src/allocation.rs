//! Splitting one quantity across many, with the residual reported.
//!
//! One arithmetic serves every allocation in this crate, because what makes an
//! allocation correct is not the key it uses but the identity that survives
//! the key:
//!
//! ```text
//! Σ allocated + residual = total        exactly, for every key, always
//! ```
//!
//! A settlement that loses a millionth of a kWh loses it into nobody's
//! account, and the loss is invisible until someone reconciles a year. So the
//! identity is a *theorem* rather than a check: shares are cut to
//! [`ALLOCATION_DP`] before anything is subtracted, and the residual is
//! defined as the difference rather than accumulated alongside it.
//!
//! | Varies | Fixed |
//! |---|---|
//! | how a weight becomes a share ([`AllocationBasis`]) | the share is cut to [`ALLOCATION_DP`], toward zero |
//! | whether a part has a ceiling ([`AllocationPart::capacity`]) | `allocated = min(capacity, share)`, never negative |
//! | what the parts mean | `residual = total − Σ allocated` |
//!
//! [`crate::compute_community_allocation`] is [`allocate`] applied once per
//! quarter-hour under § 42b EnWG; § 42c publishes no formula, so there the key
//! is a contractual input. Anything else with a pool and claims on it — sessions
//! behind one Netzanschluss, a heat allocation — uses the same call.
//!
//! **The residual is a quantity, not a rounding error.** Nothing here
//! redistributes it: no largest-remainder pass, no "give the rest to the
//! biggest". Under § 42b it is the generation that fed the public grid, and
//! turning it into a correction on an invoice would credit energy nobody
//! received.

use rust_decimal::{Decimal, RoundingStrategy};

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

// ── ALLOCATION_DP ─────────────────────────────────────────────────────────────

/// Decimal places an allocated **share** is cut to before it is capped.
///
/// A proportional key divides — `weight ÷ Σ weight × total` — and a `Decimal`
/// quotient carries up to 28 significant digits. Two things go wrong if that
/// reaches the output. It is not a *quantity*: no invoice, no MSCONS field and
/// no settlement system has a place for 0.333…3 kWh to twenty-seven places.
/// And it breaks the identity this module exists to guarantee, because
/// `total − Σ share` then needs more significant digits than a `Decimal` has
/// and rounds.
///
/// Six places is a millionth of a kWh: four orders of magnitude finer than
/// anything the market settles, and coarse enough that every derived value is
/// a number someone can write down.
///
/// The cut is **toward zero**, which is not a stylistic choice. Truncating can
/// only lower a share, so `Σ allocated ≤ Σ share ≤ total` survives it — and
/// under § 42b that inequality is the Abs. 5 pool ceiling, which therefore
/// stays a theorem rather than becoming a clamp.
pub const ALLOCATION_DP: u32 = 6;

/// A raw share, cut to [`ALLOCATION_DP`] places toward zero.
///
/// Applied where the share is *formed*, not where it is capped, so the value
/// reported as [`AllocatedPart::share`] is the one the cap was taken against —
/// which is what [`AllocatedPart::capped`], comparing the two, depends on.
#[must_use]
pub fn allocation_share(raw: Decimal) -> Decimal {
    raw.round_dp_with_strategy(ALLOCATION_DP, RoundingStrategy::ToZero)
}

// ── AllocationBasis ───────────────────────────────────────────────────────────

/// How a part's weight becomes its share of the total.
///
/// The two shapes the German market's published keys take, and between them
/// they cover every allocation in this crate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "SCREAMING_SNAKE_CASE"))]
pub enum AllocationBasis {
    /// The weight **is** the fraction: `share = weight × total`.
    ///
    /// Weights are absolute and are not normalised, so they must each be
    /// positive and sum to at most 1 — anything they do not claim stays in the
    /// residual. This is the § 42b constant key (UTILTS `CCI+ZG6` with
    /// `CAV+Z28`), where an under-subscribed community is normal and the
    /// unclaimed generation feeds the public grid.
    Fraction,
    /// The weight is **relative**: `share = (weight ÷ Σ weight) × total`.
    ///
    /// Weights are normalised, so their scale is irrelevant and only their
    /// ratios matter — consumption in kWh, session energy, connected load.
    /// With no capacities and a non-zero weight sum the residual is zero up to
    /// the [`ALLOCATION_DP`] cut. This is the § 42b proportional key (UTILTS
    /// `Z74` Divisionsquotient).
    #[default]
    Proportional,
}

impl AllocationBasis {
    /// Every variant, in declaration order.
    pub const ALL: [Self; 2] = [Self::Fraction, Self::Proportional];

    /// Stable DB/wire label. Matches the `serde` tag and
    /// [`FromStr`](std::str::FromStr) input.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Fraction => "FRACTION",
            Self::Proportional => "PROPORTIONAL",
        }
    }
}

crate::codes::string_codes! {
    AllocationBasis;
}

// ── AllocationError ───────────────────────────────────────────────────────────

/// Why an allocation could not be formed.
///
/// `#[non_exhaustive]`: a caller that wildcards an unrecognised variant still
/// behaves correctly — it reports a failure. That is the opposite of the
/// domain enums in this crate, which are exhaustive on purpose.
#[derive(Debug, thiserror::Error, PartialEq, Eq, Clone)]
#[non_exhaustive]
pub enum AllocationError {
    /// [`AllocationBasis::Fraction`] weights that are not each positive, or
    /// that claim more than the whole.
    ///
    /// Fractions are absolute, so a sum above 1 would allocate energy the pool
    /// does not hold — and a zero or negative fraction is a part that should
    /// not be in the set rather than one that receives nothing.
    #[error("allocation fractions total {sum} — each must be positive and the total at most 1")]
    InvalidFractions {
        /// The sum across the parts, or the single offending fraction when
        /// only one part was supplied.
        sum: Decimal,
    },

    /// A [`AllocationBasis::Proportional`] weight below zero.
    ///
    /// A negative weight does not merely take nothing: it shrinks the
    /// denominator and so **inflates** every other part's share. That is a
    /// silent over-allocation, which is exactly the failure this module is
    /// built to make impossible, so it is refused rather than absorbed.
    #[error("proportional weight for {key} is {weight} — a weight cannot be negative")]
    NegativeWeight {
        /// The part whose weight was negative.
        key: String,
        /// The offending weight.
        weight: Decimal,
    },

    /// A [`AllocationPart::capacity`] below zero.
    ///
    /// A ceiling of less than nothing is not a ceiling; it would make
    /// `allocated` clamp to zero while claiming the part could absorb a
    /// negative amount.
    #[error("capacity for {key} is {capacity} — a capacity cannot be negative")]
    NegativeCapacity {
        /// The part whose capacity was negative.
        key: String,
        /// The offending capacity.
        capacity: Decimal,
    },
}

// ── the parts ─────────────────────────────────────────────────────────────────

/// One claim on the pool.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct AllocationPart {
    /// What this part is — a participant id, a MaLo, a session reference.
    ///
    /// Opaque to this module: it is carried through to the result so a caller
    /// can join the row back to whatever it allocated across, and it is never
    /// parsed. Duplicate keys are not rejected; they simply produce two rows.
    pub key: String,
    /// How much of the total this part claims, read according to the
    /// [`AllocationBasis`].
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::decimal"))]
    pub weight: Decimal,
    /// The most this part can absorb, if it has a ceiling.
    ///
    /// `None` means uncapped, and the part receives its whole share. `Some(c)`
    /// makes `allocated = min(c, share)` — the `Pos()` operator of the BDEW
    /// Anwendungshilfe, where the ceiling is the participant's own
    /// consumption: nobody is credited more solar than they actually drew, and
    /// what the cap refuses stays in the residual.
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::decimal_option"))]
    pub capacity: Option<Decimal>,
}

impl AllocationPart {
    /// An uncapped part.
    #[must_use]
    pub fn new(key: impl Into<String>, weight: Decimal) -> Self {
        Self {
            key: key.into(),
            weight,
            capacity: None,
        }
    }

    /// The same part, ceilinged at what it can absorb (builder style).
    #[must_use]
    pub fn capped_at(mut self, capacity: Decimal) -> Self {
        self.capacity = Some(capacity);
        self
    }
}

/// What one part received.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct AllocatedPart {
    /// The part's key, carried through unchanged.
    pub key: String,
    /// The weight it was allocated on.
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::decimal"))]
    pub weight: Decimal,
    /// Its nominal share of the total, before any ceiling, cut to
    /// [`ALLOCATION_DP`] places toward zero.
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::decimal"))]
    pub share: Decimal,
    /// What it actually received: `min(capacity, share)`, never below zero.
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::decimal"))]
    pub allocated: Decimal,
}

impl AllocatedPart {
    /// `true` when the ceiling bound the share, so the difference went to the
    /// residual instead.
    #[must_use]
    pub fn capped(&self) -> bool {
        self.share > self.allocated
    }

    /// The part of the share the ceiling refused — `share − allocated`.
    #[must_use]
    pub fn forgone(&self) -> Decimal {
        self.share - self.allocated
    }
}

/// One completed allocation.
///
/// `Σ allocated + residual == total` holds exactly, on every row this type is
/// ever constructed with; `tests/allocation_invariants.rs` holds it under
/// proptest.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct AllocationRow {
    /// The pool that was divided.
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::decimal"))]
    pub total: Decimal,
    /// Per part, in the order they were supplied.
    pub parts: Vec<AllocatedPart>,
    /// What the parts did not take: `total − Σ allocated`.
    ///
    /// Under § 42b this is the generation that fed the public grid. It is a
    /// **quantity**, not a rounding error, which is why nothing here
    /// redistributes it.
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::decimal"))]
    pub residual: Decimal,
}

impl AllocationRow {
    /// The sum across the parts.
    #[must_use]
    pub fn allocated(&self) -> Decimal {
        self.parts.iter().map(|p| p.allocated).sum()
    }

    /// One part, by key. The first match, if a key was supplied twice.
    #[must_use]
    pub fn part(&self, key: &str) -> Option<&AllocatedPart> {
        self.parts.iter().find(|p| p.key == key)
    }
}

// ── allocate ──────────────────────────────────────────────────────────────────

/// Check that a set of parts forms a usable key, without dividing anything.
///
/// [`allocate`] runs this first, so calling it separately buys one thing: a
/// persisted allocation rule can be rejected when it is **stored** rather than
/// on the first interval it is applied to. An over-subscribed § 42b community
/// is a contract defect, and finding it a month later in a settlement run is
/// finding it in the wrong place.
///
/// # Errors
///
/// The same three [`allocate`] returns, for the same reasons.
///
/// ```rust
/// use metering::allocation::{AllocationBasis, AllocationPart, validate_key};
/// use rust_decimal::dec;
///
/// let key = vec![
///     AllocationPart::new("T1", dec!(0.6)),
///     AllocationPart::new("T2", dec!(0.5)),
/// ];
/// assert!(validate_key(&key, AllocationBasis::Fraction).is_err(), "110 % subscribed");
/// assert!(validate_key(&key, AllocationBasis::Proportional).is_ok(), "ratios, not shares");
/// ```
pub fn validate_key(
    parts: &[AllocationPart],
    basis: AllocationBasis,
) -> Result<(), AllocationError> {
    for part in parts {
        if let Some(capacity) = part.capacity
            && capacity < Decimal::ZERO
        {
            return Err(AllocationError::NegativeCapacity {
                key: part.key.clone(),
                capacity,
            });
        }
    }

    match basis {
        AllocationBasis::Fraction => {
            let sum: Decimal = parts.iter().map(|p| p.weight).sum();
            if parts.iter().any(|p| p.weight <= Decimal::ZERO) || sum > Decimal::ONE {
                return Err(AllocationError::InvalidFractions { sum });
            }
        }
        AllocationBasis::Proportional => {
            if let Some(part) = parts.iter().find(|p| p.weight < Decimal::ZERO) {
                return Err(AllocationError::NegativeWeight {
                    key: part.key.clone(),
                    weight: part.weight,
                });
            }
        }
    }
    Ok(())
}

/// Divide `total` across `parts`, and report what was left over.
///
/// ```text
/// share_i     = cut(weight_i × total)                 Fraction
///             = cut(weight_i ÷ Σ weight × total)      Proportional
/// allocated_i = min(capacity_i, share_i), never below zero
/// residual    = total − Σ allocated
/// ```
///
/// `cut` is [`allocation_share`]. With a zero weight sum under
/// [`Proportional`](AllocationBasis::Proportional) every share is zero and the
/// whole total is the residual, so a set of parts that all consumed nothing is
/// an ordinary answer rather than a division by zero.
///
/// # Errors
///
/// - [`InvalidFractions`](AllocationError::InvalidFractions) —
///   [`Fraction`](AllocationBasis::Fraction) weights that are not each positive
///   or that sum above 1.
/// - [`NegativeWeight`](AllocationError::NegativeWeight) — a negative
///   [`Proportional`](AllocationBasis::Proportional) weight.
/// - [`NegativeCapacity`](AllocationError::NegativeCapacity) — a negative
///   ceiling.
///
/// ```rust
/// use metering::allocation::{AllocationBasis, AllocationPart, allocate};
/// use rust_decimal::dec;
///
/// // 12 kWh arrived at the Übergabestelle in this quarter-hour, and three
/// // sessions ran behind it. Each is capped at what its own meter recorded.
/// let row = allocate(
///     dec!(12),
///     vec![
///         AllocationPart::new("S1", dec!(6)).capped_at(dec!(6)),
///         AllocationPart::new("S2", dec!(3)).capped_at(dec!(3)),
///         AllocationPart::new("S3", dec!(3)).capped_at(dec!(1)), // cable pulled early
///     ],
///     AllocationBasis::Proportional,
/// )?;
///
/// assert_eq!(row.part("S3").unwrap().share, dec!(3));
/// assert_eq!(row.part("S3").unwrap().allocated, dec!(1));
/// assert!(row.part("S3").unwrap().capped());
///
/// // The identity, which is the whole point.
/// assert_eq!(row.allocated() + row.residual, row.total);
/// assert_eq!(row.residual, dec!(2), "two kWh nobody claimed");
/// # Ok::<(), metering::allocation::AllocationError>(())
/// ```
pub fn allocate(
    total: Decimal,
    parts: Vec<AllocationPart>,
    basis: AllocationBasis,
) -> Result<AllocationRow, AllocationError> {
    validate_key(&parts, basis)?;
    let weight_sum: Decimal = parts.iter().map(|p| p.weight).sum();

    let allocated_parts: Vec<AllocatedPart> = parts
        .into_iter()
        .map(|part| {
            let share = allocation_share(match basis {
                AllocationBasis::Fraction => part.weight * total,
                AllocationBasis::Proportional if weight_sum > Decimal::ZERO => {
                    part.weight / weight_sum * total
                }
                AllocationBasis::Proportional => Decimal::ZERO,
            });
            let allocated = cap(share, part.capacity);
            AllocatedPart {
                key: part.key,
                weight: part.weight,
                share,
                allocated,
            }
        })
        .collect();

    let allocated: Decimal = allocated_parts.iter().map(|p| p.allocated).sum();
    Ok(AllocationRow {
        total,
        parts: allocated_parts,
        residual: total - allocated,
    })
}

/// `min(capacity, share)`, floored at zero.
///
/// One place, so the `Pos()` cap of the BDEW Anwendungshilfe is applied once
/// and every entry point reads the same arithmetic. `share` is expected to
/// have been through [`allocation_share`] already; the capacity is the
/// caller's own measurement and passes through untouched, so a part whose
/// whole claim is covered receives exactly what it claimed, however many
/// decimal places its meter delivered.
pub(crate) fn cap(share: Decimal, capacity: Option<Decimal>) -> Decimal {
    match capacity {
        Some(c) => share.min(c).max(Decimal::ZERO),
        None => share.max(Decimal::ZERO),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal::dec;

    fn parts(spec: &[(&str, Decimal, Option<Decimal>)]) -> Vec<AllocationPart> {
        spec.iter()
            .map(|(k, w, c)| AllocationPart {
                key: (*k).to_owned(),
                weight: *w,
                capacity: *c,
            })
            .collect()
    }

    #[test]
    fn proportional_splits_and_conserves() {
        let row = allocate(
            dec!(10),
            parts(&[("a", dec!(1), None), ("b", dec!(2), None)]),
            AllocationBasis::Proportional,
        )
        .unwrap();
        assert_eq!(row.parts[0].allocated, dec!(3.333333));
        assert_eq!(row.parts[1].allocated, dec!(6.666666));
        assert_eq!(row.allocated() + row.residual, row.total);
    }

    /// The repeating quotient is the case the cut exists for: without it the
    /// subtraction that forms the residual would round.
    #[test]
    fn a_repeating_quotient_still_conserves() {
        let row = allocate(
            dec!(1),
            parts(&[
                ("a", dec!(1), None),
                ("b", dec!(1), None),
                ("c", dec!(1), None),
            ]),
            AllocationBasis::Proportional,
        )
        .unwrap();
        assert_eq!(row.allocated(), dec!(0.999999));
        assert_eq!(row.residual, dec!(0.000001));
        assert_eq!(row.allocated() + row.residual, row.total);
    }

    #[test]
    fn a_zero_weight_sum_leaves_everything_in_the_residual() {
        let row = allocate(
            dec!(7),
            parts(&[("a", Decimal::ZERO, None), ("b", Decimal::ZERO, None)]),
            AllocationBasis::Proportional,
        )
        .unwrap();
        assert!(row.parts.iter().all(|p| p.share.is_zero()));
        assert_eq!(row.residual, dec!(7));
    }

    #[test]
    fn a_ceiling_moves_the_difference_to_the_residual() {
        let row = allocate(
            dec!(10),
            parts(&[
                ("a", dec!(1), Some(dec!(2))),
                ("b", dec!(1), Some(dec!(99))),
            ]),
            AllocationBasis::Proportional,
        )
        .unwrap();
        assert_eq!(row.parts[0].allocated, dec!(2));
        assert!(row.parts[0].capped());
        assert_eq!(row.parts[0].forgone(), dec!(3));
        assert_eq!(row.parts[1].allocated, dec!(5));
        assert_eq!(row.residual, dec!(3));
        assert_eq!(row.allocated() + row.residual, row.total);
    }

    #[test]
    fn fractions_are_absolute_and_the_rest_is_residual() {
        let row = allocate(
            dec!(8),
            parts(&[("a", dec!(0.25), None)]),
            AllocationBasis::Fraction,
        )
        .unwrap();
        assert_eq!(row.parts[0].allocated, dec!(2.00));
        assert_eq!(row.residual, dec!(6.00));
    }

    #[test]
    fn over_subscribed_fractions_are_refused() {
        assert_eq!(
            allocate(
                dec!(8),
                parts(&[("a", dec!(0.7), None), ("b", dec!(0.5), None)]),
                AllocationBasis::Fraction,
            ),
            Err(AllocationError::InvalidFractions { sum: dec!(1.2) })
        );
    }

    #[test]
    fn a_zero_fraction_is_refused() {
        assert!(matches!(
            allocate(
                dec!(8),
                parts(&[("a", Decimal::ZERO, None)]),
                AllocationBasis::Fraction,
            ),
            Err(AllocationError::InvalidFractions { .. })
        ));
    }

    /// A negative weight shrinks the denominator, so it would silently inflate
    /// every other part. It is refused rather than absorbed.
    #[test]
    fn a_negative_proportional_weight_is_refused() {
        assert_eq!(
            allocate(
                dec!(8),
                parts(&[("a", dec!(-1), None), ("b", dec!(3), None)]),
                AllocationBasis::Proportional,
            ),
            Err(AllocationError::NegativeWeight {
                key: "a".to_owned(),
                weight: dec!(-1),
            })
        );
    }

    #[test]
    fn a_negative_capacity_is_refused() {
        assert!(matches!(
            allocate(
                dec!(8),
                parts(&[("a", dec!(1), Some(dec!(-1)))]),
                AllocationBasis::Proportional,
            ),
            Err(AllocationError::NegativeCapacity { .. })
        ));
    }

    /// Order changes the row order and nothing else — the shares, the caps and
    /// the residual are all order-independent.
    #[test]
    fn the_result_does_not_depend_on_the_order_of_the_parts() {
        let spec = [
            ("a", dec!(1), Some(dec!(2))),
            ("b", dec!(2), None),
            ("c", dec!(3), Some(dec!(1))),
        ];
        let forward = allocate(dec!(13), parts(&spec), AllocationBasis::Proportional).unwrap();
        let mut reversed = spec;
        reversed.reverse();
        let backward = allocate(dec!(13), parts(&reversed), AllocationBasis::Proportional).unwrap();

        assert_eq!(forward.residual, backward.residual);
        for part in &forward.parts {
            assert_eq!(
                Some(part.allocated),
                backward.part(&part.key).map(|p| p.allocated)
            );
        }
    }

    #[test]
    fn an_empty_set_leaves_the_whole_total_as_residual() {
        let row = allocate(dec!(5), Vec::new(), AllocationBasis::Proportional).unwrap();
        assert!(row.parts.is_empty());
        assert_eq!(row.residual, dec!(5));
    }
}
