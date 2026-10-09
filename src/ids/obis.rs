//! Typed OBIS codes ([`ObisCode`]) per IEC 62056-21 / DLMS-COSEM and the
//! EDI@Energy *Codeliste der OBIS-Kennzahlen und Medien* v2.5c.
//!
//! ## Format
//!
//! ```text
//! A-B:C.D.E*F
//! │ │ │ │ │ └ F  Vorwertzählerstand — not used in the German market (255)
//! │ │ │ │ └── E  Tarifstufe (0 = Total, 1 = HT, 2 = NT, … 62, 63 = Fehlerregister)
//! │ │ │ └──── D  Messart (6 = Maximum, 8 = Zählerstand, 9 = Vorschub, 29 = Lastgang)
//! │ │ └────── C  Messgröße (1 = Wirkleistung +, 2 = Wirkleistung −,
//! │ │           3–8 = Blindleistung positiv/negativ/Q I–Q IV)
//! │ └──────── B  Kanal (0–65; 66 only for Blindmehrarbeit)
//! └────────── A  medium, per the DLMS/COSEM Blue Book list that OMS Vol. 2
//!               adopts: 0 = abstract, 1 = electricity, 4 = Heizkostenverteiler,
//!               5 = cooling, 6 = heat, 7 = gas, 8 = cold water, 9 = hot water
//! ```
//!
//! Read the groups through the predicates: **direction is C alone**
//! (EDI@Energy §2.1: `1-b:1.x.y` Bezug, `1-b:2.x.y` Lieferung), **D is a
//! Messart, never a direction**, and **E = 63 is the Fehlerregister, not
//! tariff 63**.
//!
//! ## One code, one string
//!
//! - **[`Display`] writes the reduced form** — `*F` omitted when F is 255:
//!   `1-0:1.8.0`.
//! - **`{:#}` writes the full form**: `1-0:1.8.0*255`.
//! - **[`FromStr`] accepts both**, plus leading zeros and surrounding
//!   whitespace, and maps them onto the same value.
//!
//! The Codeliste v2.5c marks the electricity and thermal-energy diagrams
//! **"A B C D E werden im deutschen Energiemarkt verwendet"**, and §2.3 states
//! that *"Wertegruppe F wird für die Kommunikation im deutschen Gasmarkt
//! nicht verwendet"*. Use `s.parse::<ObisCode>()?.to_string()` for any
//! database, merge or map key.
//!
//! [`ObisCode::kind`] reads the [`Messart`]; [`unit`](ObisCode::unit),
//! [`tariff`](ObisCode::tariff) and
//! [`default_resolution`](ObisCode::default_resolution) derive from it and the
//! medium, so they cannot disagree.
//!
//! [`Display`]: std::fmt::Display
//! [`FromStr`]: std::str::FromStr
//!
//! ## Commonly used codes in German MaKo
//!
//! | Code | Description |
//! |---|---|
//! | `1-0:1.8.0` | Electricity forward active energy total (kWh) |
//! | `1-0:1.8.1` | Electricity forward active energy register 1 (HT) |
//! | `1-0:1.8.2` | Electricity forward active energy register 2 (NT) |
//! | `1-0:2.8.0` | Electricity reverse active energy (Einspeisung) |
//! | `1-0:1.29.0` | Wirkarbeit Bezug — Lastgang (kWh per interval) |
//! | `1-0:1.6.0` | Wirkleistung Bezug — Maximum (kW, Spitzenleistung) |
//! | `1-0:1.9.0` | Wirkarbeit Bezug — Vorschub (kWh over a period) |
//! | `1-0:3.8.0` | Blindarbeit positiv (kvarh) |
//! | `1-0:4.8.0` | Blindarbeit negativ (kvarh) |
//! | `1-0:5.8.0` … `1-0:8.8.0` | Blindarbeit Q I … Q IV (kvarh) |
//! | `7-0:3.0.0` | Gas Betriebsvolumen, Ausspeisung (m³) — *not* a Normvolumen |
//! | `7-0:13.2.0` | Gas Normvolumen umgewertet (m³) |
//! | `7-0:52.0.22` | Zustandszahl · `7-0:54.0.22` Brennwert |
//! | `6-0:1.0.0` | Heat energy (kWh_th) — medium 6, **not** 8 |
//! | `8-0:1.0.0` | Cold water volume (m³) |
//! | `9-0:1.0.0` | Hot water volume (m³) |

use std::fmt;
use std::str::FromStr;

use crate::error::{ParseError, ParseErrorKind};
use crate::series::interval::Unit;
use crate::time::resolution::Resolution;

/// What a register measures over time — read off value group D on the
/// electricity axis and off the medium and value group C elsewhere.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Messart {
    /// A cumulative register reading (electricity D = 8, *Zeitintegral 1*;
    /// gas, heat and water volume/energy registers).
    Zaehlerstand,
    /// Energy over an arbitrary period (electricity D = 9, *Zeitintegral 2*).
    Vorschub,
    /// Energy per equidistant interval (electricity D = 29, *Zeitintegral 5*;
    /// gas profile C = 99).
    Lastgang,
    /// A maximum demand (electricity D = 6).
    Maximum,
    /// An instantaneous value (electricity D = 7).
    Momentan,
    /// Anything else — a gas conversion parameter (C = 52, 54), a
    /// Heizkostenverteiler, an abstract or unmodelled code.
    Other,
}

/// A parsed OBIS code: `A-B:C.D.E*F`.
///
/// Both spellings of one channel are one value; see the
/// [module docs](self#one-code-one-string) for the canonical string. Ordering
/// is by value group, A to F.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ObisCode {
    /// Medium: 1 = electricity, 5 = cooling, 6 = heat, 7 = gas, 8 = cold water,
    /// 9 = hot water, 4 = Heizkostenverteiler.
    pub a: u8,
    /// Kanal. For electricity: *"Die Vergabe des Kanals erfolgt durch den MSB
    /// (Wertebereich 0 bis 65) und ist für die Identifizierung relevant"*, plus
    /// 66, which the Codeliste admits only for Blindmehrarbeit und
    /// Blindmehrleistung im Lieferschein. For gas the valid range is 0–64 and
    /// the group is irrelevant except on a thermal Lastgang, where B = 10
    /// selects the Bilanzierungs- and B = 20 the Abrechnungsbrennwert.
    pub b: u8,
    /// Messgröße, per medium. Electricity: 1 = ∑Li Wirkleistung **+**
    /// (Bezug), 2 = **−** (Lieferung), 3 = Blindleistung positiv, 4 = negativ,
    /// 5–8 = Q I–Q IV. Gas (Messgröße, Quelle, Richtung and Qualifikation
    /// together): 3 = Betriebsvolumen Ausspeisung gesamt, 13 = Normvolumen
    /// umgewertet, 52 = Zustandszahl, 54 = Brennwert.
    pub c: u8,
    /// Messart — **never a direction**: 6 = Maximum, 8 = Zählerstand
    /// (Zeitintegral 1), 9 = Vorschub (Zeitintegral 2), 29 = Lastgang
    /// (Zeitintegral 5), 7 = instantaneous value.
    pub d: u8,
    /// Tarifstufe: 0 = Total, 1 = HT, 2 = NT, … up to 62 (0–9 before
    /// 2023-10-01), and [`TARIFF_FEHLERREGISTER`] (63) for the fault counter,
    /// which is not a tariff.
    ///
    /// [`TARIFF_FEHLERREGISTER`]: Self::TARIFF_FEHLERREGISTER
    pub e: u8,
    /// Vorwertzählerstand: [`STORAGE_UNUSED`] (255) = not applicable, 0–99
    /// identify stored previous readings. Unused in the German market;
    /// [`Display`](fmt::Display) omits 255.
    ///
    /// [`STORAGE_UNUSED`]: Self::STORAGE_UNUSED
    pub f: u8,
}

impl ObisCode {
    /// Value group F when it does not apply (IEC 62056-6-1: 255 = not used).
    pub const STORAGE_UNUSED: u8 = 255;

    /// The longest string any [`ObisCode`] can render as —
    /// `255-255:255.255.255*255`, 23 ASCII bytes (19 in the reduced form).
    pub const MAX_LEN: usize = 23;

    /// Electricity forward active energy — total (Bezug, kWh), HT + NT.
    pub const STROM_BEZUG_TOTAL: Self = Self {
        a: 1,
        b: 0,
        c: 1,
        d: 8,
        e: 0,
        f: Self::STORAGE_UNUSED,
    };

    /// Electricity forward active energy — register 1 (HT, Hochtarif).
    pub const STROM_BEZUG_HT: Self = Self {
        a: 1,
        b: 0,
        c: 1,
        d: 8,
        e: 1,
        f: Self::STORAGE_UNUSED,
    };

    /// Electricity forward active energy — register 2 (NT, Niedertarif).
    pub const STROM_BEZUG_NT: Self = Self {
        a: 1,
        b: 0,
        c: 1,
        d: 8,
        e: 2,
        f: Self::STORAGE_UNUSED,
    };

    /// Electricity reverse active energy — total (Einspeisung, kWh, gesamt).
    pub const STROM_EINSPEISUNG_TOTAL: Self = Self {
        a: 1,
        b: 0,
        c: 2,
        d: 8,
        e: 0,
        f: Self::STORAGE_UNUSED,
    };

    /// Electricity reverse active energy — register 1 (HT Einspeisung).
    pub const STROM_EINSPEISUNG_HT: Self = Self {
        a: 1,
        b: 0,
        c: 2,
        d: 8,
        e: 1,
        f: Self::STORAGE_UNUSED,
    };

    /// Electricity reverse active energy — register 2 (NT Einspeisung).
    pub const STROM_EINSPEISUNG_NT: Self = Self {
        a: 1,
        b: 0,
        c: 2,
        d: 8,
        e: 2,
        f: Self::STORAGE_UNUSED,
    };

    /// Wirkarbeit Bezug — **Lastgang** (`1-0:1.29.0`), energy per equidistant
    /// interval, in **kWh**.
    ///
    /// The channel a [`MeterInterval`](crate::MeterInterval) usually carries
    /// (MSCONS PID 13018 / 13025). D = 29 is *Zeitintegral 5*, **not** a
    /// maximum: the peak-demand register is [`STROM_BEZUG_MAXIMUM`] (D = 6);
    /// [`MeterInterval::demand_kw`](crate::MeterInterval::demand_kw) derives kW.
    ///
    /// [`STROM_BEZUG_MAXIMUM`]: Self::STROM_BEZUG_MAXIMUM
    pub const STROM_BEZUG_LASTGANG: Self = Self {
        a: 1,
        b: 0,
        c: 1,
        d: 29,
        e: 0,
        f: Self::STORAGE_UNUSED,
    };

    /// Wirkarbeit Lieferung — Lastgang Einspeisung (`1-0:2.29.0`), in kWh.
    pub const STROM_EINSPEISUNG_LASTGANG: Self = Self {
        a: 1,
        b: 0,
        c: 2,
        d: 29,
        e: 0,
        f: Self::STORAGE_UNUSED,
    };

    /// Wirkleistung Bezug — **Maximum** (`1-0:1.6.0`), in kW.
    ///
    /// The Jahreshöchstleistung register priced under § 17 Abs. 2 StromNEV.
    pub const STROM_BEZUG_MAXIMUM: Self = Self {
        a: 1,
        b: 0,
        c: 1,
        d: 6,
        e: 0,
        f: Self::STORAGE_UNUSED,
    };

    /// Wirkarbeit Bezug — **Vorschub** (`1-0:1.9.0`), in kWh.
    ///
    /// Energy over an arbitrary period, the difference of two Zählerstände
    /// (D = 9, *Zeitintegral 2*). MSCONS PID 13019 carries it.
    pub const STROM_BEZUG_VORSCHUB: Self = Self {
        a: 1,
        b: 0,
        c: 1,
        d: 9,
        e: 0,
        f: Self::STORAGE_UNUSED,
    };

    /// Blindarbeit **positiv** — Zählerstand (`1-0:3.8.0`, kvarh).
    ///
    /// C = 3, "∑ Li Blindleistung positiv"; inductive/capacitive is a matter of
    /// quadrant ([`STROM_BLINDARBEIT_Q1`] … [`STROM_BLINDARBEIT_Q4`]).
    ///
    /// [`STROM_BLINDARBEIT_Q1`]: Self::STROM_BLINDARBEIT_Q1
    /// [`STROM_BLINDARBEIT_Q4`]: Self::STROM_BLINDARBEIT_Q4
    pub const STROM_BLINDARBEIT_POSITIV: Self = Self {
        a: 1,
        b: 0,
        c: 3,
        d: 8,
        e: 0,
        f: Self::STORAGE_UNUSED,
    };

    /// Blindarbeit **negativ** — Zählerstand (`1-0:4.8.0`, kvarh).
    pub const STROM_BLINDARBEIT_NEGATIV: Self = Self {
        a: 1,
        b: 0,
        c: 4,
        d: 8,
        e: 0,
        f: Self::STORAGE_UNUSED,
    };

    /// Blindarbeit Quadrant I — Zählerstand (`1-0:5.8.0`, kvarh).
    pub const STROM_BLINDARBEIT_Q1: Self = Self {
        a: 1,
        b: 0,
        c: 5,
        d: 8,
        e: 0,
        f: Self::STORAGE_UNUSED,
    };

    /// Blindarbeit Quadrant II — Zählerstand (`1-0:6.8.0`, kvarh).
    pub const STROM_BLINDARBEIT_Q2: Self = Self {
        a: 1,
        b: 0,
        c: 6,
        d: 8,
        e: 0,
        f: Self::STORAGE_UNUSED,
    };

    /// Blindarbeit Quadrant III — Zählerstand (`1-0:7.8.0`, kvarh).
    pub const STROM_BLINDARBEIT_Q3: Self = Self {
        a: 1,
        b: 0,
        c: 7,
        d: 8,
        e: 0,
        f: Self::STORAGE_UNUSED,
    };

    /// Blindarbeit Quadrant IV — Zählerstand (`1-0:8.8.0`, kvarh).
    pub const STROM_BLINDARBEIT_Q4: Self = Self {
        a: 1,
        b: 0,
        c: 8,
        d: 8,
        e: 0,
        f: Self::STORAGE_UNUSED,
    };

    /// Gas **Betriebsvolumen** — Zählerstand, Ausspeisung (`7-0:3.0.0`, m³).
    ///
    /// Volume at metering conditions, before the Zustandszahl — the input
    /// [`gas_m3_to_kwh_hs`](crate::gas::conversion::gas_m3_to_kwh_hs) expects.
    /// Not [`GAS_NORMVOLUMEN_UMGEWERTET`], which is already state-converted and
    /// must not be multiplied by a Zustandszahl again.
    ///
    /// [`GAS_NORMVOLUMEN_UMGEWERTET`]: Self::GAS_NORMVOLUMEN_UMGEWERTET
    pub const GAS_VOLUME_M3: Self = Self {
        a: 7,
        b: 0,
        c: 3,
        d: 0,
        e: 0,
        f: Self::STORAGE_UNUSED,
    };

    /// Gas **Normvolumen umgewertet** — Zählerstand, Ausspeisung
    /// (`7-0:13.2.0`, m³).
    ///
    /// Converted to standard conditions by the Mengenumwerter: needs the
    /// Brennwert but **not** the Zustandszahl.
    pub const GAS_NORMVOLUMEN_UMGEWERTET: Self = Self {
        a: 7,
        b: 0,
        c: 13,
        d: 2,
        e: 0,
        f: Self::STORAGE_UNUSED,
    };

    /// Gas **Zustandszahl** — Mittelwert (`7-0:52.0.22`, dimensionless).
    pub const GAS_ZUSTANDSZAHL: Self = Self {
        a: 7,
        b: 0,
        c: 52,
        d: 0,
        e: 22,
        f: Self::STORAGE_UNUSED,
    };

    /// Gas **Brennwert** — Monatsmittelwert (`7-0:54.0.22`, kWh/m³).
    ///
    /// Value group E selects the averaging period: 16 = hourly, 20 = daily,
    /// 22 = monthly.
    pub const GAS_BRENNWERT_MONATSMITTEL: Self = Self {
        a: 7,
        b: 0,
        c: 54,
        d: 0,
        e: 22,
        f: Self::STORAGE_UNUSED,
    };

    /// Heat energy (kWh_th) — medium **6**, per the DLMS/COSEM Blue Book
    /// value-group-A list that OMS Spec Vol. 2 adopts. Medium 8 is cold water
    /// ([`WASSER_KALT_VOLUME`]).
    ///
    /// [`WASSER_KALT_VOLUME`]: Self::WASSER_KALT_VOLUME
    pub const WAERME_ENERGY: Self = Self {
        a: 6,
        b: 0,
        c: 1,
        d: 0,
        e: 0,
        f: Self::STORAGE_UNUSED,
    };

    /// Cooling energy (kWh_th) — medium 5.
    pub const KAELTE_ENERGY: Self = Self {
        a: 5,
        b: 0,
        c: 1,
        d: 0,
        e: 0,
        f: Self::STORAGE_UNUSED,
    };

    /// Cold water volume (m³) — medium 8.
    pub const WASSER_KALT_VOLUME: Self = Self {
        a: 8,
        b: 0,
        c: 1,
        d: 0,
        e: 0,
        f: Self::STORAGE_UNUSED,
    };

    /// Hot water volume (m³) — medium 9.
    ///
    /// Its heat share is [`crate::heat::heizkosten::warm_water_heat_kwh`]
    /// (HeizkostenV §9 Abs. 2).
    pub const WASSER_WARM_VOLUME: Self = Self {
        a: 9,
        b: 0,
        c: 1,
        d: 0,
        e: 0,
        f: Self::STORAGE_UNUSED,
    };

    /// `true` when this code refers to electricity (medium A = 1).
    #[must_use]
    pub fn is_electricity(&self) -> bool {
        self.a == 1
    }

    /// `true` when this code refers to gas (medium A = 7).
    #[must_use]
    pub fn is_gas(&self) -> bool {
        self.a == 7
    }

    /// `true` when this code refers to thermal energy — cooling (A = 5) or
    /// heat (A = 6).
    #[must_use]
    pub fn is_heat(&self) -> bool {
        matches!(self.a, 5 | 6)
    }

    /// `true` when this code refers to water — cold (A = 8) or hot (A = 9).
    #[must_use]
    pub fn is_water(&self) -> bool {
        matches!(self.a, 8 | 9)
    }

    /// `true` when this code refers to a Heizkostenverteiler (A = 4).
    ///
    /// HCAs report dimensionless *Verbrauchseinheiten* and carry **no
    /// Eichfrist**: they are not Messgeräte under MessEG/MessEV, and
    /// HeizkostenV §5 Abs. 1 admits them *"soweit nicht
    /// eichrechtliche Bestimmungen zur Anwendung kommen"* (EN 834 / EN 835).
    /// An Eichfrist check must skip them.
    #[must_use]
    pub fn is_heat_cost_allocator(&self) -> bool {
        self.a == 4
    }

    /// `true` when this code measures reactive energy or power (Blindarbeit /
    /// Blindleistung) — electricity C = 3…8.
    ///
    /// C = 3 positiv, 4 negativ, 5…8 the quadrants Q I…Q IV.
    #[must_use]
    pub fn is_reactive(&self) -> bool {
        self.a == 1 && matches!(self.c, 3..=8)
    }

    /// `true` when this code is a **Lastgang** — energy per equidistant interval
    /// (D = 29, *Zeitintegral 5*).
    ///
    /// An energy in kWh, **not** a power; see [`is_maximum`](Self::is_maximum).
    #[must_use]
    pub fn is_lastgang(&self) -> bool {
        self.d == 29
    }

    /// `true` when this code is a **maximum** register (D = 6).
    ///
    /// `1-0:1.6.0` is the Jahreshöchstleistung priced under § 17 Abs. 2 StromNEV.
    #[must_use]
    pub fn is_maximum(&self) -> bool {
        self.d == 6
    }

    /// `true` when this code is a **Zählerstand** — a cumulative meter reading
    /// (D = 8, *Zeitintegral 1*).
    #[must_use]
    pub fn is_zaehlerstand(&self) -> bool {
        self.d == 8
    }

    /// `true` when this code is a **Vorschub** — the energy consumed over an
    /// arbitrary period, i.e. the difference of two Zählerstände (D = 9,
    /// *Zeitintegral 2*).
    #[must_use]
    pub fn is_vorschub(&self) -> bool {
        self.d == 9
    }

    /// The direction this code counts in, if it counts in one.
    ///
    /// Read off electricity value group **C alone** (EDI@Energy §2.1): C = 1
    /// is Bezug and C = 2 Lieferung, whatever D and E. `None` for every other
    /// code — reactive, gas, heat, water.
    ///
    /// ```rust
    /// use metering::{Direction, ObisCode};
    ///
    /// let bezug: ObisCode = "1-0:1.8.0".parse()?;
    /// let einspeisung: ObisCode = "1-0:2.8.0".parse()?;
    /// let blind: ObisCode = "1-0:3.8.0".parse()?;
    ///
    /// assert_eq!(bezug.direction(), Some(Direction::Import));
    /// assert_eq!(einspeisung.direction(), Some(Direction::Export));
    /// assert_eq!(blind.direction(), None, "Blindarbeit has no flow direction");
    /// # Ok::<(), metering::ParseError>(())
    /// ```
    #[must_use]
    pub const fn direction(self) -> Option<crate::series::interval::Direction> {
        use crate::series::interval::Direction;
        match (self.a, self.c) {
            (1, 1) => Some(Direction::Import),
            (1, 2) => Some(Direction::Export),
            _ => None,
        }
    }

    /// `true` when this code counts Bezug — see [`direction`](Self::direction).
    #[must_use]
    pub const fn is_import(self) -> bool {
        matches!(
            self.direction(),
            Some(crate::series::interval::Direction::Import)
        )
    }

    /// `true` when this code counts Einspeisung — see
    /// [`direction`](Self::direction).
    #[must_use]
    pub const fn is_export(self) -> bool {
        matches!(
            self.direction(),
            Some(crate::series::interval::Direction::Export)
        )
    }

    /// Value group E of the Fehlerregister (EDI@Energy Codeliste §2.2), a
    /// fault counter rather than a tariff.
    pub const TARIFF_FEHLERREGISTER: u8 = 63;

    /// The [`Messart`] of this code.
    ///
    /// ```rust
    /// use metering::ids::obis::{Messart, ObisCode};
    ///
    /// assert_eq!(ObisCode::STROM_BEZUG_TOTAL.kind(), Messart::Zaehlerstand);
    /// assert_eq!(ObisCode::STROM_BEZUG_LASTGANG.kind(), Messart::Lastgang);
    /// assert_eq!(ObisCode::STROM_BEZUG_MAXIMUM.kind(), Messart::Maximum);
    /// assert_eq!(ObisCode::GAS_ZUSTANDSZAHL.kind(), Messart::Other);
    /// ```
    #[must_use]
    pub const fn kind(&self) -> Messart {
        match self.a {
            1 => match self.d {
                8 => Messart::Zaehlerstand,
                9 => Messart::Vorschub,
                29 => Messart::Lastgang,
                6 => Messart::Maximum,
                7 => Messart::Momentan,
                _ => Messart::Other,
            },
            7 => match self.c {
                52 | 54 => Messart::Other,
                99 => Messart::Lastgang,
                _ => Messart::Zaehlerstand,
            },
            5 | 6 | 8 | 9 => Messart::Zaehlerstand,
            _ => Messart::Other,
        }
    }

    /// The physical unit this code counts in, derived from
    /// [`kind`](Self::kind) and the medium.
    ///
    /// | Code | Unit |
    /// |---|---|
    /// | electricity active, Zählerstand/Vorschub/Lastgang | kWh |
    /// | electricity active, Maximum/Momentan | kW |
    /// | electricity reactive (C = 3…8), energy / power | kvarh / kvar |
    /// | gas Zustandszahl (C = 52) | dimensionless |
    /// | gas Brennwert (C = 54) | kWh/m³ |
    /// | other gas, water | m³ |
    /// | heat, cooling | kWh |
    ///
    /// `None` for an abstract code, a Heizkostenverteiler, or an electricity
    /// Messgröße or Messart outside the table.
    ///
    /// ```rust
    /// use metering::{ObisCode, Unit};
    ///
    /// assert_eq!(ObisCode::STROM_BEZUG_TOTAL.unit(), Some(Unit::KiloWattHour));
    /// assert_eq!(ObisCode::STROM_BEZUG_MAXIMUM.unit(), Some(Unit::KiloWatt));
    /// assert_eq!("1-0:5.6.0".parse::<ObisCode>()?.unit(), Some(Unit::KiloVar));
    /// assert_eq!(ObisCode::GAS_ZUSTANDSZAHL.unit(), Some(Unit::Dimensionless));
    /// assert_eq!(ObisCode::GAS_BRENNWERT_MONATSMITTEL.unit(), Some(Unit::KiloWattHourPerCubicMetre));
    /// assert_eq!(ObisCode::GAS_VOLUME_M3.unit(), Some(Unit::CubicMetre));
    /// assert_eq!("4-0:1.0.0".parse::<ObisCode>()?.unit(), None);
    /// # Ok::<(), metering::ParseError>(())
    /// ```
    #[must_use]
    pub const fn unit(&self) -> Option<Unit> {
        match self.a {
            1 => {
                let reactive = matches!(self.c, 3..=8);
                if !reactive && !matches!(self.c, 1 | 2) {
                    return None;
                }
                match (self.kind(), reactive) {
                    (Messart::Zaehlerstand | Messart::Vorschub | Messart::Lastgang, false) => {
                        Some(Unit::KiloWattHour)
                    }
                    (Messart::Zaehlerstand | Messart::Vorschub | Messart::Lastgang, true) => {
                        Some(Unit::KiloVarHour)
                    }
                    (Messart::Maximum | Messart::Momentan, false) => Some(Unit::KiloWatt),
                    (Messart::Maximum | Messart::Momentan, true) => Some(Unit::KiloVar),
                    (Messart::Other, _) => None,
                }
            }
            7 => match self.c {
                52 => Some(Unit::Dimensionless),
                54 => Some(Unit::KiloWattHourPerCubicMetre),
                _ => Some(Unit::CubicMetre),
            },
            5 | 6 => Some(Unit::KiloWattHour),
            8 | 9 => Some(Unit::CubicMetre),
            _ => None,
        }
    }

    /// The tariff (Tarifstufe) of an **energy register**: `Some(1)` = HT,
    /// `Some(2)` = NT, up to `Some(62)`.
    ///
    /// `None` for the total register (E = 0), the Fehlerregister (E = 63),
    /// and every code that is not an energy Zählerstand, Vorschub or Lastgang
    /// (on a gas Brennwert, E names the averaging period).
    ///
    /// ```rust
    /// use metering::ObisCode;
    ///
    /// assert_eq!(ObisCode::STROM_BEZUG_HT.tariff(), Some(1));
    /// assert_eq!(ObisCode::STROM_BEZUG_TOTAL.tariff(), None);
    /// // E = 22 on a Brennwert is "Monatsmittel", not tariff 22.
    /// assert_eq!(ObisCode::GAS_BRENNWERT_MONATSMITTEL.tariff(), None);
    /// ```
    #[must_use]
    pub const fn tariff(&self) -> Option<u8> {
        let energy = matches!(self.unit(), Some(u) if u.is_energy());
        let register = matches!(
            self.kind(),
            Messart::Zaehlerstand | Messart::Vorschub | Messart::Lastgang
        );
        match self.e {
            0 | Self::TARIFF_FEHLERREGISTER => None,
            n if energy && register => Some(n),
            _ => None,
        }
    }

    /// `true` when this is the Fehlerregister (E = 63), whose fault count must
    /// never be summed into an Arbeitsmenge.
    #[must_use]
    pub const fn is_fehlerregister(&self) -> bool {
        self.e == Self::TARIFF_FEHLERREGISTER
    }

    /// `true` when this is the total / combined register (E = 0).
    #[must_use]
    pub const fn is_total_register(&self) -> bool {
        self.e == 0
    }

    /// The grid a series on this channel is ordinarily delivered on.
    ///
    /// A default for when a message carries no resolution; the series' own is
    /// [`Series::resolution`](crate::Series::resolution). `None` where the
    /// channel is not an equidistant series:
    ///
    /// | Channel | Answer | Why |
    /// |---|---|---|
    /// | Fehlerregister, E = 63 | `None` | counts faults, not energy over time |
    /// | Maximum, electricity D = 6 | `None` | one figure per billing period |
    /// | Vorschub, electricity D = 9 | `None` | an arbitrary period, by definition off-grid |
    /// | Momentanwert, electricity D = 7 | `None` | an instant, not an interval |
    /// | electricity Zählerstand / Lastgang | `PT15M` | § 2 Satz 1 Nr. 27 MsbG |
    /// | gas Zustandszahl / Brennwert, C = 52 / 54 | from value group E | 16 hourly, 20 daily, 22 monthly |
    /// | other gas | `PT1H` | § 2 Satz 1 Nr. 27 MsbG: *stündlich* for gas |
    /// | heat, cooling (A = 5, 6) | `PT1H` | |
    /// | cold, hot water (A = 8, 9) | `P1D` | submetering is read daily at best |
    /// | anything else | `None` | |
    ///
    /// ```rust
    /// use metering::{Resolution, ObisCode};
    ///
    /// assert_eq!(
    ///     ObisCode::STROM_BEZUG_LASTGANG.default_resolution(),
    ///     Some(Resolution::QUARTER_HOUR),
    /// );
    /// // A Brennwert published as a monthly mean is not an hourly series.
    /// assert_eq!(
    ///     ObisCode::GAS_BRENNWERT_MONATSMITTEL.default_resolution(),
    ///     Some(Resolution::Month),
    /// );
    /// // A maximum register is one number per period, whether active…
    /// assert_eq!(ObisCode::STROM_BEZUG_MAXIMUM.default_resolution(), None);
    /// // …or reactive.
    /// assert_eq!("1-0:5.6.0".parse::<ObisCode>()?.default_resolution(), None);
    /// # Ok::<(), metering::ParseError>(())
    /// ```
    #[must_use]
    pub const fn default_resolution(&self) -> Option<Resolution> {
        if self.is_fehlerregister() {
            return None;
        }
        match (self.kind(), self.a) {
            (Messart::Zaehlerstand | Messart::Lastgang, 1) => Some(Resolution::QUARTER_HOUR),
            (Messart::Zaehlerstand | Messart::Lastgang, 5..=7) => Some(Resolution::Hour),
            (Messart::Zaehlerstand | Messart::Lastgang, 8 | 9) => Some(Resolution::Day),
            (Messart::Other, 7) => match self.e {
                16 => Some(Resolution::Hour),
                20 => Some(Resolution::Day),
                22 => Some(Resolution::Month),
                _ => None,
            },
            _ => None,
        }
    }

    /// The **Lastgang** channel of this register — the same measurement as a
    /// series of energies per equidistant interval (D = 29).
    ///
    /// The label for a Zählerstandsgang differenced into intervals:
    /// `1-0:1.8.0` → `1-0:1.29.0`.
    ///
    /// `None` unless the code is electricity (A = 1) with an active or reactive
    /// Messgröße (C = 1…8) and is the total register (E = 0): the Codeliste
    /// v2.5c §3.1 lists the Lastgang only as `1-b:1.29.0`, with no tariff
    /// variant. Split a total Lastgang across registers with
    /// [`Zaehlzeitdefinition`](crate::billing::zaehlzeit::Zaehlzeitdefinition).
    ///
    /// ```rust
    /// use metering::ObisCode;
    ///
    /// let bezug: ObisCode = "1-0:1.8.0".parse()?;
    /// assert_eq!(bezug.as_lastgang(), Some(ObisCode::STROM_BEZUG_LASTGANG));
    ///
    /// // Direction is preserved.
    /// let einspeisung: ObisCode = "1-0:2.8.0".parse()?;
    /// assert_eq!(
    ///     einspeisung.as_lastgang(),
    ///     Some(ObisCode::STROM_EINSPEISUNG_LASTGANG),
    /// );
    ///
    /// // ...and it round-trips.
    /// assert_eq!(bezug.as_lastgang().unwrap().as_zaehlerstand(), Some(bezug));
    ///
    /// // No Lastgang exists for a tariff register or off the electricity axis.
    /// assert_eq!("1-0:1.8.1".parse::<ObisCode>()?.as_lastgang(), None);
    /// assert_eq!(ObisCode::GAS_VOLUME_M3.as_lastgang(), None);
    /// # Ok::<(), metering::ParseError>(())
    /// ```
    #[must_use]
    pub const fn as_lastgang(&self) -> Option<Self> {
        if !self.is_convertible_messart() || self.e != 0 {
            return None;
        }
        Some(Self { d: 29, ..*self })
    }

    /// The **Zählerstand** channel of this register — the cumulative reading
    /// the Lastgang was differenced out of (D = 8, *Zeitintegral 1*).
    ///
    /// The inverse of [`as_lastgang`](Self::as_lastgang), defined for every
    /// tariff (`1-b:1.8.e`). `None` unless electricity with C = 1…8.
    #[must_use]
    pub const fn as_zaehlerstand(&self) -> Option<Self> {
        if !self.is_convertible_messart() {
            return None;
        }
        Some(Self { d: 8, ..*self })
    }

    /// The **Vorschub** channel of this register — the energy over an arbitrary
    /// period, i.e. the difference of two Zählerstände (D = 9, *Zeitintegral
    /// 2*).
    ///
    /// The label for what
    /// [`consumption_between`](crate::series::reading::consumption_between)
    /// computes. Defined for every tariff; `None` unless electricity with
    /// C = 1…8.
    #[must_use]
    pub const fn as_vorschub(&self) -> Option<Self> {
        if !self.is_convertible_messart() {
            return None;
        }
        Some(Self { d: 9, ..*self })
    }

    /// Electricity with an active or reactive Messgröße (C = 1…8).
    const fn is_convertible_messart(&self) -> bool {
        self.a == 1 && matches!(self.c, 1..=8)
    }

    /// `true` when value group F is [`STORAGE_UNUSED`], so
    /// [`Display`](fmt::Display) omits the `*F` suffix.
    ///
    /// [`STORAGE_UNUSED`]: Self::STORAGE_UNUSED
    #[must_use]
    pub const fn has_unused_storage(&self) -> bool {
        self.f == Self::STORAGE_UNUSED
    }
}

/// A stack buffer sized for the longest code [`ObisCode`] can write, so
/// [`Display`](fmt::Display) can hand a complete `&str` to
/// [`Formatter::pad`](fmt::Formatter::pad) without allocating.
struct CodeBuf {
    buf: [u8; ObisCode::MAX_LEN],
    len: usize,
}

impl CodeBuf {
    const fn new() -> Self {
        Self {
            buf: [0; ObisCode::MAX_LEN],
            len: 0,
        }
    }

    /// Always valid UTF-8: only ASCII digits and separators are ever written.
    fn as_str(&self) -> Result<&str, fmt::Error> {
        std::str::from_utf8(&self.buf[..self.len]).map_err(|_| fmt::Error)
    }
}

impl fmt::Write for CodeBuf {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        let end = self.len.checked_add(s.len()).ok_or(fmt::Error)?;
        let dst = self.buf.get_mut(self.len..end).ok_or(fmt::Error)?;
        dst.copy_from_slice(s.as_bytes());
        self.len = end;
        Ok(())
    }
}

impl fmt::Display for ObisCode {
    /// Writes the reduced form, omitting `*F` when F is
    /// [`STORAGE_UNUSED`](Self::STORAGE_UNUSED); `{:#}` forces the full form.
    /// Width, fill and alignment are honoured.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use fmt::Write as _;

        let mut buf = CodeBuf::new();
        write!(
            buf,
            "{}-{}:{}.{}.{}",
            self.a, self.b, self.c, self.d, self.e
        )?;
        if f.alternate() || !self.has_unused_storage() {
            write!(buf, "*{}", self.f)?;
        }
        f.pad(buf.as_str()?)
    }
}

/// The shape [`ObisCode`] accepts, as rendered in a [`ParseError`].
const OBIS_FORMAT: &str = "A-B:C.D.E*F, e.g. 1-0:1.8.0 (the *F suffix is optional)";

impl FromStr for ObisCode {
    type Err = ParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        parse_obis(s)
            .ok_or_else(|| ParseError::format(ParseErrorKind::Charset, "ObisCode", s, OBIS_FORMAT))
    }
}

impl TryFrom<&str> for ObisCode {
    type Error = ParseError;
    fn try_from(s: &str) -> Result<Self, Self::Error> {
        s.parse()
    }
}

/// Both spellings of `A-B:C.D.E*F`, plus leading zeros and surrounding
/// whitespace.
fn parse_obis(s: &str) -> Option<ObisCode> {
    let (a_str, rest) = s.trim().split_once('-')?;
    let (b_str, cd_ef) = rest.split_once(':')?;

    let (cde, f_str) = match cd_ef.split_once('*') {
        Some((l, r)) => (l, Some(r)),
        None => (cd_ef, None),
    };
    let (c_str, d_e) = cde.split_once('.')?;
    let (d_str, e_str) = d_e.split_once('.')?;

    Some(ObisCode {
        a: group(a_str)?,
        b: group(b_str)?,
        c: group(c_str)?,
        d: group(d_str)?,
        e: group(e_str)?,
        f: match f_str {
            Some(t) => group(t)?,
            None => ObisCode::STORAGE_UNUSED,
        },
    })
}

/// One value group: ASCII digits only (`u8::from_str` alone would accept a
/// `+` sign).
fn group(s: &str) -> Option<u8> {
    if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    s.parse().ok()
}

// ── serde ─────────────────────────────────────────────────────────────────────

#[cfg(feature = "serde")]
mod serde_impl {
    use super::ObisCode;
    use serde::de::{self, Visitor};
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use std::fmt;

    impl Serialize for ObisCode {
        /// Writes the canonical [`Display`](fmt::Display) form.
        fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
            serializer.collect_str(self)
        }
    }

    struct ObisCodeVisitor;

    impl Visitor<'_> for ObisCodeVisitor {
        type Value = ObisCode;

        fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("an OBIS code string such as \"1-0:1.8.0\"")
        }

        fn visit_str<E: de::Error>(self, v: &str) -> Result<ObisCode, E> {
            v.parse().map_err(de::Error::custom)
        }
    }

    impl<'de> Deserialize<'de> for ObisCode {
        /// Accepts either spelling. A visitor rather than a `&str` bridge, so
        /// non-borrowing deserialisers (`serde_json::from_reader`) work too.
        fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
            deserializer.deserialize_str(ObisCodeVisitor)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_standard_strom_bezug() {
        let code: ObisCode = "1-0:1.8.0*255".parse().unwrap();
        assert_eq!(code, ObisCode::STROM_BEZUG_TOTAL);
        assert!(code.is_electricity());
        assert!(code.is_import(), "1-0:1.8.0 C=1,D=8 should be import");
        assert!(code.is_total_register());
        assert!(!code.is_export());
    }

    #[test]
    fn parse_ht_register() {
        let code: ObisCode = "1-0:1.8.1*255".parse().unwrap();
        assert_eq!(code, ObisCode::STROM_BEZUG_HT);
        assert_eq!(code.tariff(), Some(1));

        assert_eq!(code.tariff(), Some(1));
    }

    #[test]
    fn parse_without_storage_number() {
        let code: ObisCode = "1-0:2.8.0".parse().unwrap();
        assert_eq!(code.f, 255);
        assert_eq!(code.c, 2);
        assert_eq!(code.d, 8);
        assert!(
            code.is_export(),
            "1-0:2.8.0 should be Einspeisung/export (C=2, D=8)"
        );
        assert_eq!(code, ObisCode::STROM_EINSPEISUNG_TOTAL);
    }

    #[test]
    fn parse_gas_volume() {
        let code: ObisCode = "7-0:3.0.0*255".parse().unwrap();
        assert_eq!(code, ObisCode::GAS_VOLUME_M3);
        assert!(code.is_gas());
        assert!(!code.is_electricity());
    }

    #[test]
    fn invalid_code_returns_error() {
        assert!("not-an-obis".parse::<ObisCode>().is_err());
        assert!("1-0:1.8".parse::<ObisCode>().is_err());
        assert!("".parse::<ObisCode>().is_err());
    }

    #[test]
    fn constants_are_correct() {
        assert_eq!(ObisCode::STROM_BEZUG_TOTAL.to_string(), "1-0:1.8.0");
        assert_eq!(ObisCode::STROM_BEZUG_HT.to_string(), "1-0:1.8.1");
        assert_eq!(ObisCode::STROM_BEZUG_NT.to_string(), "1-0:1.8.2");
        assert_eq!(ObisCode::STROM_EINSPEISUNG_TOTAL.to_string(), "1-0:2.8.0");
        assert_eq!(ObisCode::GAS_VOLUME_M3.to_string(), "7-0:3.0.0");
    }
}

/// Value-group semantics per EDI@Energy Codeliste v2.5c (§2.1, §2.2, §2.3,
/// §3.1).
#[cfg(test)]
mod mako_semantics_tests {
    use super::*;

    #[test]
    fn direction_is_group_c_across_every_messart() {
        for (code, what) in [
            ("1-0:1.8.0", "Zählerstand"),
            ("1-0:1.9.0", "Vorschub"),
            ("1-0:1.29.0", "Lastgang"),
            ("1-0:1.6.0", "Maximum"),
            ("1-0:1.8.1", "Zählerstand HT"),
        ] {
            let c: ObisCode = code.parse().unwrap();
            assert!(c.is_import(), "{code} ({what}) is Bezug");
            assert!(!c.is_export(), "{code} ({what}) is not Lieferung");
        }

        for (code, what) in [
            ("1-0:2.8.0", "Zählerstand"),
            ("1-0:2.9.0", "Vorschub"),
            ("1-0:2.29.0", "Lastgang"),
            ("1-0:2.6.0", "Maximum"),
        ] {
            let c: ObisCode = code.parse().unwrap();
            assert!(c.is_export(), "{code} ({what}) is Lieferung");
            assert_eq!(
                c.direction(),
                Some(crate::series::interval::Direction::Export)
            );
            assert!(!c.is_import(), "{code} ({what}) is not Bezug");
        }
    }

    /// `1-0:3.9.0` is Blindarbeit positiv, Vorschub — not an export register.
    #[test]
    fn messart_is_not_a_direction() {
        let vorschub: ObisCode = "1-0:1.9.0".parse().unwrap();
        assert!(vorschub.is_vorschub() && vorschub.is_import());

        let blind_vorschub: ObisCode = "1-0:3.9.0".parse().unwrap();
        assert!(blind_vorschub.is_vorschub());
        assert!(blind_vorschub.is_reactive());
        assert!(
            !blind_vorschub.is_export(),
            "D = 9 is Vorschub (Zeitintegral 2), not a reverse-direction flag"
        );
    }

    #[test]
    fn lastgang_and_maximum_are_different_registers() {
        let lastgang = ObisCode::STROM_BEZUG_LASTGANG;
        let maximum = ObisCode::STROM_BEZUG_MAXIMUM;

        assert_eq!(lastgang.to_string(), "1-0:1.29.0");
        assert_eq!(maximum.to_string(), "1-0:1.6.0");

        assert!(lastgang.is_lastgang() && !lastgang.is_maximum());
        assert!(maximum.is_maximum() && !maximum.is_lastgang());
        assert_ne!(lastgang, maximum);

        use crate::time::resolution::Resolution;
        assert_eq!(
            lastgang.default_resolution(),
            Some(Resolution::QUARTER_HOUR)
        );
        assert_eq!(maximum.default_resolution(), None);
    }

    #[test]
    fn every_reactive_quadrant_is_reactive() {
        for c in 3..=8u8 {
            let code: ObisCode = format!("1-0:{c}.8.0").parse().unwrap();
            assert!(code.is_reactive(), "1-0:{c}.8.0 is Blindarbeit");
            assert!(!code.is_import(), "Blindarbeit is not Wirkarbeit Bezug");
        }
        for named in [
            ObisCode::STROM_BLINDARBEIT_POSITIV,
            ObisCode::STROM_BLINDARBEIT_NEGATIV,
            ObisCode::STROM_BLINDARBEIT_Q1,
            ObisCode::STROM_BLINDARBEIT_Q2,
            ObisCode::STROM_BLINDARBEIT_Q3,
            ObisCode::STROM_BLINDARBEIT_Q4,
        ] {
            assert!(named.is_reactive(), "{named}");
        }
        assert!(!ObisCode::STROM_BEZUG_TOTAL.is_reactive());
        assert!(!ObisCode::STROM_EINSPEISUNG_TOTAL.is_reactive());
    }

    /// C = 3 means Blindleistung only for A = 1; `7-0:3.0.0` is gas volume.
    #[test]
    fn the_c_group_is_scoped_to_the_medium() {
        assert!(
            !ObisCode::GAS_VOLUME_M3.is_reactive(),
            "7-0:3.0.0 is a gas volume; C = 3 only means Blindleistung for A = 1"
        );
        assert!(ObisCode::GAS_VOLUME_M3.is_gas());

        assert!(!ObisCode::GAS_VOLUME_M3.is_import());
        assert!(!ObisCode::WAERME_ENERGY.is_import());
    }

    #[test]
    fn fehlerregister_is_not_a_tariff() {
        let fehler: ObisCode = "1-0:1.8.63".parse().unwrap();
        assert!(fehler.is_fehlerregister());
        assert_eq!(fehler.tariff(), None);
        assert!(
            !fehler.is_total_register(),
            "E = 63 is not the total either"
        );
        assert_eq!(
            fehler.default_resolution(),
            None,
            "a fault counter is not a time series"
        );

        assert_eq!(ObisCode::STROM_BEZUG_HT.tariff(), Some(1));
        assert_eq!(ObisCode::STROM_BEZUG_NT.tariff(), Some(2));
        let t62: ObisCode = "1-0:1.8.62".parse().unwrap();
        assert_eq!(t62.tariff(), Some(62));
        assert!(!t62.is_fehlerregister());
        assert_eq!(ObisCode::STROM_BEZUG_TOTAL.tariff(), None);
    }
}

/// One channel, one string.
#[cfg(test)]
mod canonical_string_tests {
    use super::*;

    /// Canonical spellings; each must survive `parse` → `to_string` unchanged.
    const CANONICAL: &[&str] = &[
        "1-0:1.8.0",
        "1-0:1.8.1",
        "1-0:1.8.2",
        "1-0:2.8.0",
        "1-0:2.8.1",
        "1-0:2.8.2",
        "1-0:1.29.0",
        "1-0:3.8.0",
        "1-0:4.8.0",
        "1-0:12.7.0",
        "7-0:3.0.0",
        "6-0:1.0.0",
        "5-0:1.0.0",
        "8-0:1.0.0",
        "9-0:1.0.0",
        "4-0:1.0.0",
        "1-0:1.8.0*1",
        "1-0:1.8.0*0",
    ];

    #[test]
    fn canonical_strings_survive_a_round_trip() {
        for s in CANONICAL {
            let code: ObisCode = s.parse().unwrap_or_else(|e| panic!("{s}: {e}"));
            assert_eq!(&code.to_string(), s, "{s} is not string-stable");
        }
    }

    #[test]
    fn both_spellings_are_one_value_and_one_string() {
        let typed: ObisCode = "1-0:1.8.0".parse().unwrap();
        let emitted: ObisCode = "1-0:1.8.0*255".parse().unwrap();

        assert_eq!(typed, emitted, "same channel, same value");
        assert_eq!(
            typed.to_string(),
            emitted.to_string(),
            "same channel, same key"
        );
        assert_eq!(typed.to_string(), "1-0:1.8.0");
    }

    #[test]
    fn lenient_spellings_normalise_onto_the_canonical_one() {
        for s in [
            "1-0:1.8.0",
            "1-0:1.8.0*255",
            "  1-0:1.8.0  ",
            "01-00:01.08.00",
            "1-0:01.8.0*0255",
        ] {
            assert_eq!(
                s.parse::<ObisCode>()
                    .map(|c| c.to_string())
                    .unwrap_or_else(|e| panic!("{s}: {e}")),
                "1-0:1.8.0",
                "{s} must normalise onto the canonical spelling"
            );
        }
    }

    #[test]
    fn normalize_is_idempotent() {
        for s in CANONICAL {
            let once = s.parse::<ObisCode>().map(|c| c.to_string()).unwrap();
            assert_eq!(
                once.parse::<ObisCode>().map(|c| c.to_string()).unwrap(),
                once,
                "{s}"
            );
        }
    }

    #[test]
    fn full_form_is_available_and_parses_back() {
        let code = ObisCode::STROM_BEZUG_TOTAL;
        assert_eq!(format!("{code:#}"), "1-0:1.8.0*255");
        assert_eq!(format!("{code:#}"), "1-0:1.8.0*255");
        assert_eq!(format!("{code:#}").parse::<ObisCode>().unwrap(), code);
    }

    #[test]
    fn a_real_storage_group_is_never_elided() {
        let historical: ObisCode = "1-0:1.8.0*1".parse().unwrap();
        assert!(!historical.has_unused_storage());
        assert_eq!(historical.to_string(), "1-0:1.8.0*1");
        assert_ne!(historical, ObisCode::STROM_BEZUG_TOTAL);
        assert_ne!(
            historical.to_string(),
            ObisCode::STROM_BEZUG_TOTAL.to_string()
        );
    }

    #[test]
    fn signed_and_empty_groups_are_rejected() {
        for s in [
            "+1-0:1.8.0",
            "1-0:1.8.0*+255",
            "1--0:1.8.0",
            "1-0:1.8.",
            "1-0:.8.0",
            "1-0:1.8.0*",
            "1-0:1.8.0.5",
            "1 - 0 : 1.8.0",
            "1-0:1.8.0*256",
            "256-0:1.8.0",
        ] {
            assert!(s.parse::<ObisCode>().is_err(), "{s:?} must not parse");
        }
    }

    #[test]
    fn ordering_is_by_value_group() {
        let mut codes = [
            ObisCode::GAS_VOLUME_M3,
            ObisCode::STROM_BEZUG_NT,
            ObisCode::STROM_BEZUG_TOTAL,
            ObisCode::STROM_BEZUG_HT,
        ];
        codes.sort();
        assert_eq!(
            codes,
            [
                ObisCode::STROM_BEZUG_TOTAL,
                ObisCode::STROM_BEZUG_HT,
                ObisCode::STROM_BEZUG_NT,
                ObisCode::GAS_VOLUME_M3,
            ]
        );
    }
}

#[cfg(test)]
mod media_group_tests {
    use super::*;

    #[test]
    fn medium_group_a_follows_the_dlms_media_list() {
        let hca: ObisCode = "4-0:1.0.0".parse().unwrap();
        let cooling: ObisCode = "5-0:1.0.0".parse().unwrap();
        let heat: ObisCode = "6-0:1.0.0".parse().unwrap();
        let gas: ObisCode = "7-0:3.0.0".parse().unwrap();
        let cold_water: ObisCode = "8-0:1.0.0".parse().unwrap();
        let hot_water: ObisCode = "9-0:1.0.0".parse().unwrap();

        assert!(hca.is_heat_cost_allocator());
        assert!(
            !hca.is_heat(),
            "an HCA measures Verbrauchseinheiten, not kWh"
        );

        assert!(cooling.is_heat() && heat.is_heat());
        assert!(gas.is_gas() && !gas.is_heat());

        assert!(cold_water.is_water() && hot_water.is_water());
        assert!(!cold_water.is_heat(), "A=8 is cold water");
    }

    /// Every named constant satisfies the predicate its name implies.
    #[test]
    fn named_constants_match_their_medium_predicate() {
        assert!(ObisCode::WAERME_ENERGY.is_heat(), "heat is medium 6");
        assert!(!ObisCode::WAERME_ENERGY.is_water());
        assert_eq!(ObisCode::WAERME_ENERGY.to_string(), "6-0:1.0.0");

        assert!(ObisCode::KAELTE_ENERGY.is_heat(), "cooling is medium 5");

        assert!(ObisCode::WASSER_KALT_VOLUME.is_water());
        assert!(!ObisCode::WASSER_KALT_VOLUME.is_heat());
        assert!(ObisCode::WASSER_WARM_VOLUME.is_water());

        assert!(ObisCode::GAS_VOLUME_M3.is_gas());
        assert!(ObisCode::STROM_BEZUG_TOTAL.is_electricity());

        use crate::time::resolution::Resolution;
        assert_eq!(
            ObisCode::WAERME_ENERGY.default_resolution(),
            Some(Resolution::Hour),
            "a heat meter is read hourly, not daily like a water submeter"
        );
        assert_eq!(
            ObisCode::WASSER_KALT_VOLUME.default_resolution(),
            Some(Resolution::Day)
        );
    }

    #[test]
    fn water_defaults_to_daily_resolution() {
        use crate::time::resolution::Resolution;
        let cold_water: ObisCode = "8-0:1.0.0".parse().unwrap();
        let heat: ObisCode = "6-0:1.0.0".parse().unwrap();
        assert_eq!(cold_water.default_resolution(), Some(Resolution::Day));
        assert_eq!(heat.default_resolution(), Some(Resolution::Hour));
    }
}

#[cfg(test)]
mod default_resolution_tests {
    use super::*;
    use crate::time::resolution::Resolution as R;

    fn code(s: &str) -> ObisCode {
        s.parse().unwrap_or_else(|_| panic!("{s} must parse"))
    }

    /// A channel that is not an equidistant series has no resolution, active
    /// or reactive.
    #[test]
    fn a_register_that_is_not_a_series_has_no_resolution() {
        for s in [
            "1-0:1.6.0",  // Wirkleistung Maximum
            "1-0:5.6.0",  // Blindleistung Q I Maximum
            "1-0:1.9.0",  // Vorschub — an arbitrary period
            "1-0:3.9.0",  // Blindarbeit Vorschub
            "1-0:1.7.0",  // Momentanwert
            "1-0:1.8.63", // Fehlerregister
            "4-0:1.0.0",  // Heizkostenverteiler — Verbrauchseinheiten
            "0-0:96.1.0", // an abstract code
        ] {
            assert_eq!(code(s).default_resolution(), None, "{s}");
        }
    }

    /// § 2 Satz 1 Nr. 27 MsbG: quarter-hourly for electricity, hourly for gas.
    #[test]
    fn the_series_channels_carry_the_msbg_cadences() {
        for s in ["1-0:1.8.0", "1-0:2.8.2", "1-0:1.29.0", "1-0:8.8.0"] {
            assert_eq!(code(s).default_resolution(), Some(R::QUARTER_HOUR), "{s}");
        }
        for s in ["7-0:3.0.0", "7-0:13.2.0"] {
            assert_eq!(code(s).default_resolution(), Some(R::Hour), "{s}");
        }
        assert_eq!(
            ObisCode::WAERME_ENERGY.default_resolution(),
            Some(R::Hour),
            "a heat meter delivers hourly"
        );
        assert_eq!(
            ObisCode::WASSER_KALT_VOLUME.default_resolution(),
            Some(R::Day),
            "water submetering is read daily at best"
        );
    }

    /// For the gas conversion parameters, value group E is the averaging period.
    #[test]
    fn a_gas_mittelwert_takes_its_period_from_value_group_e() {
        assert_eq!(code("7-0:54.0.16").default_resolution(), Some(R::Hour));
        assert_eq!(code("7-0:54.0.20").default_resolution(), Some(R::Day));
        assert_eq!(
            ObisCode::GAS_BRENNWERT_MONATSMITTEL.default_resolution(),
            Some(R::Month),
        );
        assert_eq!(
            ObisCode::GAS_ZUSTANDSZAHL.default_resolution(),
            Some(R::Month),
        );
        assert_eq!(code("7-0:54.0.5").default_resolution(), None);
    }

    /// A calendar resolution never claims a fixed length.
    #[test]
    fn a_calendar_answer_never_claims_a_fixed_length() {
        for a in [0u8, 1, 4, 5, 6, 7, 8, 9] {
            for c in [1u8, 2, 3, 5, 8, 13, 52, 54, 99] {
                for d in [0u8, 2, 6, 7, 8, 9, 29] {
                    for e in [0u8, 1, 16, 20, 22, 63] {
                        let code = ObisCode {
                            a,
                            b: 0,
                            c,
                            d,
                            e,
                            f: ObisCode::STORAGE_UNUSED,
                        };
                        if let Some(r) = code.default_resolution() {
                            assert_eq!(
                                r.is_calendar(),
                                matches!(r, R::Day | R::Month | R::Year),
                                "{code}"
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn kind_unit_tariff_and_grid_agree_everywhere() {
        for a in [0u8, 1, 4, 5, 6, 7, 8, 9] {
            for c in [0u8, 1, 2, 3, 5, 8, 13, 52, 54, 99] {
                for d in [0u8, 2, 6, 7, 8, 9, 29] {
                    for e in [0u8, 1, 2, 16, 22, 63] {
                        let code = ObisCode {
                            a,
                            b: 0,
                            c,
                            d,
                            e,
                            f: ObisCode::STORAGE_UNUSED,
                        };
                        let unit = code.unit();
                        if code.tariff().is_some() {
                            assert!(unit.is_some_and(Unit::is_energy), "{code}");
                        }
                        if matches!(code.kind(), Messart::Maximum | Messart::Momentan) {
                            assert!(
                                matches!(unit, None | Some(Unit::KiloWatt | Unit::KiloVar)),
                                "{code}: a power register counts kW/kvar"
                            );
                            assert_eq!(code.default_resolution(), None, "{code}");
                        }
                        if code.is_fehlerregister() {
                            assert_eq!(code.tariff(), None);
                            assert_eq!(code.default_resolution(), None);
                        }
                    }
                }
            }
        }
        assert_eq!(ObisCode::GAS_ZUSTANDSZAHL.unit(), Some(Unit::Dimensionless));
        assert_eq!(
            ObisCode::GAS_BRENNWERT_MONATSMITTEL.unit(),
            Some(Unit::KiloWattHourPerCubicMetre)
        );
        assert_eq!(ObisCode::GAS_BRENNWERT_MONATSMITTEL.tariff(), None);
        assert_eq!(ObisCode::STROM_BEZUG_MAXIMUM.unit(), Some(Unit::KiloWatt));
    }
}
