//! Virtual meter compute engine.
//!
//! Applies an [`AggregationRule`] to a map of source MaLo / MeLo time series
//! to produce a derived virtual meter time series.
//!
//! ## GGV net grid draw (§42b EnWG, Solarpaket I)
//!
//! Both GGV variants compute the tenant's **net grid draw after PV allocation**:
//!
//! - [`AggregationRule::GgvConstantAllocation`]: static fraction from UTILTS CCI+ZG6
//! - [`AggregationRule::GgvProportionalAllocation`]: dynamic ratio from actual consumption
//!
//! The result is the `Malo_i Verbrauch` quantity in the BDEW Anwendungshilfe —
//! the energy each tenant draws from the public grid after their share of the
//! community PV has been credited.
//!
//! ## Timestamp alignment
//!
//! All source series must be aligned to the same UTC timestamp grid. Only
//! timestamps present in **all** required source series are included in the output.
//! Use [`crate::resample()`] first if source series have different resolutions.
//!
//! ## Legal basis
//!
//! - **§ 42b EnWG — Gemeinschaftliche Gebäudeversorgung** (Solarpaket I). Abs. 5
//!   caps the allocation, verbatim: *"die rechnerisch aufteilbare Strommenge
//!   \[ist\] begrenzt … auf die Strommenge, die innerhalb eines
//!   15-Minuten-Zeitintervalls in der Solaranlage erzeugt oder von allen
//!   teilnehmenden Letztverbrauchern verbraucht wird, je nachdem welche dieser
//!   Strommengen geringer ist."*
//!
//!   Note the cap is on the **pool**: the lesser of generation and *total*
//!   participant consumption. The per-tenant cap this module applies —
//!   `max(0, …)`, so no tenant is credited more PV than they themselves drew —
//!   comes from the BDEW Anwendungshilfe's `Pos()` operator, not from that
//!   sentence.
//! - **BDEW Anwendungshilfe "Beispiele von Berechnungsformeln für das
//!   Solarpaket 1"** v1.0, 25.01.2024 — the worked allocation formulas.
//! - **MaBiS** (BNetzA BK6-07-002, Lesefassung BK6-24-174) — portfolio
//!   aggregation for Bilanzkreis settlement.

use std::collections::{HashMap, HashSet};
use std::hash::BuildHasher;

use rust_decimal::{Decimal, RoundingStrategy};
use time::OffsetDateTime;

use crate::aggregation_rule::{AggregationRule, VirtualMeterKind};
use crate::interval::{MeterInterval, QualityFlag};

/// Source series keyed by MaLo / MeLo ID.
///
/// Generic over the hasher so a caller already holding an `FxHashMap` or an
/// `ahash::HashMap` can pass it directly, instead of rebuilding the map — which
/// for a year of quarter-hours across a GGV community is a large copy to make
/// for a type parameter.
pub type SourceMap<S = std::collections::hash_map::RandomState> =
    HashMap<String, Vec<MeterInterval>, S>;

// ── VirtualMeterError ─────────────────────────────────────────────────────────

/// Error when computing a virtual meter.
///
/// `#[non_exhaustive]`: new rules bring new failure modes, and a caller that
/// wildcards an unrecognised error still behaves correctly — it reports a
/// failure. That is the opposite of the domain enums in this crate, which are
/// exhaustive on purpose; see the crate-level **Enum exhaustiveness** section.
#[derive(Debug, thiserror::Error, PartialEq)]
#[non_exhaustive]
pub enum VirtualMeterError {
    /// A required source MaLo has no entry in the provided data map.
    #[error("missing source MaLo: {0}")]
    MissingSource(String),

    /// An allocation fraction is out of range — each must be positive and
    /// they must sum to at most 1.
    ///
    /// [`compute_ggv_allocation`] knows only the one tenant's fraction, so
    /// `sum` there is that single value; only
    /// [`compute_community_allocation`], which sees the whole participant set,
    /// can report a genuine sum.
    #[error("allocation fractions total {sum} — each must be positive and the total at most 1")]
    InvalidFractions {
        /// The offending fraction, or the sum across the community.
        sum: Decimal,
    },

    /// [`compute_ggv_allocation`] was given a rule that allocates nothing.
    #[error("{kind} is not a GGV allocation rule — it has no allocated amount")]
    NotAGgvRule {
        /// The rule kind that was supplied.
        kind: VirtualMeterKind,
    },
}

// ── compute_virtual_meter ─────────────────────────────────────────────────────

/// Apply an [`AggregationRule`] to produce a virtual meter time series.
///
/// `sources` maps MaLo ID → sorted `Vec<MeterInterval>`.
///
/// Only intervals whose timestamp appears in **all** required source series
/// are included in the output (intersection semantics). This is conservative:
/// gaps in any single source propagate to the output rather than silently
/// producing wrong totals.
///
/// # Errors
///
/// - [`VirtualMeterError::MissingSource`] when a required MaLo is absent.
/// - [`VirtualMeterError::InvalidFractions`] for invalid GGV fractions.
pub fn compute_virtual_meter<S: BuildHasher>(
    rule: &AggregationRule,
    sources: &SourceMap<S>,
) -> Result<Vec<MeterInterval>, VirtualMeterError> {
    match rule {
        AggregationRule::Sum { source_malo_ids } => compute_sum(source_malo_ids, sources),
        AggregationRule::Residual {
            total_malo_id,
            subtract_malo_ids,
        } => compute_residual(total_malo_id, subtract_malo_ids, sources),
        AggregationRule::PvSelfConsumption {
            grid_malo_id,
            generation_malo_id,
        } => compute_net_grid(grid_malo_id, generation_malo_id, sources),
        // Both GGV variants project from the full allocation, so the `Pos()`
        // cap of § 42b Abs. 5 has exactly one implementation and the two entry
        // points cannot drift apart.
        AggregationRule::GgvConstantAllocation { .. }
        | AggregationRule::GgvProportionalAllocation { .. } => {
            Ok(compute_ggv_allocation(rule, sources)?
                .iter()
                .map(GgvInterval::to_net_interval)
                .collect())
        }
    }
}

// ── Sum ───────────────────────────────────────────────────────────────────────

fn compute_sum<S: BuildHasher>(
    malo_ids: &[String],
    sources: &SourceMap<S>,
) -> Result<Vec<MeterInterval>, VirtualMeterError> {
    for id in malo_ids {
        if !sources.contains_key(id.as_str()) {
            return Err(VirtualMeterError::MissingSource(id.clone()));
        }
    }
    let index = SourceIndex::build(sources);
    let aligned = aligned_timestamps(malo_ids.iter().map(String::as_str), sources);
    let mut result = Vec::with_capacity(aligned.len());
    for ts in aligned {
        let Some(ivs) = malo_ids
            .iter()
            .map(|id| index.get(id, ts))
            .collect::<Option<Vec<_>>>()
        else {
            continue;
        };
        let mut sum = Decimal::ZERO;
        let mut quality = QualityFlag::Measured;
        // The furthest end, not the last one listed: sources are named in the
        // rule's order, and a result whose interval length depends on how the
        // caller happened to order two ids is not a result.
        let mut end: Option<OffsetDateTime> = None;
        for iv in ivs {
            sum += iv.value;
            quality = quality.worse_of(iv.quality);
            end = Some(end.map_or(iv.to, |e: OffsetDateTime| e.max(iv.to)));
        }
        if let Some(to) = end {
            result.push(MeterInterval {
                from: ts,
                to,
                value: sum,
                quality,
                obis_code: None,
            });
        }
    }
    Ok(result)
}

// ── Residual ──────────────────────────────────────────────────────────────────

fn compute_residual<S: BuildHasher>(
    total_id: &str,
    subtract_ids: &[String],
    sources: &SourceMap<S>,
) -> Result<Vec<MeterInterval>, VirtualMeterError> {
    if !sources.contains_key(total_id) {
        return Err(VirtualMeterError::MissingSource(total_id.to_owned()));
    }
    for id in subtract_ids {
        if !sources.contains_key(id.as_str()) {
            return Err(VirtualMeterError::MissingSource(id.clone()));
        }
    }
    let index = SourceIndex::build(sources);
    let all_ids = std::iter::once(total_id).chain(subtract_ids.iter().map(String::as_str));
    let aligned = aligned_timestamps(all_ids, sources);
    let mut result = Vec::with_capacity(aligned.len());
    for ts in aligned {
        let Some(total_iv) = index.get(total_id, ts) else {
            continue;
        };
        let Some(subtract_ivs) = subtract_ids
            .iter()
            .map(|id| index.get(id, ts))
            .collect::<Option<Vec<_>>>()
        else {
            continue;
        };
        let mut subtract_sum = Decimal::ZERO;
        let mut quality = total_iv.quality;
        for iv in subtract_ivs {
            subtract_sum += iv.value;
            quality = quality.worse_of(iv.quality);
        }
        result.push(MeterInterval {
            from: ts,
            to: total_iv.to,
            value: total_iv.value - subtract_sum,
            quality,
            obis_code: None,
        });
    }
    Ok(result)
}

// ── PV net grid ───────────────────────────────────────────────────────────────

fn compute_net_grid<S: BuildHasher>(
    grid_id: &str,
    gen_id: &str,
    sources: &SourceMap<S>,
) -> Result<Vec<MeterInterval>, VirtualMeterError> {
    if !sources.contains_key(grid_id) {
        return Err(VirtualMeterError::MissingSource(grid_id.to_owned()));
    }
    if !sources.contains_key(gen_id) {
        return Err(VirtualMeterError::MissingSource(gen_id.to_owned()));
    }
    let index = SourceIndex::build(sources);
    let aligned = aligned_timestamps([grid_id, gen_id].iter().copied(), sources);
    let mut result = Vec::with_capacity(aligned.len());
    for ts in aligned {
        let (Some(grid_iv), Some(gen_iv)) = (index.get(grid_id, ts), index.get(gen_id, ts)) else {
            continue;
        };
        // Net grid draw: positive = consuming from grid, negative = exporting
        result.push(MeterInterval {
            from: ts,
            to: grid_iv.to,
            value: grid_iv.value - gen_iv.value,
            quality: grid_iv.quality.worse_of(gen_iv.quality),
            obis_code: None,
        });
    }
    Ok(result)
}

// ── GGV allocation, in full ──────────────────────────────────────────────────

/// One interval of a § 42b GGV allocation.
///
/// [`compute_virtual_meter`] returns only the tenant's **net grid draw**, which
/// is what the Marktlokation is billed for. A § 42b or § 42c settlement is
/// built on the **allocated** energy — the share of community PV credited to
/// this tenant — so it is reported here rather than left to be recovered by
/// subtracting the net from a re-projected consumption series.
///
/// `consumption == allocated + net_grid_draw` holds in every interval,
/// exactly — because [`share`](Self::share) is cut to [`ALLOCATION_DP`] first.
/// An uncut proportional share is a quotient with more significant digits than
/// the subtraction that follows can hold, and the identity would miss by a few
/// 1e-27 kWh.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct GgvInterval {
    /// Interval start (UTC, inclusive).
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::rfc3339"))]
    pub from: OffsetDateTime,
    /// Interval end (UTC, exclusive).
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::rfc3339"))]
    pub to: OffsetDateTime,

    /// The tenant's metered consumption — `Melo_i Verbrauch`.
    pub consumption: Decimal,
    /// The community plant's whole generation in the interval —
    /// `Melo1 Erzeugung`.
    pub generation: Decimal,

    /// The tenant's share of that generation **before** the `Pos()` cap:
    /// `fraction × generation`, or `ratio × generation`, cut to
    /// [`ALLOCATION_DP`] places toward zero.
    pub share: Decimal,
    /// The share actually credited: `min(consumption, share)`.
    pub allocated: Decimal,
    /// What the tenant still draws from the public grid:
    /// `consumption − allocated`.
    pub net_grid_draw: Decimal,

    /// Worst quality across the plant and every contributing tenant.
    pub quality: QualityFlag,
}

impl GgvInterval {
    /// `true` when this tenant's share was limited by their own consumption,
    /// so the remainder fed the grid — the `Pos()` operator of the BDEW
    /// Anwendungshilfe (UTILTS Z83) made visible.
    #[must_use]
    pub fn capped(&self) -> bool {
        self.share > self.allocated
    }

    /// The part of this tenant's nominal share that they could not use, and
    /// which therefore fed the public grid.
    #[must_use]
    pub fn surplus_to_grid(&self) -> Decimal {
        self.share - self.allocated
    }

    /// The net-grid-draw interval [`compute_virtual_meter`] returns for the
    /// same input.
    #[must_use]
    pub fn to_net_interval(&self) -> MeterInterval {
        MeterInterval {
            from: self.from,
            to: self.to,
            value: self.net_grid_draw,
            quality: self.quality,
            obis_code: None,
        }
    }
}

/// Compute a § 42b GGV allocation in full — consumption, generation, share,
/// allocated energy and net grid draw per interval.
///
/// Both GGV variants produce the same shape; they differ in how the share is
/// derived:
///
/// | Rule | Share |
/// |---|---|
/// | [`GgvConstantAllocation`](AggregationRule::GgvConstantAllocation) | `fraction × generation` (UTILTS `CCI+ZG6`) |
/// | [`GgvProportionalAllocation`](AggregationRule::GgvProportionalAllocation) | `(consumption ÷ Σ consumption) × generation` (UTILTS Z74) |
///
/// In both, `allocated = min(consumption, share)` — the `Pos()` operator — and
/// `net_grid_draw = consumption − allocated`.
///
/// # Errors
///
/// [`NotAGgvRule`](VirtualMeterError::NotAGgvRule) for a rule that allocates
/// nothing, [`MissingSource`](VirtualMeterError::MissingSource) for an absent
/// series, [`InvalidFractions`](VirtualMeterError::InvalidFractions) for a
/// constant fraction outside `(0, 1]`.
///
/// # Example
///
/// ```rust
/// use metering::{AggregationRule, MeterInterval, QualityFlag, compute_ggv_allocation};
/// use rust_decimal::dec;
/// use std::collections::HashMap;
/// use time::macros::datetime;
///
/// let iv = |kwh| vec![MeterInterval {
///     from: datetime!(2026-06-01 12:00 UTC),
///     to:   datetime!(2026-06-01 12:15 UTC),
///     value: kwh,
///     quality: QualityFlag::Measured,
///     obis_code: None,
/// }];
/// let mut sources = HashMap::new();
/// sources.insert("PLANT".to_owned(), iv(dec!(10)));   // 10 kWh generated
/// sources.insert("T1".to_owned(), iv(dec!(1)));       //  1 kWh drawn
///
/// let rule = AggregationRule::GgvConstantAllocation {
///     plant_melo_id: "PLANT".to_owned(),
///     tenant_melo_id: "T1".to_owned(),
///     fraction: dec!(0.5),                             // half the plant
/// };
/// let out = compute_ggv_allocation(&rule, &sources)?;
///
/// // The nominal share is 5 kWh, but the tenant only drew 1.
/// assert_eq!(out[0].share, dec!(5.0));
/// assert_eq!(out[0].allocated, dec!(1));
/// assert_eq!(out[0].net_grid_draw, dec!(0));
/// assert!(out[0].capped(), "limited by their own consumption");
/// assert_eq!(out[0].surplus_to_grid(), dec!(4.0), "the rest fed the grid");
///
/// // ...and the identity holds exactly.
/// assert_eq!(out[0].consumption, out[0].allocated + out[0].net_grid_draw);
/// # Ok::<(), metering::VirtualMeterError>(())
/// ```
pub fn compute_ggv_allocation<S: BuildHasher>(
    rule: &AggregationRule,
    sources: &SourceMap<S>,
) -> Result<Vec<GgvInterval>, VirtualMeterError> {
    match rule {
        AggregationRule::GgvConstantAllocation {
            plant_melo_id,
            tenant_melo_id,
            fraction,
        } => ggv_constant(plant_melo_id, tenant_melo_id, *fraction, sources),
        AggregationRule::GgvProportionalAllocation {
            plant_melo_id,
            tenant_melo_id,
            all_tenant_melo_ids,
        } => ggv_proportional(plant_melo_id, tenant_melo_id, all_tenant_melo_ids, sources),
        other => Err(VirtualMeterError::NotAGgvRule { kind: other.kind() }),
    }
}

/// Decimal places an allocated **share** is cut to before the `Pos()` cap.
///
/// The proportional key divides — `consumption ÷ Σ consumption × generation` —
/// and a `Decimal` quotient carries up to 28 significant digits. Two things go
/// wrong if that reaches the output. It is not a *quantity*: no invoice, no
/// MSCONS field and no settlement system has a place for 0.333…3 kWh to
/// twenty-seven places. And it breaks the identity this module documents,
/// because `consumption − allocated` then needs more significant digits than a
/// `Decimal` has and rounds.
///
/// Six places is a millionth of a kWh: four orders of magnitude finer than
/// anything the market settles, and coarse enough that every derived value is
/// a number someone can write down.
///
/// The cut is **toward zero**, which is not a stylistic choice. Truncating can
/// only lower a share, so `Σ allocated ≤ Σ share ≤ generation` survives it and
/// the § 42b Abs. 5 ceiling stays a theorem rather than becoming a clamp.
pub const ALLOCATION_DP: u32 = 6;

/// A raw share, cut to [`ALLOCATION_DP`] places toward zero.
///
/// Applied where the share is *formed*, not where it is capped, so the value
/// reported as [`GgvInterval::share`] is the one the cap was taken against —
/// which is what `capped()`, comparing the two, depends on.
fn allocation_share(raw: Decimal) -> Decimal {
    raw.round_dp_with_strategy(ALLOCATION_DP, RoundingStrategy::ToZero)
}

/// `allocated = min(consumption, share)`, and the net that follows.
///
/// One place, so the `Pos()` cap of § 42b Abs. 5 / UTILTS Z83 is applied once
/// and both entry points read the same arithmetic. `share` is expected to have
/// been through [`allocation_share`] already.
///
/// The consumption is the caller's measurement and passes through untouched,
/// so a participant whose whole draw is covered is credited exactly what they
/// drew and nets exactly zero, however many decimal places their meter
/// delivered.
fn ggv_split(consumption: Decimal, share: Decimal) -> (Decimal, Decimal) {
    let allocated = consumption.min(share).max(Decimal::ZERO);
    (allocated, consumption - allocated)
}

// ── GGV constant allocation (§42b EnWG Beispiel 1, CCI+ZG6) ──────────────────

/// Constant-fraction GGV allocation — BDEW Anwendungshilfe Beispiel 1,
/// UTILTS `CCI+ZG6` with `CAV+Z28`.
///
/// ```text
/// share     = fraction × generation
/// allocated = min(consumption, share)          // Pos(), UTILTS Z83
/// net       = consumption − allocated
/// ```
fn ggv_constant<S: BuildHasher>(
    plant_id: &str,
    tenant_id: &str,
    fraction: Decimal,
    sources: &SourceMap<S>,
) -> Result<Vec<GgvInterval>, VirtualMeterError> {
    require(sources, [plant_id, tenant_id])?;
    if fraction <= Decimal::ZERO || fraction > Decimal::ONE {
        return Err(VirtualMeterError::InvalidFractions { sum: fraction });
    }

    let index = SourceIndex::build(sources);
    let aligned = aligned_timestamps([plant_id, tenant_id].iter().copied(), sources);
    let mut result = Vec::with_capacity(aligned.len());
    for ts in aligned {
        let (Some(plant_iv), Some(tenant_iv)) = (index.get(plant_id, ts), index.get(tenant_id, ts))
        else {
            continue;
        };
        let share = allocation_share(fraction * plant_iv.value);
        let (allocated, net_grid_draw) = ggv_split(tenant_iv.value, share);
        result.push(GgvInterval {
            from: ts,
            to: tenant_iv.to,
            consumption: tenant_iv.value,
            generation: plant_iv.value,
            share,
            allocated,
            net_grid_draw,
            quality: plant_iv.quality.worse_of(tenant_iv.quality),
        });
    }
    Ok(result)
}

/// Consumption-proportional GGV allocation — BDEW Anwendungshilfe Beispiel 3,
/// UTILTS Z74 Divisionsquotient.
///
/// ```text
/// total     = Σ consumption_j
/// share     = (consumption ÷ total) × generation     (0 when total = 0)
/// allocated = min(consumption, share)
/// net       = consumption − allocated
/// ```
///
/// The zero-denominator branch follows the Anwendungshilfe: *"Ist die
/// Energiemenge einer Marktlokation zugeordneten Messlokation = 0, so ist auch
/// der Verbrauch der Marktlokation auf 0 zu setzen. Dies verhindert auch eine
/// Division durch 0."* With quantities positive-or-zero (Codeliste v2.5c
/// §2.1), `total = 0` implies this tenant's own consumption is 0, so the net
/// is 0 by the identity rather than by a special case — which is why the net
/// is computed the same way in both branches instead of being forced to zero.
fn ggv_proportional<S: BuildHasher>(
    plant_id: &str,
    tenant_id: &str,
    all_tenant_ids: &[String],
    sources: &SourceMap<S>,
) -> Result<Vec<GgvInterval>, VirtualMeterError> {
    require(sources, [plant_id, tenant_id])?;
    require(sources, all_tenant_ids.iter().map(String::as_str))?;

    let all_ids: Vec<&str> = std::iter::once(plant_id)
        .chain(all_tenant_ids.iter().map(String::as_str))
        .collect();
    let index = SourceIndex::build(sources);
    let aligned = aligned_timestamps(all_ids.iter().copied(), sources);
    let mut result = Vec::with_capacity(aligned.len());

    for ts in aligned {
        let (Some(plant_iv), Some(tenant_iv)) = (index.get(plant_id, ts), index.get(tenant_id, ts))
        else {
            continue;
        };
        let Some(total) = all_tenant_ids
            .iter()
            .map(|id| index.get(id, ts).map(|iv| iv.value))
            .sum::<Option<Decimal>>()
        else {
            continue;
        };

        let share = allocation_share(if total > Decimal::ZERO {
            tenant_iv.value / total * plant_iv.value
        } else {
            Decimal::ZERO
        });
        let (allocated, net_grid_draw) = ggv_split(tenant_iv.value, share);

        // Any estimated or substituted contributor moves the whole result.
        let quality = all_tenant_ids
            .iter()
            .filter_map(|id| index.get(id, ts))
            .fold(plant_iv.quality, |q, iv| q.worse_of(iv.quality));

        result.push(GgvInterval {
            from: ts,
            to: tenant_iv.to,
            consumption: tenant_iv.value,
            generation: plant_iv.value,
            share,
            allocated,
            net_grid_draw,
            quality,
        });
    }
    Ok(result)
}

/// Every named series must be present before any arithmetic starts.
fn require<'a, S: BuildHasher>(
    sources: &SourceMap<S>,
    ids: impl IntoIterator<Item = &'a str>,
) -> Result<(), VirtualMeterError> {
    for id in ids {
        if !sources.contains_key(id) {
            return Err(VirtualMeterError::MissingSource(id.to_owned()));
        }
    }
    Ok(())
}

// ── the whole community at once ──────────────────────────────────────────────

/// How a community's generation is divided among its participants.
///
/// The two keys the BDEW Anwendungshilfe publishes for § 42b, and the shape a
/// § 42c *Aufteilungsschlüssel* takes as well — see
/// [`compute_community_allocation`].
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(
    feature = "serde",
    serde(tag = "kind", rename_all = "SCREAMING_SNAKE_CASE")
)]
pub enum AllocationKey {
    /// A fixed fraction per participant — UTILTS `CCI+ZG6` with `CAV+Z28`.
    ///
    /// The fractions must be positive and sum to at most 1; the remainder is
    /// generation the community does not claim and which feeds the grid.
    Constant {
        /// Participant series id → fraction of the plant's generation.
        fractions: std::collections::BTreeMap<String, Decimal>,
    },
    /// Proportional to each participant's own consumption in the interval —
    /// UTILTS `Z74` Divisionsquotient.
    Proportional {
        /// Participant series ids. Order does not affect the result.
        participants: Vec<String>,
    },
}

impl AllocationKey {
    /// The participant series ids this key names, in a stable order.
    #[must_use]
    pub fn participant_ids(&self) -> Vec<&str> {
        match self {
            Self::Constant { fractions } => fractions.keys().map(String::as_str).collect(),
            Self::Proportional { participants } => {
                let mut ids: Vec<&str> = participants.iter().map(String::as_str).collect();
                ids.sort_unstable();
                ids.dedup();
                ids
            }
        }
    }
}

/// What one participant received in one interval.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ParticipantAllocation {
    /// The participant's series id.
    pub id: String,
    /// What they consumed.
    pub consumption: Decimal,
    /// Their nominal share of the generation, before the `Pos()` cap, cut to
    /// [`ALLOCATION_DP`] places toward zero.
    pub share: Decimal,
    /// What was actually credited: `min(consumption, share)`.
    pub allocated: Decimal,
    /// What they still drew from the public grid.
    pub net_grid_draw: Decimal,
}

impl ParticipantAllocation {
    /// `true` when the share was limited by this participant's own
    /// consumption, so the remainder fed the grid.
    #[must_use]
    pub fn capped(&self) -> bool {
        self.share > self.allocated
    }
}

/// One interval of a community allocation, from the community's side.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct CommunityInterval {
    /// Interval start (UTC, inclusive).
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::rfc3339"))]
    pub from: OffsetDateTime,
    /// Interval end (UTC, exclusive).
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::rfc3339"))]
    pub to: OffsetDateTime,
    /// The plant's whole generation in the interval.
    pub generation: Decimal,
    /// The sum of every participant's consumption.
    pub total_consumption: Decimal,
    /// The § 42b Abs. 5 ceiling: `min(generation, total_consumption)`.
    ///
    /// **A ceiling, not a clamp.** See
    /// [`compute_community_allocation`](compute_community_allocation#the-pool-cap-is-a-theorem-here-not-a-step).
    pub pool_cap: Decimal,
    /// Per participant, in the key's own order.
    pub participants: Vec<ParticipantAllocation>,
    /// Generation the community did not use, which fed the public grid.
    pub surplus_to_grid: Decimal,
    /// Worst quality across the plant and every participant.
    pub quality: QualityFlag,
}

impl CommunityInterval {
    /// The energy credited across every participant.
    #[must_use]
    pub fn total_allocated(&self) -> Decimal {
        self.participants.iter().map(|p| p.allocated).sum()
    }

    /// The energy the community still drew from the public grid.
    #[must_use]
    pub fn total_net_grid_draw(&self) -> Decimal {
        self.participants.iter().map(|p| p.net_grid_draw).sum()
    }

    /// One participant's allocation, by series id.
    #[must_use]
    pub fn participant(&self, id: &str) -> Option<&ParticipantAllocation> {
        self.participants.iter().find(|p| p.id == id)
    }
}

/// Allocate a community's generation across **every** participant at once.
///
/// [`compute_ggv_allocation`] answers for one tenant, which is the shape a
/// persisted rule has. This answers for the community, and is not the same
/// computation run N times: the proportional denominator and the source index
/// are formed once rather than once per tenant, the surplus that fed the grid
/// is a number rather than something to reconstruct, and the § 42b Abs. 5 pool
/// ceiling becomes computable at all.
///
/// ## The pool cap is a theorem, not a step
///
/// § 42b Abs. 5 caps the allocatable energy at *"die Strommenge, die innerhalb
/// eines 15-Minuten-Zeitintervalls in der Solaranlage erzeugt oder von allen
/// teilnehmenden Letztverbrauchern verbraucht wird, je nachdem welche dieser
/// Strommengen geringer ist."*
///
/// This does **not** clamp to that figure. With fractions summing to at most 1
/// the per-participant `Pos()` cap already implies it — `Σ min(cᵢ, shareᵢ) ≤ Σ cᵢ`
/// and `Σ shareᵢ ≤ generation` — so clamping again would be a rule the statute
/// does not contain. [`CommunityInterval::pool_cap`] reports the ceiling, and
/// `tests/allocation_invariants.rs` holds the inequality under proptest.
///
/// ## § 42c uses the same arithmetic
///
/// Energy Sharing has no published allocation formula: § 42c Abs. 3 Nr. 2
/// requires the *contract* to state *"einen Aufteilungsschlüssel, aus dem sich
/// der Umfang des Rechts zur Nutzung der Elektrizität ergibt"*, so the key is
/// an input. Both [`AllocationKey`] variants express one, and
/// `min(consumption, share)` carries over as physics. [`pool_cap`] does not:
/// § 42c has no counterpart to Abs. 5, so there it is an observation.
///
/// [`pool_cap`]: CommunityInterval::pool_cap
///
/// # Errors
///
/// - [`MissingSource`](VirtualMeterError::MissingSource) for an absent series.
/// - [`InvalidFractions`](VirtualMeterError::InvalidFractions) when the
///   constant fractions are not all positive, or sum above 1.
///
/// ```rust
/// use metering::{AllocationKey, MeterInterval, QualityFlag, compute_community_allocation};
/// use rust_decimal::dec;
/// use std::collections::HashMap;
/// use time::macros::datetime;
///
/// let iv = |kwh| vec![MeterInterval {
///     from: datetime!(2026-06-01 12:00 UTC),
///     to:   datetime!(2026-06-01 12:15 UTC),
///     value: kwh,
///     quality: QualityFlag::Measured,
///     obis_code: None,
/// }];
/// let mut sources = HashMap::new();
/// sources.insert("PLANT".to_owned(), iv(dec!(10)));
/// sources.insert("T1".to_owned(), iv(dec!(1)));
/// sources.insert("T2".to_owned(), iv(dec!(3)));
///
/// let key = AllocationKey::Proportional {
///     participants: vec!["T1".to_owned(), "T2".to_owned()],
/// };
/// let out = compute_community_allocation("PLANT", &key, &sources)?;
/// let interval = &out[0];
///
/// // Four kWh drawn against ten generated, so the pool cap is the consumption.
/// assert_eq!(interval.total_consumption, dec!(4));
/// assert_eq!(interval.pool_cap, dec!(4));
///
/// // Both tenants are capped by their own draw; six kWh fed the grid.
/// assert_eq!(interval.total_allocated(), dec!(4));
/// assert_eq!(interval.surplus_to_grid, dec!(6));
/// assert_eq!(interval.total_net_grid_draw(), dec!(0));
/// assert!(interval.participant("T1").unwrap().capped());
/// # Ok::<(), metering::VirtualMeterError>(())
/// ```
pub fn compute_community_allocation<S: BuildHasher>(
    plant_id: &str,
    key: &AllocationKey,
    sources: &SourceMap<S>,
) -> Result<Vec<CommunityInterval>, VirtualMeterError> {
    let ids = key.participant_ids();
    require(sources, std::iter::once(plant_id))?;
    require(sources, ids.iter().copied())?;

    if let AllocationKey::Constant { fractions } = key {
        let sum: Decimal = fractions.values().copied().sum();
        if fractions.values().any(|f| *f <= Decimal::ZERO) || sum > Decimal::ONE {
            return Err(VirtualMeterError::InvalidFractions { sum });
        }
    }

    let index = SourceIndex::build(sources);
    let aligned = aligned_timestamps(
        std::iter::once(plant_id).chain(ids.iter().copied()),
        sources,
    );
    let mut out = Vec::with_capacity(aligned.len());

    for ts in aligned {
        let Some(plant_iv) = index.get(plant_id, ts) else {
            continue;
        };
        // Every participant must be present, or the denominator — and the
        // surplus — would be formed over a different set than the numerator.
        let Some(consumption): Option<Vec<(&str, &MeterInterval)>> = ids
            .iter()
            .map(|id| index.get(id, ts).map(|iv| (*id, iv)))
            .collect()
        else {
            continue;
        };

        let total_consumption: Decimal = consumption.iter().map(|(_, iv)| iv.value).sum();
        let generation = plant_iv.value;

        let participants: Vec<ParticipantAllocation> = consumption
            .iter()
            .map(|(id, iv)| {
                let share = allocation_share(match key {
                    AllocationKey::Constant { fractions } => {
                        fractions.get(*id).copied().unwrap_or(Decimal::ZERO) * generation
                    }
                    AllocationKey::Proportional { .. } => {
                        if total_consumption > Decimal::ZERO {
                            iv.value / total_consumption * generation
                        } else {
                            Decimal::ZERO
                        }
                    }
                });
                let (allocated, net_grid_draw) = ggv_split(iv.value, share);
                ParticipantAllocation {
                    id: (*id).to_owned(),
                    consumption: iv.value,
                    share,
                    allocated,
                    net_grid_draw,
                }
            })
            .collect();

        let allocated: Decimal = participants.iter().map(|p| p.allocated).sum();
        let quality = consumption
            .iter()
            .fold(plant_iv.quality, |q, (_, iv)| q.worse_of(iv.quality));

        out.push(CommunityInterval {
            from: ts,
            to: plant_iv.to,
            generation,
            total_consumption,
            pool_cap: generation.min(total_consumption),
            participants,
            surplus_to_grid: (generation - allocated).max(Decimal::ZERO),
            quality,
        });
    }
    Ok(out)
}

// ── helpers ───────────────────────────────────────────────────────────────────

/// Compute the intersection of timestamps across all named source series.
fn aligned_timestamps<'a, S: BuildHasher>(
    malo_ids: impl Iterator<Item = &'a str>,
    sources: &SourceMap<S>,
) -> Vec<OffsetDateTime> {
    let ids: Vec<&str> = malo_ids.collect();
    let Some((first, rest)) = ids.split_first() else {
        return Vec::new();
    };
    let series = |id: &str| -> HashSet<i64> {
        sources
            .get(id)
            .map(|ivs| ivs.iter().map(|iv| iv.from.unix_timestamp()).collect())
            .unwrap_or_default()
    };

    let intersection = rest.iter().fold(series(first), |acc, id| {
        let other = series(id);
        acc.intersection(&other).copied().collect()
    });

    let mut sorted: Vec<i64> = intersection.into_iter().collect();
    sorted.sort_unstable();
    sorted
        .into_iter()
        .filter_map(|t| OffsetDateTime::from_unix_timestamp(t).ok())
        .collect()
}

/// Every source series indexed by interval start, so a per-timestamp lookup is
/// a hash probe rather than a scan.
///
/// The scan it replaces made every rule quadratic in the series length: one
/// linear `find` per source per aligned timestamp. On a year of quarter-hours
/// that is 35 040² probes per source — the difference between a virtual meter
/// that computes in milliseconds and one that appears to hang.
struct SourceIndex<'a> {
    by_id: HashMap<&'a str, HashMap<i64, &'a MeterInterval>>,
}

impl<'a> SourceIndex<'a> {
    fn build<S: BuildHasher>(sources: &'a SourceMap<S>) -> Self {
        let by_id = sources
            .iter()
            .map(|(id, ivs)| {
                let index = ivs
                    .iter()
                    .map(|iv| (iv.from.unix_timestamp(), iv))
                    .collect();
                (id.as_str(), index)
            })
            .collect();
        Self { by_id }
    }

    /// The interval starting at `ts` in the named source series.
    ///
    /// `aligned_timestamps` guarantees the timestamp exists in every required
    /// series, so `None` means the series carries two intervals with the same
    /// start and the later one displaced the earlier, or the id is absent. The
    /// compute functions skip such a timestamp rather than panic: a pure
    /// library must not abort the process on inconsistent input.
    fn get(&self, malo_id: &str, ts: OffsetDateTime) -> Option<&'a MeterInterval> {
        self.by_id
            .get(malo_id)
            .and_then(|index| index.get(&ts.unix_timestamp()))
            .copied()
    }
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal::dec;
    use time::{Duration, macros::datetime};

    fn make_iv(from: OffsetDateTime, kwh: Decimal, quality: QualityFlag) -> MeterInterval {
        MeterInterval {
            from,
            to: from + Duration::minutes(15),
            value: kwh,
            quality,
            obis_code: None,
        }
    }

    fn ts(offset_min: i64) -> OffsetDateTime {
        datetime!(2026-01-01 00:00 UTC) + Duration::minutes(offset_min)
    }

    fn source(id: &str, values: Vec<(i64, Decimal)>) -> (String, Vec<MeterInterval>) {
        let ivs = values
            .into_iter()
            .map(|(min, kwh)| make_iv(ts(min), kwh, QualityFlag::Measured))
            .collect();
        (id.to_owned(), ivs)
    }

    // ── Sum ───────────────────────────────────────────────────────────────────

    #[test]
    fn sum_rule_adds_two_series() {
        let mut map: HashMap<String, Vec<MeterInterval>> = HashMap::new();
        let (ka, va) = source("A", vec![(0, dec!(3.0)), (15, dec!(3.0))]);
        let (kb, vb) = source("B", vec![(0, dec!(2.0)), (15, dec!(2.0))]);
        map.insert(ka, va);
        map.insert(kb, vb);

        let rule = AggregationRule::Sum {
            source_malo_ids: vec!["A".to_owned(), "B".to_owned()],
        };
        let result = compute_virtual_meter(&rule, &map).unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].value, dec!(5.0));
    }

    #[test]
    fn sum_missing_source_returns_error() {
        let map: HashMap<String, Vec<MeterInterval>> = HashMap::new();
        let rule = AggregationRule::Sum {
            source_malo_ids: vec!["MISSING".to_owned()],
        };
        assert!(matches!(
            compute_virtual_meter(&rule, &map),
            Err(VirtualMeterError::MissingSource(_))
        ));
    }

    // ── Residual ──────────────────────────────────────────────────────────────

    #[test]
    fn residual_rule_subtracts_generation() {
        let mut map: HashMap<String, Vec<MeterInterval>> = HashMap::new();
        let (kt, vt) = source("TOTAL", vec![(0, dec!(10.0)), (15, dec!(8.0))]);
        let (kp, vp) = source("PV", vec![(0, dec!(3.0)), (15, dec!(2.0))]);
        map.insert(kt, vt);
        map.insert(kp, vp);

        let rule = AggregationRule::Residual {
            total_malo_id: "TOTAL".to_owned(),
            subtract_malo_ids: vec!["PV".to_owned()],
        };
        let result = compute_virtual_meter(&rule, &map).unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].value, dec!(7.0));
        assert_eq!(result[1].value, dec!(6.0));
    }

    #[test]
    fn residual_can_produce_negative_for_net_exporter() {
        let mut map: HashMap<String, Vec<MeterInterval>> = HashMap::new();
        let (kt, vt) = source("GRID", vec![(0, dec!(1.0))]);
        let (kp, vp) = source("PV", vec![(0, dec!(5.0))]);
        map.insert(kt, vt);
        map.insert(kp, vp);

        let rule = AggregationRule::Residual {
            total_malo_id: "GRID".to_owned(),
            subtract_malo_ids: vec!["PV".to_owned()],
        };
        let result = compute_virtual_meter(&rule, &map).unwrap();
        assert_eq!(result[0].value, dec!(-4.0));
    }

    // ── PV net grid ───────────────────────────────────────────────────────────

    #[test]
    fn pv_self_consumption_net_grid() {
        let mut map: HashMap<String, Vec<MeterInterval>> = HashMap::new();
        let (kg, vg) = source("GRID", vec![(0, dec!(4.0))]);
        let (kp, vp) = source("GEN", vec![(0, dec!(2.0))]);
        map.insert(kg, vg);
        map.insert(kp, vp);

        let rule = AggregationRule::PvSelfConsumption {
            grid_malo_id: "GRID".to_owned(),
            generation_malo_id: "GEN".to_owned(),
        };
        let result = compute_virtual_meter(&rule, &map).unwrap();
        assert_eq!(result[0].value, dec!(2.0));
    }

    // ── GGV constant allocation (§42b Beispiel 1) ────────────────────────────

    #[test]
    fn ggv_constant_tenant_draws_residual_after_pv() {
        // Beispiel 1: Melo2=10%, Melo3=90%
        // Interval: plant generates 10 kWh, tenant consumes 5 kWh
        // allocated = 10% × 10 = 1 kWh → net_grid_draw = max(0, 5 - 1) = 4 kWh
        let mut map: HashMap<String, Vec<MeterInterval>> = HashMap::new();
        map.insert(
            "PLANT".to_owned(),
            vec![make_iv(ts(0), dec!(10.0), QualityFlag::Measured)],
        );
        map.insert(
            "T2".to_owned(),
            vec![make_iv(ts(0), dec!(5.0), QualityFlag::Measured)],
        );

        let rule = AggregationRule::GgvConstantAllocation {
            plant_melo_id: "PLANT".to_owned(),
            tenant_melo_id: "T2".to_owned(),
            fraction: dec!(0.10),
        };
        let result = compute_virtual_meter(&rule, &map).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].value, dec!(4.0), "net grid draw = 5 - 1 = 4");
    }

    #[test]
    fn ggv_constant_allocation_capped_by_tenant_consumption() {
        // §42b Abs. 5: allocated PV ≤ tenant consumption
        // plant = 10 kWh, fraction = 90%, allocation attempt = 9 kWh
        // but tenant only consumes 2 kWh → net_grid_draw = max(0, 2 - 9) = 0
        // (tenant gets 2 kWh of PV, excess 7 kWh feeds back to grid)
        let mut map: HashMap<String, Vec<MeterInterval>> = HashMap::new();
        map.insert(
            "PLANT".to_owned(),
            vec![make_iv(ts(0), dec!(10.0), QualityFlag::Measured)],
        );
        map.insert(
            "T3".to_owned(),
            vec![make_iv(ts(0), dec!(2.0), QualityFlag::Measured)],
        );

        let rule = AggregationRule::GgvConstantAllocation {
            plant_melo_id: "PLANT".to_owned(),
            tenant_melo_id: "T3".to_owned(),
            fraction: dec!(0.90),
        };
        let result = compute_virtual_meter(&rule, &map).unwrap();
        assert_eq!(result[0].value, dec!(0.0), "pos(2 - 9) = 0 — cap enforced");
    }

    #[test]
    fn ggv_constant_feedin_balance_check() {
        // BDEW Beispiel 1: plant=10 kWh, T2=10%, T3=90%
        // T2 consumes 5, T3 consumes 20
        // T2 net = max(0, 5 - 1) = 4  → PV to T2 = 1
        // T3 net = max(0, 20 - 9) = 11 → PV to T3 = 9
        // Total PV delivered = 1 + 9 = 10 = full plant generation (no grid feed-in)
        let plant_gen = dec!(10.0);
        let t2_consumption = dec!(5.0);
        let t3_consumption = dec!(20.0);

        let t2_net = (t2_consumption - dec!(0.10) * plant_gen).max(Decimal::ZERO);
        let t3_net = (t3_consumption - dec!(0.90) * plant_gen).max(Decimal::ZERO);

        let pv_to_t2 = t2_consumption - t2_net;
        let pv_to_t3 = t3_consumption - t3_net;
        let grid_feedin = plant_gen - pv_to_t2 - pv_to_t3;

        assert_eq!(t2_net, dec!(4.0));
        assert_eq!(t3_net, dec!(11.0));
        assert_eq!(pv_to_t2 + pv_to_t3, dec!(10.0));
        assert_eq!(grid_feedin, dec!(0.0), "all PV consumed locally");
    }

    #[test]
    fn ggv_constant_multiple_intervals() {
        let mut map: HashMap<String, Vec<MeterInterval>> = HashMap::new();
        map.insert(
            "PLANT".to_owned(),
            vec![
                make_iv(ts(0), dec!(10.0), QualityFlag::Measured),
                make_iv(ts(15), dec!(0.0), QualityFlag::Measured),
            ],
        );
        map.insert(
            "T".to_owned(),
            vec![
                make_iv(ts(0), dec!(3.0), QualityFlag::Measured),
                make_iv(ts(15), dec!(3.0), QualityFlag::Measured),
            ],
        );

        let rule = AggregationRule::GgvConstantAllocation {
            plant_melo_id: "PLANT".to_owned(),
            tenant_melo_id: "T".to_owned(),
            fraction: dec!(0.5),
        };
        let result = compute_virtual_meter(&rule, &map).unwrap();
        assert_eq!(result[0].value, dec!(0.0), "3 - 5 < 0 → max(0)");
        assert_eq!(result[1].value, dec!(3.0), "no PV → full load from grid");
    }

    #[test]
    fn ggv_constant_invalid_fraction_rejected() {
        let mut map: HashMap<String, Vec<MeterInterval>> = HashMap::new();
        map.insert(
            "PLANT".to_owned(),
            vec![make_iv(ts(0), dec!(1.0), QualityFlag::Measured)],
        );
        map.insert(
            "T".to_owned(),
            vec![make_iv(ts(0), dec!(1.0), QualityFlag::Measured)],
        );

        let rule = AggregationRule::GgvConstantAllocation {
            plant_melo_id: "PLANT".to_owned(),
            tenant_melo_id: "T".to_owned(),
            fraction: dec!(1.5), // > 1 — invalid
        };
        assert!(matches!(
            compute_virtual_meter(&rule, &map),
            Err(VirtualMeterError::InvalidFractions { .. })
        ));
    }

    // ── GGV proportional allocation (§42b Beispiel 3) ────────────────────────

    #[test]
    fn ggv_proportional_tenant_gets_consumption_weighted_pv() {
        // Beispiel 3: plant=10 kWh, T2 consumes 2, T3 consumes 8
        // T2 ratio = 2/10 = 0.2 → allocation = 0.2 × 10 = 2 → net = max(0, 2-2) = 0
        // T3 ratio = 8/10 = 0.8 → allocation = 0.8 × 10 = 8 → net = max(0, 8-8) = 0
        // both tenants fully covered by PV
        let mut map: HashMap<String, Vec<MeterInterval>> = HashMap::new();
        map.insert(
            "PLANT".to_owned(),
            vec![make_iv(ts(0), dec!(10.0), QualityFlag::Measured)],
        );
        map.insert(
            "T2".to_owned(),
            vec![make_iv(ts(0), dec!(2.0), QualityFlag::Measured)],
        );
        map.insert(
            "T3".to_owned(),
            vec![make_iv(ts(0), dec!(8.0), QualityFlag::Measured)],
        );

        let rule_t2 = AggregationRule::GgvProportionalAllocation {
            plant_melo_id: "PLANT".to_owned(),
            tenant_melo_id: "T2".to_owned(),
            all_tenant_melo_ids: vec!["T2".to_owned(), "T3".to_owned()],
        };
        let rule_t3 = AggregationRule::GgvProportionalAllocation {
            plant_melo_id: "PLANT".to_owned(),
            tenant_melo_id: "T3".to_owned(),
            all_tenant_melo_ids: vec!["T2".to_owned(), "T3".to_owned()],
        };

        let r2 = compute_virtual_meter(&rule_t2, &map).unwrap();
        let r3 = compute_virtual_meter(&rule_t3, &map).unwrap();
        assert_eq!(r2[0].value, dec!(0.0), "T2 fully covered by PV");
        assert_eq!(r3[0].value, dec!(0.0), "T3 fully covered by PV");
    }

    #[test]
    fn ggv_proportional_partial_coverage() {
        // plant=6 kWh, T2 consumes 2, T3 consumes 8 → total=10
        // T2 ratio = 0.2 → allocated = 0.2 × 6 = 1.2 → net = 2 - 1.2 = 0.8
        // T3 ratio = 0.8 → allocated = 0.8 × 6 = 4.8 → net = 8 - 4.8 = 3.2
        let mut map: HashMap<String, Vec<MeterInterval>> = HashMap::new();
        map.insert(
            "PLANT".to_owned(),
            vec![make_iv(ts(0), dec!(6.0), QualityFlag::Measured)],
        );
        map.insert(
            "T2".to_owned(),
            vec![make_iv(ts(0), dec!(2.0), QualityFlag::Measured)],
        );
        map.insert(
            "T3".to_owned(),
            vec![make_iv(ts(0), dec!(8.0), QualityFlag::Measured)],
        );

        let rule_t2 = AggregationRule::GgvProportionalAllocation {
            plant_melo_id: "PLANT".to_owned(),
            tenant_melo_id: "T2".to_owned(),
            all_tenant_melo_ids: vec!["T2".to_owned(), "T3".to_owned()],
        };
        let rule_t3 = AggregationRule::GgvProportionalAllocation {
            plant_melo_id: "PLANT".to_owned(),
            tenant_melo_id: "T3".to_owned(),
            all_tenant_melo_ids: vec!["T2".to_owned(), "T3".to_owned()],
        };

        let r2 = compute_virtual_meter(&rule_t2, &map).unwrap();
        let r3 = compute_virtual_meter(&rule_t3, &map).unwrap();
        assert_eq!(r2[0].value, dec!(0.8));
        assert_eq!(r3[0].value, dec!(3.2));
        // total PV delivered = (2-0.8) + (8-3.2) = 1.2 + 4.8 = 6 = plant generation
        assert_eq!(
            (dec!(2.0) - r2[0].value) + (dec!(8.0) - r3[0].value),
            dec!(6.0)
        );
    }

    #[test]
    fn ggv_proportional_zero_division_guard() {
        // All tenants consume 0 → denominator = 0 → no PV allocated, no grid draw
        let mut map: HashMap<String, Vec<MeterInterval>> = HashMap::new();
        map.insert(
            "PLANT".to_owned(),
            vec![make_iv(ts(0), dec!(5.0), QualityFlag::Measured)],
        );
        map.insert(
            "T2".to_owned(),
            vec![make_iv(ts(0), dec!(0.0), QualityFlag::Measured)],
        );
        map.insert(
            "T3".to_owned(),
            vec![make_iv(ts(0), dec!(0.0), QualityFlag::Measured)],
        );

        let rule = AggregationRule::GgvProportionalAllocation {
            plant_melo_id: "PLANT".to_owned(),
            tenant_melo_id: "T2".to_owned(),
            all_tenant_melo_ids: vec!["T2".to_owned(), "T3".to_owned()],
        };
        let result = compute_virtual_meter(&rule, &map).unwrap();
        assert_eq!(
            result[0].value,
            dec!(0.0),
            "zero total → zero draw (no division by zero)"
        );
    }

    #[test]
    fn ggv_proportional_cap_when_allocation_exceeds_consumption() {
        // plant=100 kWh, T2 consumes 1, T3 consumes 1 → total=2
        // T2 ratio = 0.5, allocated = 50 kWh but T2 only consumed 1 → net = max(0, 1-50) = 0
        let mut map: HashMap<String, Vec<MeterInterval>> = HashMap::new();
        map.insert(
            "PLANT".to_owned(),
            vec![make_iv(ts(0), dec!(100.0), QualityFlag::Measured)],
        );
        map.insert(
            "T2".to_owned(),
            vec![make_iv(ts(0), dec!(1.0), QualityFlag::Measured)],
        );
        map.insert(
            "T3".to_owned(),
            vec![make_iv(ts(0), dec!(1.0), QualityFlag::Measured)],
        );

        let rule = AggregationRule::GgvProportionalAllocation {
            plant_melo_id: "PLANT".to_owned(),
            tenant_melo_id: "T2".to_owned(),
            all_tenant_melo_ids: vec!["T2".to_owned(), "T3".to_owned()],
        };
        let result = compute_virtual_meter(&rule, &map).unwrap();
        assert_eq!(result[0].value, dec!(0.0), "§42b cap: no negative draw");
    }

    #[test]
    fn ggv_proportional_missing_source_returns_error() {
        let map: HashMap<String, Vec<MeterInterval>> = HashMap::new();
        let rule = AggregationRule::GgvProportionalAllocation {
            plant_melo_id: "PLANT".to_owned(),
            tenant_melo_id: "T".to_owned(),
            all_tenant_melo_ids: vec!["T".to_owned()],
        };
        assert!(matches!(
            compute_virtual_meter(&rule, &map),
            Err(VirtualMeterError::MissingSource(_))
        ));
    }

    // ── Alignment ─────────────────────────────────────────────────────────────

    #[test]
    fn misaligned_timestamps_produce_intersection() {
        let mut map: HashMap<String, Vec<MeterInterval>> = HashMap::new();
        // A has ts 0 and 15; B has only ts 15
        let (ka, va) = source("A", vec![(0, dec!(1.0)), (15, dec!(1.0))]);
        let (kb, vb) = source("B", vec![(15, dec!(2.0))]);
        map.insert(ka, va);
        map.insert(kb, vb);

        let rule = AggregationRule::Sum {
            source_malo_ids: vec!["A".to_owned(), "B".to_owned()],
        };
        let result = compute_virtual_meter(&rule, &map).unwrap();
        assert_eq!(result.len(), 1, "only ts=15 is in both series");
        assert_eq!(result[0].value, dec!(3.0));
    }

    // ── Quality propagation ────────────────────────────────────────────────────

    #[test]
    fn worst_quality_propagates_in_sum() {
        let base = ts(0);
        let mut map: HashMap<String, Vec<MeterInterval>> = HashMap::new();
        map.insert(
            "A".to_owned(),
            vec![make_iv(base, dec!(1.0), QualityFlag::Measured)],
        );
        map.insert(
            "B".to_owned(),
            vec![make_iv(base, dec!(1.0), QualityFlag::Estimated)],
        );

        let rule = AggregationRule::Sum {
            source_malo_ids: vec!["A".to_owned(), "B".to_owned()],
        };
        let result = compute_virtual_meter(&rule, &map).unwrap();
        assert_eq!(result[0].quality, QualityFlag::Estimated);
    }
}

#[cfg(test)]
mod ggv_allocation_tests {
    use super::*;
    use rust_decimal::dec;
    use time::{Duration, macros::datetime};

    fn series(values: &[Decimal]) -> Vec<MeterInterval> {
        values
            .iter()
            .enumerate()
            .map(|(i, &value)| {
                let from = datetime!(2026-06-01 12:00 UTC) + Duration::minutes(15 * i as i64);
                MeterInterval {
                    from,
                    to: from + Duration::minutes(15),
                    value,
                    quality: QualityFlag::Measured,
                    obis_code: None,
                }
            })
            .collect()
    }

    fn sources(pairs: &[(&str, &[Decimal])]) -> SourceMap {
        pairs
            .iter()
            .map(|(id, vals)| ((*id).to_owned(), series(vals)))
            .collect()
    }

    fn constant(fraction: Decimal) -> AggregationRule {
        AggregationRule::GgvConstantAllocation {
            plant_melo_id: "PLANT".to_owned(),
            tenant_melo_id: "T1".to_owned(),
            fraction,
        }
    }

    /// The identity a settlement reconciles against, exactly — all three are
    /// `Decimal`, so there is no rounding residue to explain away.
    #[test]
    fn consumption_is_exactly_allocated_plus_net() {
        let src = sources(&[
            ("PLANT", &[dec!(10), dec!(0), dec!(4), dec!(7.3)]),
            ("T1", &[dec!(1), dec!(3), dec!(4), dec!(2.9)]),
        ]);
        let out = compute_ggv_allocation(&constant(dec!(0.5)), &src).unwrap();
        assert_eq!(out.len(), 4);
        for iv in &out {
            assert_eq!(
                iv.consumption,
                iv.allocated + iv.net_grid_draw,
                "at {}",
                iv.from
            );
            assert!(iv.allocated >= Decimal::ZERO && iv.net_grid_draw >= Decimal::ZERO);
            assert!(iv.allocated <= iv.share, "the cap never credits more");
        }
    }

    /// `capped` is the number an operator wants: it says the tenant's share was
    /// limited by their own draw and the remainder fed the grid — which is the
    /// whole economics of § 42b Abs. 5.
    #[test]
    fn capping_and_the_surplus_that_fed_the_grid() {
        // 10 kWh generated, half allocated, 1 kWh drawn → 4 kWh surplus.
        let src = sources(&[("PLANT", &[dec!(10)]), ("T1", &[dec!(1)])]);
        let capped = &compute_ggv_allocation(&constant(dec!(0.5)), &src).unwrap()[0];
        assert_eq!(capped.share, dec!(5.0));
        assert_eq!(capped.allocated, dec!(1));
        assert_eq!(capped.net_grid_draw, dec!(0));
        assert!(capped.capped());
        assert_eq!(capped.surplus_to_grid(), dec!(4.0));

        // 10 kWh generated, half allocated, 8 kWh drawn → nothing spare.
        let src = sources(&[("PLANT", &[dec!(10)]), ("T1", &[dec!(8)])]);
        let short = &compute_ggv_allocation(&constant(dec!(0.5)), &src).unwrap()[0];
        assert_eq!(short.allocated, dec!(5.0));
        assert_eq!(short.net_grid_draw, dec!(3.0));
        assert!(!short.capped());
        assert_eq!(short.surplus_to_grid(), Decimal::ZERO);

        // Exactly equal is not capped — the tenant used their whole share.
        let src = sources(&[("PLANT", &[dec!(10)]), ("T1", &[dec!(5)])]);
        let exact = &compute_ggv_allocation(&constant(dec!(0.5)), &src).unwrap()[0];
        assert!(!exact.capped());
        assert_eq!(exact.net_grid_draw, Decimal::ZERO);
    }

    /// The proportional variant shares by actual draw, and the shares over all
    /// tenants exhaust the generation when nobody is capped.
    #[test]
    fn the_proportional_shares_exhaust_the_generation() {
        let src = sources(&[
            ("PLANT", &[dec!(9)]),
            ("T1", &[dec!(2)]),
            ("T2", &[dec!(4)]),
            ("T3", &[dec!(6)]),
        ]);
        let tenants: Vec<String> = ["T1", "T2", "T3"].iter().map(|s| (*s).to_owned()).collect();

        let mut total_share = Decimal::ZERO;
        for tenant in &tenants {
            let rule = AggregationRule::GgvProportionalAllocation {
                plant_melo_id: "PLANT".to_owned(),
                tenant_melo_id: tenant.clone(),
                all_tenant_melo_ids: tenants.clone(),
            };
            let iv = &compute_ggv_allocation(&rule, &src).unwrap()[0];
            assert_eq!(iv.generation, dec!(9));
            // Nobody draws less than their share here, so nothing is capped.
            assert!(!iv.capped(), "{tenant}");
            assert_eq!(iv.allocated, iv.share);
            total_share += iv.share;
        }
        // The shares exhaust the generation to the last representable place,
        // and never past it. Exhausting it *exactly* is what would require an
        // unrepresentable share: 4 ÷ 12 × 9 is 2.999…7 at `Decimal`'s full
        // width, and `ALLOCATION_DP` cuts it toward zero to 2.999999. Erring
        // downwards is deliberate — it keeps `Σ share ≤ generation`, so the
        // § 42b Abs. 5 ceiling holds without a second clamp.
        assert!(total_share <= dec!(9), "never over the generation");
        assert!(
            dec!(9) - total_share < dec!(0.00001),
            "and within a hundredth of a milliwatt-hour of it: {total_share}"
        );
        assert_eq!(total_share, dec!(8.999999));
    }

    /// A zero total is the Anwendungshilfe's division-by-zero guard. With
    /// quantities positive-or-zero it implies this tenant drew nothing, so the
    /// identity gives a zero net without needing a special case.
    #[test]
    fn a_zero_denominator_allocates_nothing() {
        let src = sources(&[
            ("PLANT", &[dec!(5)]),
            ("T1", &[dec!(0)]),
            ("T2", &[dec!(0)]),
        ]);
        let rule = AggregationRule::GgvProportionalAllocation {
            plant_melo_id: "PLANT".to_owned(),
            tenant_melo_id: "T1".to_owned(),
            all_tenant_melo_ids: vec!["T1".to_owned(), "T2".to_owned()],
        };
        let iv = &compute_ggv_allocation(&rule, &src).unwrap()[0];
        assert_eq!(iv.share, Decimal::ZERO);
        assert_eq!(iv.allocated, Decimal::ZERO);
        assert_eq!(iv.net_grid_draw, Decimal::ZERO);
        assert_eq!(iv.consumption, iv.allocated + iv.net_grid_draw);
    }

    /// The two entry points cannot drift: the net series is a projection of the
    /// allocation, not a second implementation of the same cap.
    #[test]
    fn the_net_series_is_a_projection_of_the_allocation() {
        let src = sources(&[
            ("PLANT", &[dec!(10), dec!(2), dec!(0)]),
            ("T1", &[dec!(1), dec!(3), dec!(4)]),
        ]);
        let rule = constant(dec!(0.5));
        let net = compute_virtual_meter(&rule, &src).unwrap();
        let full = compute_ggv_allocation(&rule, &src).unwrap();

        assert_eq!(net.len(), full.len());
        for (n, f) in net.iter().zip(&full) {
            assert_eq!(n, &f.to_net_interval());
            assert_eq!(n.value, f.net_grid_draw);
        }
    }

    /// Quality propagates from every contributor, so an estimated plant reading
    /// marks the tenant's allocation as estimated too.
    #[test]
    fn the_worst_contributor_sets_the_quality() {
        let mut src = sources(&[("PLANT", &[dec!(10)]), ("T1", &[dec!(8)])]);
        src.get_mut("PLANT").unwrap()[0].quality = QualityFlag::Estimated;
        let iv = &compute_ggv_allocation(&constant(dec!(0.5)), &src).unwrap()[0];
        assert_eq!(iv.quality, QualityFlag::Estimated);
        assert_eq!(iv.to_net_interval().quality, QualityFlag::Estimated);
    }

    /// A rule that allocates nothing is an error, not an empty result.
    #[test]
    fn a_non_ggv_rule_is_refused_by_name() {
        let src = sources(&[("A", &[dec!(1)]), ("B", &[dec!(2)])]);
        let err = compute_ggv_allocation(
            &AggregationRule::Sum {
                source_malo_ids: vec!["A".to_owned(), "B".to_owned()],
            },
            &src,
        )
        .unwrap_err();
        assert_eq!(
            err,
            VirtualMeterError::NotAGgvRule {
                kind: VirtualMeterKind::Sum
            }
        );
        assert!(err.to_string().contains("SUM"), "{err}");
    }

    #[test]
    fn missing_sources_and_bad_fractions_are_refused() {
        let src = sources(&[("PLANT", &[dec!(10)])]);
        assert!(matches!(
            compute_ggv_allocation(&constant(dec!(0.5)), &src),
            Err(VirtualMeterError::MissingSource(id)) if id == "T1"
        ));

        let src = sources(&[("PLANT", &[dec!(10)]), ("T1", &[dec!(1)])]);
        for bad in [dec!(0), dec!(-0.1), dec!(1.5)] {
            assert!(matches!(
                compute_ggv_allocation(&constant(bad), &src),
                Err(VirtualMeterError::InvalidFractions { .. })
            ));
        }
        assert!(compute_ggv_allocation(&constant(dec!(1)), &src).is_ok());
    }
}

#[cfg(test)]
mod community_tests {
    use super::*;
    use rust_decimal::dec;
    use std::collections::{BTreeMap, HashMap};
    use time::{Duration, macros::datetime};

    const BASE: OffsetDateTime = datetime!(2026-06-01 12:00 UTC);

    fn series(values: &[Decimal]) -> Vec<MeterInterval> {
        values
            .iter()
            .enumerate()
            .map(|(i, &value)| {
                let from = BASE + Duration::minutes(15 * i as i64);
                MeterInterval {
                    from,
                    to: from + Duration::minutes(15),
                    value,
                    quality: QualityFlag::Measured,
                    obis_code: None,
                }
            })
            .collect()
    }

    fn community() -> SourceMap {
        let mut sources: SourceMap = HashMap::new();
        sources.insert("PLANT".to_owned(), series(&[dec!(10), dec!(2), dec!(0)]));
        sources.insert("T1".to_owned(), series(&[dec!(1), dec!(4), dec!(2)]));
        sources.insert("T2".to_owned(), series(&[dec!(3), dec!(4), dec!(6)]));
        sources
    }

    fn proportional() -> AllocationKey {
        AllocationKey::Proportional {
            participants: vec!["T1".to_owned(), "T2".to_owned()],
        }
    }

    fn constant(t1: Decimal, t2: Decimal) -> AllocationKey {
        AllocationKey::Constant {
            fractions: BTreeMap::from([("T1".to_owned(), t1), ("T2".to_owned(), t2)]),
        }
    }

    /// The three identities the output has to satisfy in every interval, for
    /// both keys — all exact, because all three are `Decimal`.
    #[test]
    fn the_allocation_identities_hold_exactly() {
        let sources = community();
        for key in [proportional(), constant(dec!(0.25), dec!(0.5))] {
            let out = compute_community_allocation("PLANT", &key, &sources).unwrap();
            assert_eq!(out.len(), 3);
            for iv in &out {
                for p in &iv.participants {
                    assert_eq!(p.consumption, p.allocated + p.net_grid_draw, "{p:?}");
                    assert!(p.allocated <= p.consumption);
                    assert!(p.allocated <= p.share);
                }
                assert_eq!(
                    iv.generation,
                    iv.total_allocated() + iv.surplus_to_grid,
                    "generation splits into credited and exported",
                );
                assert_eq!(
                    iv.total_consumption,
                    iv.total_allocated() + iv.total_net_grid_draw(),
                );
            }
        }
    }

    /// § 42b Abs. 5 caps the *pool*, and the per-participant `Pos()` cap
    /// already implies it — so this is an assertion, not a clamp.
    #[test]
    fn the_pool_never_exceeds_the_paragraph_42b_ceiling() {
        let sources = community();
        for key in [
            proportional(),
            constant(dec!(0.25), dec!(0.5)),
            constant(dec!(0.5), dec!(0.5)), // fully subscribed
        ] {
            for iv in compute_community_allocation("PLANT", &key, &sources).unwrap() {
                assert_eq!(iv.pool_cap, iv.generation.min(iv.total_consumption));
                assert!(
                    iv.total_allocated() <= iv.pool_cap,
                    "allocated {} over the cap {} at {}",
                    iv.total_allocated(),
                    iv.pool_cap,
                    iv.from,
                );
            }
        }
    }

    /// The community view and the per-tenant view must not drift: the same
    /// input has to give the same tenant a byte-identical allocation.
    #[test]
    fn the_community_view_agrees_with_the_per_tenant_one() {
        let sources = community();
        let community = compute_community_allocation("PLANT", &proportional(), &sources).unwrap();

        for tenant in ["T1", "T2"] {
            let rule = AggregationRule::GgvProportionalAllocation {
                plant_melo_id: "PLANT".to_owned(),
                tenant_melo_id: tenant.to_owned(),
                all_tenant_melo_ids: vec!["T1".to_owned(), "T2".to_owned()],
            };
            let per_tenant = compute_ggv_allocation(&rule, &sources).unwrap();
            assert_eq!(per_tenant.len(), community.len());
            for (single, whole) in per_tenant.iter().zip(&community) {
                let p = whole.participant(tenant).expect("named participant");
                assert_eq!(single.from, whole.from);
                assert_eq!(single.consumption, p.consumption);
                assert_eq!(single.share, p.share);
                assert_eq!(single.allocated, p.allocated);
                assert_eq!(single.net_grid_draw, p.net_grid_draw);
                assert_eq!(single.capped(), p.capped());
            }
        }
    }

    /// The constant key is the one that *can* over-subscribe the plant, and
    /// the community view is the only place the sum is visible.
    #[test]
    fn constant_fractions_must_not_over_subscribe_the_plant() {
        let sources = community();
        let over = compute_community_allocation("PLANT", &constant(dec!(0.7), dec!(0.5)), &sources);
        assert_eq!(
            over,
            Err(VirtualMeterError::InvalidFractions { sum: dec!(1.2) }),
        );

        // Zero and negative shares are not shares.
        assert!(
            compute_community_allocation("PLANT", &constant(dec!(0), dec!(0.5)), &sources).is_err()
        );

        // Exactly fully subscribed is fine; the surplus is then only what the
        // tenants could not use.
        assert!(
            compute_community_allocation("PLANT", &constant(dec!(0.5), dec!(0.5)), &sources)
                .is_ok()
        );
    }

    /// A participant missing in one interval drops that interval rather than
    /// forming the denominator over a different set than the numerator.
    #[test]
    fn an_incomplete_interval_is_skipped_whole() {
        let mut sources = community();
        sources.insert("T2".to_owned(), series(&[dec!(3)])); // only the first
        let out = compute_community_allocation("PLANT", &proportional(), &sources).unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].total_consumption, dec!(4));
    }

    #[test]
    fn a_missing_series_is_named() {
        let sources = community();
        let key = AllocationKey::Proportional {
            participants: vec!["T1".to_owned(), "GHOST".to_owned()],
        };
        assert_eq!(
            compute_community_allocation("PLANT", &key, &sources),
            Err(VirtualMeterError::MissingSource("GHOST".to_owned())),
        );
        assert_eq!(
            compute_community_allocation("NO_PLANT", &proportional(), &sources),
            Err(VirtualMeterError::MissingSource("NO_PLANT".to_owned())),
        );
    }

    /// Nothing generated and nothing drawn is zero, not a division by zero.
    #[test]
    fn a_dead_interval_allocates_nothing() {
        let mut sources: SourceMap = HashMap::new();
        sources.insert("PLANT".to_owned(), series(&[dec!(0)]));
        sources.insert("T1".to_owned(), series(&[dec!(0)]));
        sources.insert("T2".to_owned(), series(&[dec!(0)]));
        let out = compute_community_allocation("PLANT", &proportional(), &sources).unwrap();
        assert_eq!(out[0].pool_cap, Decimal::ZERO);
        assert_eq!(out[0].total_allocated(), Decimal::ZERO);
        assert_eq!(out[0].surplus_to_grid, Decimal::ZERO);
    }

    /// Listing the participants in another order is the same community.
    #[test]
    fn the_participant_order_does_not_change_the_result() {
        let sources = community();
        let forward = compute_community_allocation("PLANT", &proportional(), &sources).unwrap();
        let reversed = AllocationKey::Proportional {
            participants: vec!["T2".to_owned(), "T1".to_owned()],
        };
        let backward = compute_community_allocation("PLANT", &reversed, &sources).unwrap();
        assert_eq!(forward, backward);
    }

    /// Quality is the worst contributor, plant included.
    #[test]
    fn a_substituted_tenant_reading_downgrades_the_whole_interval() {
        let mut sources = community();
        let mut t2 = series(&[dec!(3), dec!(4), dec!(6)]);
        t2[1].quality = QualityFlag::Substituted;
        sources.insert("T2".to_owned(), t2);
        let out = compute_community_allocation("PLANT", &proportional(), &sources).unwrap();
        assert_eq!(out[0].quality, QualityFlag::Measured);
        assert_eq!(out[1].quality, QualityFlag::Substituted);
    }
}

#[cfg(test)]
mod allocation_precision_tests {
    use super::*;
    use rust_decimal::dec;
    use std::collections::HashMap;
    use time::macros::datetime;

    fn one(value: Decimal) -> Vec<MeterInterval> {
        vec![MeterInterval {
            from: datetime!(2026-06-01 12:00 UTC),
            to: datetime!(2026-06-01 12:15 UTC),
            value,
            quality: QualityFlag::Measured,
            obis_code: None,
        }]
    }

    /// Three tenants drawing equally from a plant that cannot cover them
    /// divides by three. Without the cut, that quotient carries twenty-seven
    /// decimal places into `consumption − allocated`, which then rounds and
    /// breaks `consumption = allocated + net_grid_draw` by a few 1e-27 kWh.
    #[test]
    fn a_repeating_share_does_not_break_the_split_identity() {
        let mut sources: SourceMap = HashMap::new();
        sources.insert("PLANT".to_owned(), one(dec!(10)));
        for t in ["T1", "T2", "T3"] {
            sources.insert(t.to_owned(), one(dec!(40)));
        }
        let key = AllocationKey::Proportional {
            participants: vec!["T1".to_owned(), "T2".to_owned(), "T3".to_owned()],
        };
        let out = compute_community_allocation("PLANT", &key, &sources).unwrap();
        let iv = &out[0];

        for p in &iv.participants {
            assert_eq!(
                p.consumption,
                p.allocated + p.net_grid_draw,
                "{}: {} != {} + {}",
                p.id,
                p.consumption,
                p.allocated,
                p.net_grid_draw,
            );
            // The share is a quantity somebody could write on an invoice.
            assert!(p.share.scale() <= ALLOCATION_DP, "share {} is not", p.share);
            assert_eq!(p.share, dec!(3.333333), "10 ÷ 3, cut toward zero");
        }
        assert_eq!(iv.total_consumption, dec!(120));
        assert_eq!(iv.total_allocated(), dec!(9.999999));
        // Cutting toward zero keeps the allocation inside the generation, so
        // the surplus stays non-negative and the ceiling stays a theorem.
        assert_eq!(iv.surplus_to_grid, dec!(0.000001));
        assert!(iv.total_allocated() <= iv.pool_cap);
    }

    /// A participant whose whole draw is covered nets exactly zero, whatever
    /// scale their meter delivered — the consumption is never truncated.
    #[test]
    fn a_fully_covered_participant_nets_exactly_zero() {
        let mut sources: SourceMap = HashMap::new();
        sources.insert("PLANT".to_owned(), one(dec!(100)));
        // Seven decimal places, finer than ALLOCATION_DP.
        sources.insert("T1".to_owned(), one(dec!(1.2345678)));
        let key = AllocationKey::Proportional {
            participants: vec!["T1".to_owned()],
        };
        let p = &compute_community_allocation("PLANT", &key, &sources).unwrap()[0].participants[0];
        assert_eq!(
            p.allocated,
            dec!(1.2345678),
            "credited exactly what was drawn"
        );
        assert_eq!(p.net_grid_draw, Decimal::ZERO);
        assert!(p.capped());
    }
}
