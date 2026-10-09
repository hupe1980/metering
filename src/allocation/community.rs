//! § 42b EnWG Gemeinschaftliche Gebäudeversorgung: one plant's generation
//! divided across the participants, per interval, with the rest reported.
//!
//! Start at [`allocate`]; a tenant's view is [`AllocationRow::share`], and its
//! [`net_grid_draw`](Share::net_grid_draw) equals the Berechnungsformel of
//! [`Formula::constant_share`](crate::allocation::formula::Formula::constant_share)
//! and [`Formula::proportional_share`](crate::allocation::formula::Formula::proportional_share).
//!
//! ```text
//! pool          = max(0, generation)
//! share_i       = q_i × pool                       q_i from the key
//! allocated_i   = min(max(0, consumption_i), share_i)   the Pos() cap
//! residual      = pool − Σ allocated_i             exactly
//! ```
//!
//! | [`AllocationKey`] | `q_i` | Source |
//! |---|---|---|
//! | [`Constant`](AllocationKey::Constant) | the agreed [`SplitFactor`] | UTILTS ZG6, AWH Beispiel 1 |
//! | [`Proportional`](AllocationKey::Proportional) | `cut(c_i ÷ Σ c)` | AWH Beispiel 3 |
//! | [`EqualShares`](AllocationKey::EqualShares) | `cut(1 ÷ n)` | § 42b Abs. 5 Satz 3 EnWG |
//! | [`Cascading`](AllocationKey::Cascading) | relative weights, re-offered | no source; contractual arithmetic |
//!
//! `cut` is [`FORMULA_QUOTIENT_DP`](crate::precision::FORMULA_QUOTIENT_DP)
//! places toward zero, so the `q_i` sum to at most 1 and `Σ allocated ≤ pool`.
//! The residual is the generation that fed the public grid; nothing
//! redistributes it.
//!
//! § 42b Abs. 5 caps the pool, verbatim: *"wobei die rechnerisch aufteilbare
//! Strommenge begrenzt ist auf die Strommenge, die innerhalb eines
//! 15-Minuten-Zeitintervalls in der Solaranlage erzeugt oder von allen
//! teilnehmenden Letztverbrauchern verbraucht wird, je nachdem welche dieser
//! Strommengen geringer ist."* The per-participant `Pos()` cap implies it:
//! `Σ min(cᵢ, shareᵢ) ≤ min(Σ cᵢ, pool)`.
//!
//! A negative consumption or generation is clamped to zero for that interval
//! and reported by [`Share::clamped`] / [`AllocationRow::clamped`]; it never
//! aborts the run.

use std::collections::{BTreeMap, BTreeSet};

use rust_decimal::Decimal;
use time::OffsetDateTime;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use crate::allocation::formula::{SplitFactor, quotient, same_grid};
use crate::ids::MeloId;
use crate::series::Series;
use crate::series::interval::{MeterInterval, QualityFlag};

// ── the key ───────────────────────────────────────────────────────────────────

/// How the generation is divided among the participants.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(
    feature = "serde",
    serde(tag = "kind", rename_all = "SCREAMING_SNAKE_CASE")
)]
pub enum AllocationKey {
    /// A fixed fraction per participant — the Aufteilungsfaktor (UTILTS ZG6);
    /// one per participant, summing to at most 1.
    Constant {
        /// Participant → fraction of the generation.
        fractions: BTreeMap<MeloId, SplitFactor>,
    },
    /// Proportional to each participant's consumption in the interval.
    Proportional,
    /// Equal shares — [§ 42b Abs. 5 Satz 3 EnWG]: *"Im Zweifel ist die durch
    /// die Gebäudestromanlage erzeugte elektrische Energie zu gleichen Teilen
    /// auf die teilnehmenden Letztverbraucher zu verteilen."* Each share is
    /// still capped at the participant's consumption.
    EqualShares,
    /// Relative weights ≥ 0, one per participant; what a cap refuses is
    /// re-offered to the participants still open until nothing moves. A
    /// contractual key: § 42b Abs. 5 Satz 2 and § 42c Abs. 3 Nr. 2 EnWG leave
    /// the key to the agreement.
    Cascading {
        /// Participant → relative weight.
        #[cfg_attr(feature = "serde", serde(with = "crate::wire::decimal_map"))]
        weights: BTreeMap<MeloId, Decimal>,
    },
}

// ── the rows ──────────────────────────────────────────────────────────────────

/// What one participant received in one interval.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[non_exhaustive]
pub struct Share {
    /// The participant.
    pub participant: MeloId,
    /// The metered consumption, as supplied — negative when the meter said so.
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::decimal"))]
    pub consumption: Decimal,
    /// The nominal share of the pool, before the `Pos()` cap. For
    /// [`Cascading`](AllocationKey::Cascading), the first pass's offer.
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::decimal"))]
    pub share: Decimal,
    /// What was credited: `min(max(0, consumption), share)`, or for
    /// [`Cascading`](AllocationKey::Cascading) the sum over every pass.
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::decimal"))]
    pub allocated: Decimal,
}

impl Share {
    /// What the participant still drew from the public grid:
    /// `max(0, consumption) − allocated` — the MaLo Verbrauch of the AWH.
    #[must_use]
    pub fn net_grid_draw(&self) -> Decimal {
        self.consumption.max(Decimal::ZERO) - self.allocated
    }

    /// `true` when the participant's own consumption bounded what they were
    /// credited (§ 42b Abs. 5 Satz 4), so part of the share stayed residual.
    #[must_use]
    pub fn capped(&self) -> bool {
        self.allocated < self.share
    }

    /// `true` when the metered consumption was negative and counted as zero.
    #[must_use]
    pub fn clamped(&self) -> bool {
        self.consumption < Decimal::ZERO
    }
}

/// One interval of a community allocation.
///
/// `Σ allocated + residual == pool()` holds exactly on every row.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[non_exhaustive]
pub struct AllocationRow {
    /// Interval start.
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::rfc3339"))]
    pub from: OffsetDateTime,
    /// Interval end.
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::rfc3339"))]
    pub to: OffsetDateTime,
    /// The plant's metered generation, as supplied.
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::decimal"))]
    pub generation: Decimal,
    /// Per participant, in the order supplied.
    pub shares: Vec<Share>,
    /// The generation no participant took, which fed the public grid:
    /// `pool() − Σ allocated`.
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::decimal"))]
    pub residual: Decimal,
    /// The worst quality of the plant and every participant.
    pub quality: QualityFlag,
}

impl AllocationRow {
    /// The pool that was divided: `max(0, generation)`.
    #[must_use]
    pub fn pool(&self) -> Decimal {
        self.generation.max(Decimal::ZERO)
    }

    /// `true` when the metered generation was negative and counted as zero.
    #[must_use]
    pub fn clamped(&self) -> bool {
        self.generation < Decimal::ZERO
    }

    /// The energy credited across every participant.
    #[must_use]
    pub fn allocated(&self) -> Decimal {
        self.shares.iter().map(|s| s.allocated).sum()
    }

    /// One participant's share — the single-tenant view of this row.
    #[must_use]
    pub fn share(&self, participant: &MeloId) -> Option<&Share> {
        self.shares.iter().find(|s| s.participant == *participant)
    }
}

// ── errors ────────────────────────────────────────────────────────────────────

/// Why an allocation could not be run.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum AllocationError {
    /// A participant is listed twice.
    #[error("{participant} is listed twice")]
    DuplicateParticipant {
        /// The participant.
        participant: MeloId,
    },
    /// A participant has no entry in a [`Constant`](AllocationKey::Constant)
    /// or [`Cascading`](AllocationKey::Cascading) key.
    #[error("{participant} has no entry in the allocation key")]
    NotInKey {
        /// The participant.
        participant: MeloId,
    },
    /// The key names a Messlokation that is not a participant.
    #[error("the allocation key names {melo}, which is not a participant")]
    NotAParticipant {
        /// The stray key entry.
        melo: MeloId,
    },
    /// Constant fractions summing above 1.
    #[error("the constant fractions sum to {sum}, above 1")]
    FractionsExceedOne {
        /// The sum.
        sum: Decimal,
    },
    /// A negative cascading weight.
    #[error("the weight of {participant} is {weight}; a weight cannot be negative")]
    NegativeWeight {
        /// The participant.
        participant: MeloId,
        /// The weight.
        weight: Decimal,
    },
    /// A participant's series is not on the plant's grid.
    #[error("the series of {participant} is not on the plant's grid (first difference at {at:?})")]
    Misaligned {
        /// The participant.
        participant: MeloId,
        /// The first differing instant; `None` when only the resolution or
        /// day boundary differs.
        at: Option<OffsetDateTime>,
    },
    /// A value exceeded `Decimal`'s range.
    #[error("the allocation overflows at {at}")]
    Overflow {
        /// The interval start.
        at: OffsetDateTime,
    },
}

// ── allocate ──────────────────────────────────────────────────────────────────

/// Allocate `plant`'s generation across `participants` under `key`, one row
/// per interval (arithmetic in the [module docs](self)). Every series must be
/// on the plant's grid: same resolution, day boundary and intervals.
///
/// # Errors
///
/// An [`AllocationError`] for an unusable key or a misaligned series, before
/// any interval is computed; [`Overflow`](AllocationError::Overflow) naming the
/// instant. A negative value is never an error.
///
/// ```rust
/// use metering::allocation::community::{AllocationKey, allocate};
/// use metering::{DayBoundary, MeloId, MeterInterval, QualityFlag, Resolution, Series};
/// use rust_decimal::dec;
/// use time::macros::datetime;
///
/// let one = |kwh| -> Result<Series, Box<dyn std::error::Error>> {
///     let iv = MeterInterval::quarter_hour(datetime!(2026-06-01 12:00 UTC), kwh, QualityFlag::Measured)?;
///     Ok(Series::new(Resolution::QUARTER_HOUR, DayBoundary::Strom, vec![iv])?)
/// };
/// let t1: MeloId = "DE0001234567890000000000000000001".parse()?;
/// let t2: MeloId = "DE0001234567890000000000000000002".parse()?;
/// let (plant, a, b) = (one(dec!(10))?, one(dec!(1))?, one(dec!(3))?);
///
/// let rows = allocate(&plant, &[(t1, &a), (t2, &b)], &AllocationKey::Proportional)?;
/// let row = &rows[0];
/// assert_eq!(row.allocated(), dec!(4));        // both capped by their own draw
/// assert_eq!(row.residual, dec!(6));           // fed the grid
/// assert!(row.share(&t1).unwrap().capped());
/// assert_eq!(row.allocated() + row.residual, row.pool());
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub fn allocate(
    plant: &Series,
    participants: &[(MeloId, &Series)],
    key: &AllocationKey,
) -> Result<Vec<AllocationRow>, AllocationError> {
    let ids = check_key(participants, key)?;
    for (participant, series) in participants {
        same_grid(plant, series).map_err(|at| AllocationError::Misaligned {
            participant: *participant,
            at,
        })?;
    }

    let mut rows = Vec::with_capacity(plant.len());
    for (k, generation) in plant.iter().enumerate() {
        let at = generation.from();
        let metered: Vec<&MeterInterval> = participants
            .iter()
            .filter_map(|(_, s)| s.as_slice().get(k))
            .collect();
        let consumption: Vec<Decimal> = metered.iter().map(|iv| iv.value()).collect();
        let pool = generation.value().max(Decimal::ZERO);
        let (shares, allocated) =
            divide(pool, &ids, &consumption, key).ok_or(AllocationError::Overflow { at })?;
        let taken = allocated
            .iter()
            .try_fold(Decimal::ZERO, |acc, a| acc.checked_add(*a))
            .ok_or(AllocationError::Overflow { at })?;
        rows.push(AllocationRow {
            from: at,
            to: generation.to(),
            generation: generation.value(),
            shares: ids
                .iter()
                .zip(consumption)
                .zip(shares.into_iter().zip(allocated))
                .map(|((participant, consumption), (share, allocated))| Share {
                    participant: *participant,
                    consumption,
                    share,
                    allocated,
                })
                .collect(),
            residual: pool - taken,
            quality: metered
                .iter()
                .fold(generation.quality(), |q, iv| q.worse_of(iv.quality())),
        });
    }
    Ok(rows)
}

/// The participants in order, once the key is checked against them.
fn check_key(
    participants: &[(MeloId, &Series)],
    key: &AllocationKey,
) -> Result<Vec<MeloId>, AllocationError> {
    let mut seen = BTreeSet::new();
    for (participant, _) in participants {
        if !seen.insert(*participant) {
            return Err(AllocationError::DuplicateParticipant {
                participant: *participant,
            });
        }
    }
    let named: Option<Vec<MeloId>> = match key {
        AllocationKey::Constant { fractions } => Some(fractions.keys().copied().collect()),
        AllocationKey::Cascading { weights } => Some(weights.keys().copied().collect()),
        AllocationKey::Proportional | AllocationKey::EqualShares => None,
    };
    if let Some(named) = named {
        if let Some(stray) = named.iter().find(|m| !seen.contains(m)) {
            return Err(AllocationError::NotAParticipant { melo: *stray });
        }
        if let Some((missing, _)) = participants.iter().find(|(p, _)| !named.contains(p)) {
            return Err(AllocationError::NotInKey {
                participant: *missing,
            });
        }
    }
    match key {
        AllocationKey::Constant { fractions } => {
            let sum: Decimal = fractions.values().map(|f| f.get()).sum();
            if sum > Decimal::ONE {
                return Err(AllocationError::FractionsExceedOne { sum });
            }
        }
        AllocationKey::Cascading { weights } => {
            if let Some((participant, weight)) = weights.iter().find(|(_, w)| **w < Decimal::ZERO) {
                return Err(AllocationError::NegativeWeight {
                    participant: *participant,
                    weight: *weight,
                });
            }
        }
        AllocationKey::Proportional | AllocationKey::EqualShares => {}
    }
    Ok(participants.iter().map(|(p, _)| *p).collect())
}

/// One interval: the nominal shares and what was allocated, per participant.
/// `None` on overflow.
fn divide(
    pool: Decimal,
    ids: &[MeloId],
    consumption: &[Decimal],
    key: &AllocationKey,
) -> Option<(Vec<Decimal>, Vec<Decimal>)> {
    let caps: Vec<Decimal> = consumption
        .iter()
        .map(|c| (*c).max(Decimal::ZERO))
        .collect();
    let fractions: Vec<Decimal> = match key {
        AllocationKey::Cascading { weights } => {
            let weights: Vec<Decimal> = ids
                .iter()
                .map(|id| weights.get(id).copied().unwrap_or(Decimal::ZERO))
                .collect();
            return cascade(pool, &weights, &caps);
        }
        AllocationKey::Constant { fractions } => ids
            .iter()
            .map(|id| fractions.get(id).map_or(Decimal::ZERO, |f| f.get()))
            .collect(),
        AllocationKey::Proportional => {
            let total = caps
                .iter()
                .try_fold(Decimal::ZERO, |acc, c| acc.checked_add(*c))?;
            caps.iter()
                .map(|c| {
                    if total.is_zero() {
                        Some(Decimal::ZERO)
                    } else {
                        quotient(*c, total)
                    }
                })
                .collect::<Option<_>>()?
        }
        AllocationKey::EqualShares => {
            let q = quotient(Decimal::ONE, Decimal::from(ids.len().max(1)))?;
            vec![q; ids.len()]
        }
    };
    let shares: Vec<Decimal> = fractions
        .iter()
        .map(|q| q.checked_mul(pool))
        .collect::<Option<_>>()?;
    let allocated = shares.iter().zip(&caps).map(|(s, c)| *s.min(c)).collect();
    Some((shares, allocated))
}

/// The cascading key: offer `cut(wᵢ × remaining ÷ Σ w_open)` to every open
/// participant, close those their cap stops, and repeat until nothing moves.
fn cascade(
    pool: Decimal,
    weights: &[Decimal],
    caps: &[Decimal],
) -> Option<(Vec<Decimal>, Vec<Decimal>)> {
    let n = weights.len();
    let mut allocated = vec![Decimal::ZERO; n];
    let mut nominal = vec![Decimal::ZERO; n];
    let mut open = vec![true; n];
    let mut remaining = pool;
    let mut first = true;
    loop {
        let total = weights
            .iter()
            .zip(&open)
            .filter(|(_, o)| **o)
            .try_fold(Decimal::ZERO, |acc, (w, _)| acc.checked_add(*w))?;
        if remaining <= Decimal::ZERO || total <= Decimal::ZERO {
            break;
        }
        let mut moved = Decimal::ZERO;
        for i in 0..n {
            let (Some(w), Some(cap), Some(is_open)) =
                (weights.get(i), caps.get(i), open.get_mut(i))
            else {
                continue;
            };
            let offered = if *is_open {
                quotient(w.checked_mul(remaining)?, total)?
            } else {
                Decimal::ZERO
            };
            if first && let Some(slot) = nominal.get_mut(i) {
                *slot = offered;
            }
            let Some(taken) = allocated.get_mut(i) else {
                continue;
            };
            if !*is_open {
                continue;
            }
            let give = offered.min((*cap - *taken).max(Decimal::ZERO));
            if give < offered {
                *is_open = false;
            }
            *taken += give;
            moved += give;
        }
        first = false;
        if moved <= Decimal::ZERO {
            break;
        }
        remaining -= moved;
    }
    Some((nominal, allocated))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::time::calendar::DayBoundary;
    use crate::time::resolution::Resolution;
    use rust_decimal::dec;
    use time::Duration;
    use time::macros::datetime;

    const T0: OffsetDateTime = datetime!(2026-06-01 0:00 UTC);

    fn qh(values: &[Decimal]) -> Series {
        let ivs = values
            .iter()
            .enumerate()
            .map(|(i, v)| {
                MeterInterval::quarter_hour(
                    T0 + Duration::minutes(15 * i as i64),
                    *v,
                    QualityFlag::Measured,
                )
                .unwrap()
            })
            .collect();
        Series::new(Resolution::QUARTER_HOUR, DayBoundary::Strom, ivs).unwrap()
    }

    fn melo(n: u32) -> MeloId {
        format!("DE0001234567890000000000000{n:06}")
            .parse()
            .unwrap()
    }

    fn fraction(f: Decimal) -> SplitFactor {
        SplitFactor::new(f).unwrap()
    }

    #[test]
    fn a_constant_key_credits_its_fraction_capped_at_consumption() {
        let (plant, a, b) = (qh(&[dec!(10)]), qh(&[dec!(5)]), qh(&[dec!(2)]));
        let key = AllocationKey::Constant {
            fractions: BTreeMap::from([
                (melo(1), fraction(dec!(0.1))),
                (melo(2), fraction(dec!(0.9))),
            ]),
        };
        let row = &allocate(&plant, &[(melo(1), &a), (melo(2), &b)], &key).unwrap()[0];
        let (s1, s2) = (row.share(&melo(1)).unwrap(), row.share(&melo(2)).unwrap());
        assert_eq!(
            (s1.share, s1.allocated, s1.net_grid_draw()),
            (dec!(1), dec!(1), dec!(4))
        );
        assert_eq!(
            (s2.share, s2.allocated, s2.net_grid_draw()),
            (dec!(9), dec!(2), dec!(0))
        );
        assert!(s2.capped() && !s1.capped());
        assert_eq!(
            row.residual,
            dec!(7),
            "what the 90 % tenant could not use fed the grid"
        );
    }

    #[test]
    fn equal_shares_is_the_statutory_doubt_case() {
        let (plant, a, b, c) = (
            qh(&[dec!(9)]),
            qh(&[dec!(10)]),
            qh(&[dec!(1)]),
            qh(&[dec!(10)]),
        );
        let rows = allocate(
            &plant,
            &[(melo(1), &a), (melo(2), &b), (melo(3), &c)],
            &AllocationKey::EqualShares,
        )
        .unwrap();
        let row = &rows[0];
        // q = cut(1/3) = 0.333333, share = 2.999997 each; the second is capped.
        assert_eq!(row.share(&melo(1)).unwrap().allocated, dec!(2.999997));
        assert_eq!(row.share(&melo(2)).unwrap().allocated, dec!(1));
        assert_eq!(row.allocated() + row.residual, dec!(9));
    }

    #[test]
    fn a_cascade_re_offers_what_a_cap_refused() {
        let (plant, a, b, c) = (
            qh(&[dec!(9)]),
            qh(&[dec!(1)]),
            qh(&[dec!(10)]),
            qh(&[dec!(10)]),
        );
        let key = AllocationKey::Cascading {
            weights: BTreeMap::from([(melo(1), dec!(1)), (melo(2), dec!(1)), (melo(3), dec!(1))]),
        };
        let row =
            &allocate(&plant, &[(melo(1), &a), (melo(2), &b), (melo(3), &c)], &key).unwrap()[0];
        assert_eq!(row.share(&melo(1)).unwrap().allocated, dec!(1));
        assert_eq!(row.share(&melo(2)).unwrap().allocated, dec!(4));
        assert_eq!(row.share(&melo(3)).unwrap().allocated, dec!(4));
        assert_eq!(row.residual, dec!(0));
    }

    #[test]
    fn nobody_consuming_leaves_everything_residual() {
        let (plant, a) = (qh(&[dec!(3.5)]), qh(&[dec!(0)]));
        let row = &allocate(&plant, &[(melo(1), &a)], &AllocationKey::Proportional).unwrap()[0];
        assert_eq!((row.allocated(), row.residual), (dec!(0), dec!(3.5)));
    }

    /// A negative value is clamped in its own interval only.
    #[test]
    fn a_negative_consumption_is_clamped_and_never_aborts_the_run() {
        let mut values = vec![dec!(1); 96];
        values[40] = dec!(-0.001);
        let (plant, tenant) = (qh(&[dec!(2); 96]), qh(&values));
        let rows = allocate(&plant, &[(melo(1), &tenant)], &AllocationKey::Proportional).unwrap();
        assert_eq!(rows.len(), 96);
        let bad = &rows[40].shares[0];
        assert!(bad.clamped());
        assert_eq!((bad.allocated, bad.net_grid_draw()), (dec!(0), dec!(0)));
        assert_eq!(rows[40].residual, dec!(2));
        assert_eq!(rows[39].shares[0].allocated, dec!(1));
    }

    #[test]
    fn a_negative_generation_is_an_empty_pool() {
        let (plant, a) = (qh(&[dec!(-0.2)]), qh(&[dec!(1)]));
        let row = &allocate(&plant, &[(melo(1), &a)], &AllocationKey::EqualShares).unwrap()[0];
        assert!(row.clamped());
        assert_eq!(
            (row.pool(), row.allocated(), row.residual),
            (dec!(0), dec!(0), dec!(0))
        );
    }

    /// A 15-minute and an hourly series are never added slot by slot.
    #[test]
    fn a_participant_on_another_grid_is_refused() {
        let plant = qh(&[dec!(1); 4]);
        let hourly = Series::new(
            Resolution::Hour,
            DayBoundary::Strom,
            vec![MeterInterval::hour(T0, dec!(4), QualityFlag::Measured).unwrap()],
        )
        .unwrap();
        assert_eq!(
            allocate(&plant, &[(melo(1), &hourly)], &AllocationKey::Proportional),
            Err(AllocationError::Misaligned {
                participant: melo(1),
                at: Some(T0)
            })
        );
        let short = qh(&[dec!(1); 3]);
        assert_eq!(
            allocate(&plant, &[(melo(1), &short)], &AllocationKey::Proportional),
            Err(AllocationError::Misaligned {
                participant: melo(1),
                at: Some(T0 + Duration::minutes(45))
            })
        );
    }

    #[test]
    fn the_key_must_name_exactly_the_participants() {
        let (plant, a, b) = (qh(&[dec!(1)]), qh(&[dec!(1)]), qh(&[dec!(1)]));
        let only_one = AllocationKey::Constant {
            fractions: BTreeMap::from([(melo(1), fraction(dec!(0.5)))]),
        };
        assert_eq!(
            allocate(&plant, &[(melo(1), &a), (melo(2), &b)], &only_one),
            Err(AllocationError::NotInKey {
                participant: melo(2)
            })
        );
        assert_eq!(
            allocate(&plant, &[(melo(2), &b)], &only_one),
            Err(AllocationError::NotAParticipant { melo: melo(1) })
        );
        assert_eq!(
            allocate(
                &plant,
                &[(melo(1), &a), (melo(1), &b)],
                &AllocationKey::Proportional
            ),
            Err(AllocationError::DuplicateParticipant {
                participant: melo(1)
            })
        );
    }

    #[test]
    fn an_over_subscribed_or_negative_key_is_refused() {
        let (plant, a, b) = (qh(&[dec!(1)]), qh(&[dec!(1)]), qh(&[dec!(1)]));
        let over = AllocationKey::Constant {
            fractions: BTreeMap::from([
                (melo(1), fraction(dec!(0.6))),
                (melo(2), fraction(dec!(0.5))),
            ]),
        };
        assert_eq!(
            allocate(&plant, &[(melo(1), &a), (melo(2), &b)], &over),
            Err(AllocationError::FractionsExceedOne { sum: dec!(1.1) })
        );
        let negative = AllocationKey::Cascading {
            weights: BTreeMap::from([(melo(1), dec!(-1))]),
        };
        assert_eq!(
            allocate(&plant, &[(melo(1), &a)], &negative),
            Err(AllocationError::NegativeWeight {
                participant: melo(1),
                weight: dec!(-1)
            })
        );
    }

    #[test]
    fn a_row_carries_the_worst_quality() {
        let plant = qh(&[dec!(1)]);
        let estimated = Series::new(
            Resolution::QUARTER_HOUR,
            DayBoundary::Strom,
            vec![MeterInterval::quarter_hour(T0, dec!(1), QualityFlag::Estimated).unwrap()],
        )
        .unwrap();
        let rows = allocate(
            &plant,
            &[(melo(1), &estimated)],
            &AllocationKey::Proportional,
        )
        .unwrap();
        assert_eq!(rows[0].quality, QualityFlag::Estimated);
    }
}
