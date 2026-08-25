//! Virtual meter rules and aggregation specifications.
//!
//! ## Use cases
//!
//! | Rule type | German term | Application |
//! |---|---|---|
//! | `Sum` | Summenmessung | Aggregation of parallel MaLos (e.g. grid intake + local PV) |
//! | `Residual` | Residuallast | Grid withdrawal = total load − own generation |
//! | `PvSelfConsumption` | PV-Eigenverbrauch | Consumed from own PV plant before grid feed-in |
//! | `GgvConstantAllocation` | GGV Nutzungsplan (konstant) | §42b EnWG constant-fraction allocation (UTILTS CCI+ZG6) |
//! | `GgvProportionalAllocation` | GGV Nutzungsplan (variabel) | §42b EnWG proportional consumption-based allocation |
//!
//! ## GGV allocation methods (§42b Abs. 5 EnWG — Solarpaket I)
//!
//! Both GGV variants compute the **net grid draw** after PV allocation for a
//! single tenant MaLo. The formula applies the `Pos()` (positive-value) operator
//! which caps the allocated PV at the tenant's actual consumption in each
//! 15-minute interval.
//!
//! **Constant allocation (CCI+ZG6, UTILTS Z82/ZG6):**
//! ```text
//! net_grid_draw = max(0, melo_consumption - fraction × melo_generation)
//! ```
//! — BDEW "Anwendungshilfe Solarpaket 1" Beispiel 1 (v1.0, 25.01.2024)
//!
//! **Proportional allocation (variable, UTILTS Z74 Divisionsquotient):**
//! ```text
//! ratio = melo_consumption / Σ all_tenant_consumption  (0 if denominator = 0)
//! net_grid_draw = max(0, melo_consumption - ratio × melo_generation)
//! ```
//! — BDEW "Anwendungshilfe Solarpaket 1" Beispiel 3
//!
//! ## Design: one rule per tenant MaLo
//!
//! Each GGV tenant has its own `virtual_meter_configs` row with a separate rule.
//! The rule references the shared PV plant MeLo plus the tenant's own consumption
//! MeLo. For proportional allocation the rule also lists all other tenant MeLos so
//! the denominator can be computed.
//!
//! ## Relationship to other modules
//!
//! - A rule is persisted configuration; this module only defines and evaluates it
//! - Evaluation produces the aggregated Lastgang for a virtual MaLo
//! - [`crate::aggregation`] performs the kWh arithmetic on individual intervals
//!
//! ## Regulatory basis
//!
//! - **§42b EnWG 2023 (Solarpaket I)** — Gemeinschaftliche Gebäudeversorgung (GGV)
//! - **BDEW "Anwendungshilfe Beispiele von Berechnungsformeln für das Solarpaket 1"** (25.01.2024 v1.0)
//! - **MaBiS (BNetzA BK6-07-002)**, in the consolidated Lesefassung of
//!   **BK6-24-174** — portfolio aggregation for Bilanzkreis settlement

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// An aggregation rule defining how a virtual MaLo's time series is computed.
///
/// Virtual meters aggregate two or more physical MaLo / MeLo time series. The
/// result is stored as a derived `meter_reads` entry with `source = "VIRTUAL"`.
///
/// **GGV rules output the tenant's net grid draw** (Bezug aus dem öffentlichen
/// Netz nach PV-Abzug) — the metered energy the tenant draws from the grid after
/// the allocated community PV has been subtracted. This matches the `Malo_i
/// Verbrauch` formula in the BDEW Anwendungshilfe. For the *allocated* amount,
/// which is what a § 42b or § 42c settlement is actually built on, use
/// [`compute_ggv_allocation`](crate::compute_ggv_allocation).
///
/// ## The `serde` shape
///
/// **Internally tagged** on a `kind` field carrying
/// [`VirtualMeterKind::as_str`], so the discriminator is one spelling at one
/// fixed path — indexable and queryable as a column value:
///
/// ```json
/// { "kind": "GGV_CONSTANT_ALLOCATION", "plant_melo_id": "…", "fraction": "0.1" }
/// ```
///
/// Internal tagging needs a self-describing format, so this type does **not**
/// round-trip through bincode or postcard. A rule is configuration, stored once
/// per delivery point in a queryable document; the hot types a binary format is
/// chosen for ([`MeterInterval`](crate::MeterInterval),
/// [`ObisCode`](crate::ObisCode)) are unaffected.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(
    feature = "serde",
    serde(tag = "kind", rename_all = "SCREAMING_SNAKE_CASE")
)]
pub enum AggregationRule {
    /// Sum of multiple MaLo time series — used for portfolio totals and
    /// Summenmessung (multiple parallel transformers, shared substations).
    ///
    /// `result[t] = Σ source_malo_ids[i][t]`
    Sum {
        /// MaLo IDs whose interval values are summed.
        source_malo_ids: Vec<String>,
    },

    /// Residual load: total minus one or more subtracted sources.
    ///
    /// `result[t] = total_malo_id[t] - Σ subtract_malo_ids[i][t]`
    ///
    /// Common application: grid feed-in metering at a building with local PV.
    /// The net grid intake = building load − PV generation.
    ///
    /// Net feed-in for a generation contract is the same shape: gross
    /// generation minus own consumption. This is arithmetic, not a regulated
    /// procedure — which series to subtract from which is the contract's to
    /// say.
    Residual {
        /// MaLo whose value is the minuend (total).
        total_malo_id: String,
        /// MaLo IDs whose values are subtracted.
        subtract_malo_ids: Vec<String>,
    },

    /// Net grid exchange at a prosumer's connection point.
    ///
    /// `result[t] = load[t] − generation[t]`, **signed**: positive is draw from
    /// the grid, negative is feed-in. One series, not three — the rest follow
    /// from it without further metering:
    ///
    /// ```text
    /// grid_draw[t]        = max(0,  result[t])
    /// grid_feed_in[t]     = max(0, -result[t])
    /// self_consumption[t] = min(generation[t], load[t]) = load[t] − grid_draw[t]
    /// ```
    PvSelfConsumption {
        /// MaLo for the total building load (from grid measurement point).
        grid_malo_id: String,
        /// MaLo for the PV generation (bidirectional meter or generation meter).
        generation_malo_id: String,
    },

    /// §42b EnWG GGV: constant-fraction allocation (UTILTS CCI+ZG6).
    ///
    /// Computes the **net grid draw** for a single tenant delivery point after
    /// deducting the statically allocated fraction of community PV generation.
    ///
    /// ## Formula (BDEW Anwendungshilfe Beispiel 1, §42b Abs. 5 EnWG)
    ///
    /// ```text
    /// net_grid_draw[t] = max(0, tenant_consumption[t] - fraction × plant_generation[t])
    /// ```
    ///
    /// where `max(0, x)` is the `Pos()` operator (UTILTS Z83).
    ///
    /// The allocated PV amount for this tenant is:
    /// ```text
    /// pv_allocated[t] = tenant_consumption[t] - net_grid_draw[t]
    ///                 = min(tenant_consumption[t], fraction × plant_generation[t])
    /// ```
    ///
    /// ## The `max(0, …)` cap
    ///
    /// No tenant is credited more PV than they drew in the interval. This is
    /// the `Pos()` operator of the BDEW Anwendungshilfe; § 42b Abs. 5 EnWG caps
    /// the **pool**, not the individual share. See
    /// [`crate::virtual_meter`] for the two bounds side by side.
    ///
    /// ## UTILTS encoding
    ///
    /// Transmitted as `CCI+ZG6` (Aufteilungsfaktor Energiemenge) in the UTILTS
    /// with `CAV+Z28:::fraction` encoding the static fraction.
    GgvConstantAllocation {
        /// MeLo ID of the shared PV plant generation measurement point.
        /// Corresponds to `Melo1 Erzeugung` in the BDEW Anwendungshilfe.
        plant_melo_id: String,
        /// MeLo ID of this tenant's consumption measurement point.
        /// Corresponds to `Melo_i Verbrauch` in the BDEW Anwendungshilfe.
        tenant_melo_id: String,
        /// Allocated fraction of plant generation for this tenant (0 < fraction ≤ 1).
        ///
        /// Example: `0.10` for 10%, `0.90` for 90%.
        /// Fractions across all tenants should sum to ≤ 1; the remainder feeds into the grid.
        fraction: rust_decimal::Decimal,
    },

    /// §42b EnWG GGV: variable consumption-proportional allocation.
    ///
    /// Computes the **net grid draw** for a single tenant delivery point using a
    /// dynamically computed ratio based on all tenants' actual consumption in each
    /// 15-minute interval.
    ///
    /// ## Formula (BDEW Anwendungshilfe Beispiel 3, §42b Abs. 5 EnWG)
    ///
    /// ```text
    /// total_consumption[t] = Σ consumption_j[t]   for all participating tenants j
    ///
    /// ratio[t] = tenant_consumption[t] / total_consumption[t]   (0 if total = 0)
    ///
    /// net_grid_draw[t] = max(0, tenant_consumption[t] - ratio[t] × plant_generation[t])
    /// ```
    ///
    /// ## Division-by-zero protection (§42b Abs. 5 EnWG implicit)
    ///
    /// When `total_consumption[t] = 0` (all tenants consume nothing), the ratio is
    /// 0 and no PV energy is allocated — `net_grid_draw[t] = max(0, 0) = 0`.
    /// This matches the BDEW Anwendungshilfe: "Ist die Energiemenge einer Marktlokation
    /// zugeordneten Messlokation = 0, so ist auch der Verbrauch der Marktlokation auf
    /// 0 zu setzen. Dies verhindert auch eine Division durch 0."
    GgvProportionalAllocation {
        /// MeLo ID of the shared PV plant generation measurement point.
        plant_melo_id: String,
        /// MeLo ID of this tenant's consumption measurement point.
        tenant_melo_id: String,
        /// MeLo IDs of **all** participating tenants (including this tenant's own MeLo).
        ///
        /// Required to compute the denominator `Σ all_tenant_consumption[t]`.
        all_tenant_melo_ids: Vec<String>,
    },
}

/// Which kind of [`AggregationRule`] produced a derived series.
///
/// The rule itself carries the MaLo IDs it references and is configuration; this
/// is the tag a *result* records, so a stored virtual-meter series can say what
/// made it without embedding the whole rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "SCREAMING_SNAKE_CASE"))]
pub enum VirtualMeterKind {
    /// [`AggregationRule::Sum`].
    Sum,
    /// [`AggregationRule::Residual`].
    Residual,
    /// [`AggregationRule::PvSelfConsumption`].
    PvSelfConsumption,
    /// [`AggregationRule::GgvConstantAllocation`].
    GgvConstantAllocation,
    /// [`AggregationRule::GgvProportionalAllocation`].
    GgvProportionalAllocation,
}

impl VirtualMeterKind {
    /// Stable DB/wire label. Matches the `serde` tag of both this type **and**
    /// [`AggregationRule`]'s internal `kind` field, and
    /// [`FromStr`](std::str::FromStr) input.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Sum => "SUM",
            Self::Residual => "RESIDUAL",
            Self::PvSelfConsumption => "PV_SELF_CONSUMPTION",
            Self::GgvConstantAllocation => "GGV_CONSTANT_ALLOCATION",
            Self::GgvProportionalAllocation => "GGV_PROPORTIONAL_ALLOCATION",
        }
    }

    /// Every kind, in declaration order.
    pub const ALL: [Self; 5] = [
        Self::Sum,
        Self::Residual,
        Self::PvSelfConsumption,
        Self::GgvConstantAllocation,
        Self::GgvProportionalAllocation,
    ];
}

crate::codes::string_codes! {
    VirtualMeterKind;
}

impl From<&AggregationRule> for VirtualMeterKind {
    fn from(rule: &AggregationRule) -> Self {
        rule.kind()
    }
}

impl AggregationRule {
    /// Which kind of rule this is.
    #[must_use]
    pub const fn kind(&self) -> VirtualMeterKind {
        match self {
            Self::Sum { .. } => VirtualMeterKind::Sum,
            Self::Residual { .. } => VirtualMeterKind::Residual,
            Self::PvSelfConsumption { .. } => VirtualMeterKind::PvSelfConsumption,
            Self::GgvConstantAllocation { .. } => VirtualMeterKind::GgvConstantAllocation,
            Self::GgvProportionalAllocation { .. } => VirtualMeterKind::GgvProportionalAllocation,
        }
    }

    /// All source MaLo / MeLo IDs referenced by this rule.
    ///
    /// Used to enumerate which underlying series must be fetched
    /// before computing the virtual meter value.
    #[must_use]
    pub fn source_malo_ids(&self) -> Vec<&str> {
        match self {
            Self::Sum { source_malo_ids } => source_malo_ids.iter().map(String::as_str).collect(),
            Self::Residual {
                total_malo_id,
                subtract_malo_ids,
            } => {
                let mut ids = vec![total_malo_id.as_str()];
                ids.extend(subtract_malo_ids.iter().map(String::as_str));
                ids
            }
            Self::PvSelfConsumption {
                grid_malo_id,
                generation_malo_id,
            } => vec![grid_malo_id.as_str(), generation_malo_id.as_str()],
            Self::GgvConstantAllocation {
                plant_melo_id,
                tenant_melo_id,
                ..
            } => vec![plant_melo_id.as_str(), tenant_melo_id.as_str()],
            Self::GgvProportionalAllocation {
                plant_melo_id,
                all_tenant_melo_ids,
                ..
            } => {
                let mut ids = vec![plant_melo_id.as_str()];
                ids.extend(all_tenant_melo_ids.iter().map(String::as_str));
                ids
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal::dec;

    #[test]
    fn sum_rule_lists_all_sources() {
        let rule = AggregationRule::Sum {
            source_malo_ids: vec!["MALO_A".to_owned(), "MALO_B".to_owned()],
        };
        let sources = rule.source_malo_ids();
        assert_eq!(sources, vec!["MALO_A", "MALO_B"]);
        assert_eq!(rule.kind(), VirtualMeterKind::Sum);
    }

    #[test]
    fn residual_rule_includes_total_and_subtracts() {
        let rule = AggregationRule::Residual {
            total_malo_id: "TOTAL".to_owned(),
            subtract_malo_ids: vec!["PV".to_owned()],
        };
        let sources = rule.source_malo_ids();
        assert!(sources.contains(&"TOTAL"));
        assert!(sources.contains(&"PV"));
        assert_eq!(rule.kind(), VirtualMeterKind::Residual);
    }

    #[test]
    fn ggv_constant_lists_plant_and_tenant() {
        let rule = AggregationRule::GgvConstantAllocation {
            plant_melo_id: "MELO_PLANT".to_owned(),
            tenant_melo_id: "MELO_T1".to_owned(),
            fraction: dec!(0.10),
        };
        let sources = rule.source_malo_ids();
        assert_eq!(sources.len(), 2);
        assert!(sources.contains(&"MELO_PLANT"));
        assert!(sources.contains(&"MELO_T1"));
        assert_eq!(rule.kind(), VirtualMeterKind::GgvConstantAllocation);
    }

    #[test]
    fn ggv_proportional_lists_plant_and_all_tenants() {
        let rule = AggregationRule::GgvProportionalAllocation {
            plant_melo_id: "MELO_PLANT".to_owned(),
            tenant_melo_id: "MELO_T1".to_owned(),
            all_tenant_melo_ids: vec!["MELO_T1".to_owned(), "MELO_T2".to_owned()],
        };
        let sources = rule.source_malo_ids();
        // plant + t1 + t2
        assert_eq!(sources.len(), 3);
        assert!(sources.contains(&"MELO_PLANT"));
        assert_eq!(rule.kind(), VirtualMeterKind::GgvProportionalAllocation);
    }
}
