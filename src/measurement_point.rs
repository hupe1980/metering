//! Measurement point — what is being metered, and on whose account.
//!
//! A [`MeasurementPoint`] binds one **OBIS register** on one meter to the
//! market location it is billed against, for a stated validity period. It is
//! the structural context that turns a bare [`crate::MeterInterval`] into a
//! reading somebody can invoice.
//!
//! ## One type, not two
//!
//! Earlier releases also carried a `MeterRegister` in a separate module. The
//! two modelled the same thing — MaLo, meter serial, OBIS code, direction,
//! validity — with two different direction enums (`EnergyFlow` and
//! `EnergyDirection`) that could disagree about the same register. They had
//! separate `is_import` / `is_bezug` predicates, and when those turned out to
//! be able to report a point as both import *and* export, the bug had to be
//! fixed twice because the concept existed twice.
//!
//! `MeterRegister`'s two genuinely distinct fields survive here: the
//! [`wandler_factor`](MeasurementPoint::wandler_factor), and the register unit
//! — which is now **derived** from the OBIS code
//! ([`ObisCode::register_unit`](crate::ObisCode::register_unit)) rather than
//! stored, since a stored unit can contradict the code that determines it.
//!
//! ## Relationship to MSCONS
//!
//! In MSCONS, each time series is identified by:
//! - `NAD+MS/MR` — sender/receiver market participant
//! - `LOC+172` — Marktlokations-ID (MaLo)
//! - `LOC+237` — Messlokations-ID (MeLo, optional)
//! - `PIA` — OBIS code
//!
//! `MeasurementPoint` binds all four together for a specific validity period.
//!
//! ## Multiple registers per MeLo
//!
//! A MeLo can have multiple registers at the same timestamp — e.g. HT (register 1)
//! and NT (register 2). Each register is a distinct `MeasurementPoint` with a
//! different OBIS code but the same MaLo and MeLo.
//!
//! ## Regulatory basis
//!
//! - **§ 2 MsbG**: MeLo is the physical measurement reference.
//! - **BDEW MaKo**: MaLo is the billing reference.
//! - **BSI TR-03109**: Zählpunkt-ID ties MeLo to the SMGW.

use rust_decimal::Decimal;
use time::Date;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use crate::interval::Sparte;
use crate::obis::{ObisCode, RegisterUnit};

// ── MarktRolle ────────────────────────────────────────────────────────────────

/// Market role responsible for this measurement point.
///
/// Governs which entity owns the metering obligation and which processes
/// are triggered by changes (Messstellen­betreiberwechsel, Lieferbeginn, etc.).
///
/// Source: BDEW Rollenmodell V2.2 §2.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "SCREAMING_SNAKE_CASE"))]
pub enum MarktRolle {
    /// Netzbetreiber — owns grid metering for supply accounting.
    Nb,
    /// Lieferant — owns load forecasting and billing.
    Lf,
    /// Messstellenbetreiber — physically operates the meter.
    Msb,
    /// Bilanzkreisverantwortlicher — balance group management.
    Bkv,
    /// Übertragungsnetzbetreiber (Strom) / FNB (Gas).
    Uenb,
    /// Einspeiseverantwortlicher — responsible for feed-in control.
    Eiv,
    /// Direktvermarkter — direct marketing of renewable energy.
    Direktvermarkter,
    /// Marktgebietsverantwortlicher (Gas).
    Mgv,
    /// Energieserviceanbieter des Anschlussnutzers (iMSys context).
    Esa,
}

impl MarktRolle {
    /// BDEW abbreviation.
    #[must_use]
    pub fn abbreviation(self) -> &'static str {
        match self {
            Self::Nb => "NB",
            Self::Lf => "LF",
            Self::Msb => "MSB",
            Self::Bkv => "BKV",
            Self::Uenb => "ÜNB",
            Self::Eiv => "EIV",
            Self::Direktvermarkter => "DV",
            Self::Mgv => "MGV",
            Self::Esa => "ESA",
        }
    }

    /// `true` for roles that receive meter data via MSCONS from NB/MSB.
    #[must_use]
    pub fn is_mscons_receiver(self) -> bool {
        matches!(
            self,
            Self::Lf | Self::Bkv | Self::Mgv | Self::Direktvermarkter
        )
    }
}

// ── EnergyFlow ────────────────────────────────────────────────────────────────

/// Which way energy flows at this measurement point.
///
/// The **one** direction enum in this crate. It is master data — a statement of
/// what the point is for — and the OBIS code is the metered fact. Where the two
/// disagree, [`MeasurementPoint::is_bezug`] and
/// [`is_einspeisung`](MeasurementPoint::is_einspeisung) believe the code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "SCREAMING_SNAKE_CASE"))]
pub enum EnergyFlow {
    /// Consumption — energy drawn from the grid.
    Consumption,
    /// Generation — energy fed into the grid (Einspeisung).
    Generation,
    /// Storage charging. Consumption from the grid's point of view, kept
    /// distinct because EEG and Redispatch treat a battery differently from a
    /// load.
    StorageCharge,
    /// Storage discharging. Generation from the grid's point of view.
    StorageDischarge,
    /// Bidirectional — a four-quadrant meter at one connection point, where
    /// neither direction is the point's purpose.
    Bidirectional,
}

impl EnergyFlow {
    /// Every variant, in declaration order.
    pub const ALL: [Self; 5] = [
        Self::Consumption,
        Self::Generation,
        Self::StorageCharge,
        Self::StorageDischarge,
        Self::Bidirectional,
    ];

    /// `true` when energy flows out of the grid at this point — consumption,
    /// or a storage unit charging.
    #[must_use]
    pub const fn draws_from_grid(self) -> bool {
        matches!(self, Self::Consumption | Self::StorageCharge)
    }

    /// `true` when energy flows into the grid at this point — generation, or a
    /// storage unit discharging.
    #[must_use]
    pub const fn feeds_grid(self) -> bool {
        matches!(self, Self::Generation | Self::StorageDischarge)
    }

    /// `true` for a storage point in either direction.
    #[must_use]
    pub const fn is_storage(self) -> bool {
        matches!(self, Self::StorageCharge | Self::StorageDischarge)
    }
}

// ── MeasurementPoint ─────────────────────────────────────────────────────────

/// The complete regulatory and physical context for a meter register.
///
/// Binds together: Marktlokation (MaLo), Messlokation (MeLo), OBIS register,
/// accountable market role, and energy flow direction.
///
/// ## Bitemporal validity
///
/// `valid_from` / `valid_to` track when this configuration was active.
/// This is essential for:
/// - Meter exchange events (MeLo changes, MSB change per WiM process)
/// - Supplier switch (Lieferbeginn/-ende)
/// - Register reconfiguration (HT↔NT tariff changes per §14a)
///
/// ## Virtual meters
///
/// Virtual meters (GGV community solar, Residuallast) also have `MeasurementPoint`
/// entries with `is_virtual = true`. Their `obis_code` is a conventional code
/// like `1-0:1.8.0` (total import) since virtual meters are logical.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct MeasurementPoint {
    /// 11-digit Marktlokations-ID (billing reference).
    pub malo_id: String,

    /// 33-character Messlokations-ID (physical metering reference).
    ///
    /// `None` for SLP customers without an explicit MeLo.
    pub melo_id: Option<String>,

    /// Physical meter serial number.
    ///
    /// `None` for virtual meters or when meter identity is not yet known.
    pub meter_serial: Option<String>,

    /// OBIS code identifying this register on the meter.
    pub obis_code: ObisCode,

    /// Energy commodity.
    pub sparte: Sparte,

    /// Energy flow direction for this register.
    pub energy_flow: EnergyFlow,

    /// Market role accountable for this measurement point.
    pub accountable_role: MarktRolle,

    /// 13-digit BDEW or DVGW Codenummer of the accountable market participant.
    pub accountable_mp_id: String,

    /// `true` when this is a virtual/derived measurement point (GGV, Residuallast).
    pub is_virtual: bool,

    /// Multiplier from the raw counter display to the real quantity
    /// (Wandlerfaktor).
    ///
    /// `1` for direct metering; 100–1000 for a Wandlermessung. **Every
    /// [`MeterInterval`](crate::MeterInterval) in this crate is post-Wandler**:
    /// the factor is applied when the counter is read, so this field is
    /// traceability, and applying it a second time inflates consumption by the
    /// factor.
    pub wandler_factor: Decimal,

    /// Validity start (German local date, inclusive).
    pub valid_from: Date,

    /// Validity end (German local date, inclusive).
    ///
    /// `None` = still active.
    pub valid_to: Option<Date>,
}

impl MeasurementPoint {
    /// `true` when this point is active on the given date.
    #[must_use]
    pub fn is_active(&self, on_date: Date) -> bool {
        on_date >= self.valid_from && self.valid_to.is_none_or(|end| on_date <= end)
    }

    /// `true` when this point measures Bezug — energy drawn from the grid.
    ///
    /// Decided by the **OBIS code**, which is the metered fact (direction is
    /// value group C — see [`ObisCode::is_import`]), falling back to
    /// [`energy_flow`](Self::energy_flow) only when the code carries no
    /// direction, as a gas or heat code does not.
    ///
    /// The two used to be combined with `||`, which let a point whose
    /// `energy_flow` said `Generation` and whose OBIS code said `1-0:1.8.0`
    /// answer `true` to both this and [`is_einspeisung`](Self::is_einspeisung).
    /// A pair of predicates that are simultaneously true is worse than either
    /// being wrong, because nothing downstream can detect the contradiction.
    #[must_use]
    pub fn is_bezug(&self) -> bool {
        if self.obis_code.is_import() {
            return true;
        }
        if self.obis_code.is_export() {
            return false;
        }
        matches!(self.energy_flow, EnergyFlow::Consumption)
    }

    /// `true` when this point measures Einspeisung — energy fed into the grid.
    ///
    /// The mirror of [`is_bezug`](Self::is_bezug), and mutually exclusive with
    /// it by construction.
    #[must_use]
    pub fn is_einspeisung(&self) -> bool {
        if self.obis_code.is_export() {
            return true;
        }
        if self.obis_code.is_import() {
            return false;
        }
        matches!(self.energy_flow, EnergyFlow::Generation)
    }

    /// `true` when this point measures Blindarbeit / Blindleistung —
    /// electricity C = 3…8, including the Q I–Q IV quadrant registers.
    #[must_use]
    pub fn is_reactive(&self) -> bool {
        self.obis_code.is_reactive()
    }

    /// `true` when this point measures Gas.
    #[must_use]
    pub fn is_gas(&self) -> bool {
        matches!(self.sparte, Sparte::Gas)
    }

    /// The unit this register counts in, derived from the OBIS code.
    ///
    /// `None` for a code whose unit this crate cannot name — see
    /// [`ObisCode::register_unit`].
    #[must_use]
    pub fn unit(&self) -> Option<RegisterUnit> {
        self.obis_code.register_unit()
    }

    /// Tariff register: `None` for the total, `Some(1)` for HT, `Some(2)` for
    /// NT — see [`ObisCode::tariff_register`].
    #[must_use]
    pub fn tariff_register(&self) -> Option<u8> {
        self.obis_code.tariff_register()
    }

    /// Apply the Wandlerfaktor to a raw counter reading.
    ///
    /// Only ever call this on a value straight off the meter display. Values
    /// that reach [`MeterInterval`](crate::MeterInterval) have already had it
    /// applied.
    #[must_use]
    pub fn apply_wandler(&self, raw_display_value: Decimal) -> Decimal {
        raw_display_value * self.wandler_factor
    }
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal::dec;
    use time::macros::date;

    fn bezug_point() -> MeasurementPoint {
        MeasurementPoint {
            malo_id: "51238696780".to_owned(),
            melo_id: Some("DE0012345678901234567890123456789".to_owned()),
            meter_serial: Some("MSN-001".to_owned()),
            obis_code: ObisCode::STROM_BEZUG_TOTAL,
            sparte: Sparte::Strom,
            energy_flow: EnergyFlow::Consumption,
            accountable_role: MarktRolle::Lf,
            accountable_mp_id: "9900987654321".to_owned(),
            is_virtual: false,
            wandler_factor: Decimal::ONE,
            valid_from: date!(2026 - 01 - 01),
            valid_to: None,
        }
    }

    #[test]
    fn active_within_validity() {
        let mp = bezug_point();
        assert!(mp.is_active(date!(2026 - 06 - 15)));
    }

    #[test]
    fn inactive_before_valid_from() {
        let mp = bezug_point();
        assert!(!mp.is_active(date!(2025 - 12 - 31)));
    }

    #[test]
    fn inactive_after_valid_to() {
        let mut mp = bezug_point();
        mp.valid_to = Some(date!(2026 - 06 - 30));
        assert!(!mp.is_active(date!(2026 - 07 - 01)));
        assert!(mp.is_active(date!(2026 - 06 - 30)));
    }

    #[test]
    fn bezug_detection() {
        let mp = bezug_point();
        assert!(mp.is_bezug());
        assert!(!mp.is_einspeisung());
    }

    #[test]
    fn einspeisung_detection() {
        let mut mp = bezug_point();
        mp.obis_code = ObisCode::STROM_EINSPEISUNG_TOTAL;
        mp.energy_flow = EnergyFlow::Generation;
        assert!(mp.is_einspeisung());
        assert!(!mp.is_bezug());
    }

    #[test]
    fn gas_detection() {
        let mut mp = bezug_point();
        mp.sparte = Sparte::Gas;
        mp.obis_code = ObisCode::GAS_VOLUME_M3;
        assert!(mp.is_gas());
    }

    #[test]
    fn virtual_meter_flag() {
        let mut mp = bezug_point();
        mp.is_virtual = true;
        assert!(mp.is_virtual);
    }

    /// The unit is read off the OBIS code, so it cannot contradict it — the
    /// failure a stored `unit` field made possible.
    #[test]
    fn the_unit_is_derived_from_the_obis_code() {
        let mut mp = bezug_point();
        assert_eq!(mp.unit(), Some(RegisterUnit::KiloWattHour));

        mp.obis_code = ObisCode::STROM_BLINDARBEIT_Q1;
        assert_eq!(mp.unit(), Some(RegisterUnit::KiloVarHour));

        mp.obis_code = ObisCode::STROM_BEZUG_MAXIMUM;
        assert_eq!(mp.unit(), Some(RegisterUnit::KiloWatt));
        assert!(!mp.unit().unwrap().is_cumulative(), "a maximum is a power");

        mp.obis_code = ObisCode::GAS_VOLUME_M3;
        assert_eq!(mp.unit(), Some(RegisterUnit::CubicMetre));

        mp.obis_code = ObisCode::WAERME_ENERGY;
        assert_eq!(mp.unit(), Some(RegisterUnit::KiloWattHourThermal));
    }

    /// The Wandlerfaktor, and the direction predicates the merged type
    /// inherited from `MeterRegister`.
    #[test]
    fn the_merged_type_carries_the_register_facts() {
        let mut mp = bezug_point();
        mp.wandler_factor = dec!(100);
        assert_eq!(mp.apply_wandler(dec!(1234)), dec!(123400));

        assert_eq!(mp.tariff_register(), None, "1-0:1.8.0 is the total");
        mp.obis_code = ObisCode::STROM_BEZUG_HT;
        assert_eq!(mp.tariff_register(), Some(1));
        mp.obis_code = ObisCode::STROM_BEZUG_NT;
        assert_eq!(mp.tariff_register(), Some(2));
    }

    /// The direction predicates are mutually exclusive — the invariant that
    /// had to be fixed twice while the concept existed twice.
    #[test]
    fn direction_predicates_are_mutually_exclusive() {
        let mut mp = bezug_point();
        // Master data and the OBIS code disagree on purpose.
        mp.energy_flow = EnergyFlow::Generation;
        mp.obis_code = ObisCode::STROM_BEZUG_TOTAL;
        assert!(mp.is_bezug(), "the metered code wins");
        assert!(!mp.is_einspeisung());

        // ...and where the code carries no direction, the master data decides.
        mp.obis_code = ObisCode::GAS_VOLUME_M3;
        assert!(!mp.is_bezug());
        assert!(mp.is_einspeisung());

        for flow in EnergyFlow::ALL {
            mp.energy_flow = flow;
            assert!(
                !(mp.is_bezug() && mp.is_einspeisung()),
                "{flow:?} must not be both"
            );
        }
    }

    #[test]
    fn energy_flow_groups_storage_with_the_direction_it_acts_in() {
        assert!(EnergyFlow::Consumption.draws_from_grid());
        assert!(EnergyFlow::StorageCharge.draws_from_grid());
        assert!(EnergyFlow::Generation.feeds_grid());
        assert!(EnergyFlow::StorageDischarge.feeds_grid());
        assert!(!EnergyFlow::Bidirectional.draws_from_grid());
        assert!(!EnergyFlow::Bidirectional.feeds_grid());
        assert!(EnergyFlow::StorageCharge.is_storage());
        assert!(!EnergyFlow::Consumption.is_storage());
    }

    #[test]
    fn markt_rolle_mscons_receiver() {
        assert!(MarktRolle::Lf.is_mscons_receiver());
        assert!(MarktRolle::Bkv.is_mscons_receiver());
        assert!(!MarktRolle::Nb.is_mscons_receiver());
        assert!(!MarktRolle::Msb.is_mscons_receiver());
    }
}
