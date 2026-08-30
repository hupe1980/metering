//! Typed OBIS (Object Identification System) codes for German energy metering.
//!
//! ## Standard
//!
//! IEC 62056-21 / DLMS-COSEM, as adopted by BDEW and Messstellenbetreiber
//! in Germany. OBIS codes identify measurement channels on smart meters and
//! data communication systems.
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
//! C, D and E are the groups that get misread most often, so they have
//! predicates rather than raw comparisons: **direction is C alone** (1 = Bezug,
//! 2 = Lieferung — EDI@Energy §2.1 writes them as `1-b:1.x.y` and `1-b:2.x.y`,
//! with `x`/`y` free), **D is a Messart, never a direction**, and **E = 63 is
//! the Fehlerregister, not tariff 63**.
//!
//! ## One code, one string
//!
//! `1-0:1.8.0` and `1-0:1.8.0*255` denote the same channel, and both occur in
//! the wild — MSCONS and hand-typed input carry the short form, meter firmware
//! often the long one. Two spellings for one channel is how a stored key ends up
//! disagreeing with itself, so this type defines exactly one canonical string:
//!
//! - **[`Display`] writes the reduced form** — `*F` omitted when F is 255.
//!   `1-0:1.8.0`.
//! - **`{:#}` writes the full six-group form** when a downstream system demands
//!   an explicit F. `1-0:1.8.0*255`.
//! - **[`FromStr`] accepts both**, plus leading zeros and surrounding
//!   whitespace, and maps them onto the same value.
//!
//! The reduced form is not a preference: the EDI@Energy *Codeliste der
//! OBIS-Kennzahlen und Medien* v2.5c (01.10.2025, binding from 01.04.2026)
//! marks both the electricity and the thermal-energy value-group diagram
//! **"A B C D E werden im deutschen Energiemarkt verwendet"**, and §2.3 states
//! again that *"Wertegruppe F wird für die Kommunikation im deutschen Gasmarkt
//! nicht verwendet"*. Every code in that list is printed without a suffix —
//! `1-b:1.8.e`, `1-b:1.29.0`, `1-b:1.6.0`. Emitting `*255` appended a value
//! group the German market does not use.
//!
//! So `s.parse::<ObisCode>()?.to_string() == s` holds for every canonical `s`,
//! and [`ObisCode::normalize`] turns any accepted spelling into that canonical
//! `s` in one call. Anything used as a database key, a merge key or a map key
//! should go through one of the two.
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

use crate::error::ParseError;

/// A parsed OBIS code: `A-B:C.D.E*F`.
///
/// All six value groups are stored, so two spellings of one channel are one
/// value. See the [module docs](self#one-code-one-string) for the canonical
/// string form — briefly: [`Display`](fmt::Display) writes `1-0:1.8.0`, `{:#}`
/// writes `1-0:1.8.0*255`, and [`FromStr`] reads either.
///
/// Ordering is lexicographic by value group (A, then B, … then F), which is the
/// order a channel listing is conventionally sorted in.
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
    /// Messgröße, and its meaning depends on [`a`](Self::a). For electricity:
    /// 1 = ∑Li Wirkleistung **+** (Bezug), 2 = ∑Li Wirkleistung **−**
    /// (Lieferung), 3 = Blindleistung positiv, 4 = negativ, 5–8 = Q I–Q IV.
    /// **Direction lives here**, not in [`d`](Self::d).
    ///
    /// For gas it instead encodes Messgröße, Quelle, Richtung and
    /// Qualifikation together — 3 = Betriebsvolumen Ausspeisung gesamt,
    /// 13 = Normvolumen umgewertet, 52 = Zustandszahl, 54 = Brennwert.
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
    /// identify stored previous readings on meters that keep them.
    ///
    /// The German market does not use this group at all — see the
    /// [module docs](self#one-code-one-string) — so it is 255 in practice and
    /// [`Display`](fmt::Display) omits it.
    ///
    /// [`STORAGE_UNUSED`]: Self::STORAGE_UNUSED
    pub f: u8,
}

impl ObisCode {
    /// Value group F when the storage / billing-period group does not apply.
    ///
    /// IEC 62056-6-1 reserves 255 for "not used", and the reduced OBIS form
    /// omits the group entirely — which is what [`Display`](fmt::Display) does.
    pub const STORAGE_UNUSED: u8 = 255;

    /// The longest string any [`ObisCode`] can render as —
    /// `255-255:255.255.255*255`, 23 bytes.
    ///
    /// Every code is ASCII, so this is a byte length and a character count
    /// alike. Use it to size a fixed-width database column: `VARCHAR(23)` holds
    /// any code in either spelling, and the canonical form is at most 19.
    pub const MAX_LEN: usize = 23;

    /// Electricity forward active energy — total (Bezug, kWh, gesamt).
    /// HT + NT combined.
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
    /// This is the channel a [`MeterInterval`](crate::MeterInterval) usually
    /// carries: MSCONS PID 13018 / 13025 transmit it. D = 29 is *Zeitintegral 5*
    /// in the EDI@Energy Codeliste — a load profile — **not** a maximum. The
    /// peak-demand register is [`STROM_BEZUG_MAXIMUM`] (D = 6), and a demand in
    /// kW is derived from this channel by
    /// [`MeterInterval::demand_kw`](crate::MeterInterval::demand_kw).
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
    /// D = 6 is *Maximum* in the EDI@Energy Codeliste.
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
    /// A Vorschub is the energy consumed over an arbitrary period — the
    /// difference between two Zählerstände. D = 9 is *Zeitintegral 2*; it is a
    /// Messart, **not** a direction. MSCONS PID 13019 carries it.
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
    /// The EDI@Energy Codeliste names C = 3 "∑ Li Blindleistung positiv". The
    /// inductive/capacitive distinction is a property of the *quadrant*
    /// ([`STROM_BLINDARBEIT_Q1`] … [`STROM_BLINDARBEIT_Q4`]), not of the sign,
    /// so this constant is named for what the standard measures.
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
    /// Volume at metering conditions, *before* the Zustandszahl is applied. This
    /// is the input [`gas_m3_to_kwh_hs`](crate::gas_m3_to_kwh_hs) expects.
    ///
    /// **Not to be confused with a Normvolumen.** `7-0:13.2.0`
    /// ([`GAS_NORMVOLUMEN_UMGEWERTET`]) is already state-converted; multiplying
    /// it by a Zustandszahl applies the correction twice and overstates the
    /// energy by roughly the Zustandszahl's deviation from 1 — a few percent on
    /// a billed quantity, with no error raised.
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
    /// Already converted to standard conditions by the Mengenumwerter, so it
    /// needs the Brennwert but **not** the Zustandszahl. See
    /// [`GAS_VOLUME_M3`](Self::GAS_VOLUME_M3).
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
    /// value-group-A list that OMS Spec Vol. 2 adopts.
    ///
    /// Medium 8 is *cold water*, not heat: see [`WASSER_KALT_VOLUME`]. Using
    /// A = 8 for a Wärmezähler puts the reading in the wrong Sparte, gives it
    /// water's daily default resolution instead of hourly, and makes
    /// [`is_heat`](Self::is_heat) report `false` for a heat register.
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
    /// Billed as a volume; the heat share it apportions out of the building's
    /// heating bill is [`crate::warm_water_heat_kwh`] (HeizkostenV §9 Abs. 2).
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
    ///
    /// Media are numbered per the DLMS/COSEM Blue Book value-group-A list, which
    /// OMS Spec Vol. 2 adopts.
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
    /// HCAs report dimensionless *Verbrauchseinheiten*, not a physical quantity.
    ///
    /// They carry **no Eichfrist**, because they are not Messgeräte under
    /// MessEG at all — "Heizkostenverteiler" appears nowhere in MessEV, neither
    /// in Anlage 1 nor in the Eichfristen of Anlage 7. HeizkostenV §5 Abs. 1
    /// admits them through an explicitly conditional clause — *"soweit nicht
    /// eichrechtliche Bestimmungen zur Anwendung kommen"* — requiring instead
    /// that an expert body confirm conformity with the anerkannte Regeln der
    /// Technik (EN 834 for evaporation, EN 835 for electronic HCAs).
    ///
    /// So an Eichfrist check must **skip** these devices rather than treat a
    /// missing date as an expiry.
    #[must_use]
    pub fn is_heat_cost_allocator(&self) -> bool {
        self.a == 4
    }

    /// `true` when this code measures reactive energy or power (Blindarbeit /
    /// Blindleistung) — electricity C = 3…8.
    ///
    /// The EDI@Energy Codeliste assigns C = 3 to Blindleistung positiv, C = 4 to
    /// negativ, and **C = 5…8 to the four quadrants Q I…Q IV**. Checking only
    /// 3 and 4 reports a quadrant register as active energy, which puts a kvarh
    /// value into a kWh column.
    ///
    /// Reactive energy is required for Blindstromberechnung in industrial
    /// tariffs, for SMGW power quality monitoring (BSI TR-03109) and for RLM
    /// power quality records.
    #[must_use]
    pub fn is_reactive(&self) -> bool {
        self.a == 1 && matches!(self.c, 3..=8)
    }

    /// `true` when this code is a **Lastgang** — energy per equidistant interval
    /// (D = 29, *Zeitintegral 5*).
    ///
    /// This is what a [`MeterInterval`](crate::MeterInterval) carries. It is an
    /// energy quantity in kWh, **not** a power in kW: see
    /// [`is_maximum`](Self::is_maximum) for the peak-demand register.
    #[must_use]
    pub fn is_lastgang(&self) -> bool {
        self.d == 29
    }

    /// `true` when this code is a **maximum** register (D = 6).
    ///
    /// `1-0:1.6.0` is the Jahreshöchstleistung priced under § 17 Abs. 2 StromNEV.
    /// D = 29 is the load profile it is derived from, not the maximum itself —
    /// conflating the two bills a 15-minute energy quantity as a power.
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

    /// `true` when this code represents import (Bezug from the grid).
    ///
    /// Direction lives in value group **C alone**: EDI@Energy §2.1 states
    /// "+ Bezug des Kunden aus dem Netz (z. B. `1-b:1.x.y`)", with `x` and `y`
    /// explicitly free. So the Zählerstand `1-0:1.8.0`, the Vorschub
    /// `1-0:1.9.0`, the Lastgang `1-0:1.29.0` and the Maximum `1-0:1.6.0` are
    /// all import.
    ///
    /// Requiring D = 8 here would report the Lastgang — the commonest code in
    /// MSCONS interval data — as neither import nor export.
    #[must_use]
    pub fn is_import(&self) -> bool {
        self.a == 1 && self.c == 1
    }

    /// `true` when this code represents export / Einspeisung (Rücklieferung to
    /// the grid).
    ///
    /// EDI@Energy §2.1: "− (Rück-)Lieferung des Kunden an das Netz (z. B.
    /// `1-b:2.x.y`)". As with [`is_import`](Self::is_import), the Messart D is
    /// not part of the direction — `D = 9` is *Vorschub*, not a reverse flag.
    #[must_use]
    pub fn is_export(&self) -> bool {
        self.a == 1 && self.c == 2
    }

    /// `true` when this code represents Einspeisung (feed-in to the grid).
    ///
    /// Alias for `is_export()` using the German market terminology.
    /// Identifies `1-0:2.8.x` codes (reverse active energy).
    #[must_use]
    pub fn is_einspeisung(&self) -> bool {
        self.is_export()
    }

    /// Value group E when the register counts faults rather than energy.
    ///
    /// EDI@Energy Codeliste §2.2 lists "63 Fehlerregister" alongside the
    /// tariffs, so 63 is not a tariff number and its contents are not a billable
    /// quantity.
    pub const TARIFF_FEHLERREGISTER: u8 = 63;

    /// Tariff register: `None` = total/combined (E = 0), `Some(1)` = HT,
    /// `Some(2)` = NT, up to `Some(62)`.
    ///
    /// Returns `None` for the **Fehlerregister** (E = 63) as well, because it is
    /// not a tariff — reporting it as `Some(63)` invites a caller to bill a
    /// fault counter as tariff 63's consumption. Distinguish the two cases with
    /// [`is_total_register`](Self::is_total_register) or
    /// [`is_fehlerregister`](Self::is_fehlerregister).
    ///
    /// Tariffs ran 0…9 until 2023-10-01 and 0…62 since.
    #[must_use]
    pub fn tariff_register(&self) -> Option<u8> {
        match self.e {
            0 | Self::TARIFF_FEHLERREGISTER => None,
            n => Some(n),
        }
    }

    /// `true` when this is the Fehlerregister (E = 63) rather than a tariff.
    ///
    /// A Fehlerregister counts fault occurrences. Its value must never be
    /// summed into an Arbeitsmenge.
    #[must_use]
    pub const fn is_fehlerregister(&self) -> bool {
        self.e == Self::TARIFF_FEHLERREGISTER
    }

    /// `true` when this is the total / combined register (E = 0).
    #[must_use]
    pub fn is_total_register(&self) -> bool {
        self.e == 0
    }

    /// `true` when this is the HT (Hochtarif) register (E = 1).
    #[must_use]
    pub fn is_ht(&self) -> bool {
        self.e == 1
    }

    /// `true` when this is the NT (Niedertarif) register (E = 2).
    #[must_use]
    pub fn is_nt(&self) -> bool {
        self.e == 2
    }

    /// The grid a series on this channel is ordinarily delivered on.
    ///
    /// A *default*, not a guarantee: it is the expectation an ingest path
    /// starts from when the message carries no explicit resolution. What the
    /// series actually holds is
    /// [`detect_interval_length`](crate::detect_interval_length).
    ///
    /// `None` where the channel is **not an equidistant series at all**, which
    /// is a different answer from "some resolution we cannot name" and the
    /// reason this returns an `Option`:
    ///
    /// | Channel | Answer | Why |
    /// |---|---|---|
    /// | Fehlerregister, E = 63 | `None` | counts faults, not energy over time |
    /// | Maximum, electricity D = 6 | `None` | one figure per billing period |
    /// | Vorschub, electricity D = 9 | `None` | an arbitrary period, by definition off-grid |
    /// | Momentanwert, electricity D = 7 | `None` | an instant, not an interval |
    /// | electricity Zählerstand / Lastgang, C = 1…8 | `PT15M` | § 2 Satz 1 Nr. 27 MsbG |
    /// | gas Zustandszahl / Brennwert, C = 52 / 54 | from value group E | 16 hourly, 20 daily, 22 monthly |
    /// | other gas | `PT1H` | § 2 Satz 1 Nr. 27 MsbG: *stündlich* for gas |
    /// | heat, cooling (A = 5, 6) | `PT1H` | |
    /// | cold, hot water (A = 8, 9) | `P1D` | submetering is read daily at best |
    ///
    /// The Messart exclusions apply on the **electricity** axis only. Value
    /// group C carries a different vocabulary for gas — Messgröße, Quelle,
    /// Richtung and Qualifikation at once — and D is not a Messart there.
    ///
    /// ```rust
    /// use metering::{IntervalResolution, ObisCode};
    ///
    /// assert_eq!(
    ///     ObisCode::STROM_BEZUG_LASTGANG.default_resolution(),
    ///     Some(IntervalResolution::QuarterHour),
    /// );
    /// // A Brennwert published as a monthly mean is not an hourly series.
    /// assert_eq!(
    ///     ObisCode::GAS_BRENNWERT_MONATSMITTEL.default_resolution(),
    ///     Some(IntervalResolution::Month),
    /// );
    /// // A maximum register is one number per period, whether active…
    /// assert_eq!(ObisCode::STROM_BEZUG_MAXIMUM.default_resolution(), None);
    /// // …or reactive.
    /// assert_eq!("1-0:5.6.0".parse::<ObisCode>()?.default_resolution(), None);
    /// # Ok::<(), metering::ParseError>(())
    /// ```
    #[must_use]
    pub const fn default_resolution(&self) -> Option<crate::resolution::IntervalResolution> {
        use crate::resolution::IntervalResolution as R;
        // A fault counter is not a time series at any resolution.
        if self.is_fehlerregister() {
            return None;
        }
        match self.a {
            // Electricity: the Messart says whether this is a series at all,
            // and the Messgröße whether it is a quantity this crate names.
            1 => match (self.c, self.d) {
                // Zählerstand (Zeitintegral 1) and Lastgang (Zeitintegral 5),
                // active and reactive alike — the quarter-hour grid of
                // § 2 Satz 1 Nr. 27 MsbG.
                (1..=8, 8 | 29) => Some(R::QuarterHour),
                // Maximum (6), Vorschub (9), Momentanwert (7) and anything
                // else: not an equidistant series.
                _ => None,
            },
            // Cooling and heat meters deliver hourly.
            5 | 6 => Some(R::Hour),
            // Gas: value group E names the averaging period of the two
            // conversion parameters the Codeliste publishes as Mittelwerte.
            // Treating `7-0:54.0.22` as an hourly channel called a monthly
            // Brennwert an hourly one.
            7 => match (self.c, self.e) {
                (52 | 54, 16) => Some(R::Hour),
                (52 | 54, 20) => Some(R::Day),
                (52 | 54, 22) => Some(R::Month),
                (52 | 54, _) => None,
                // Volumes and energies: *stündlich ermittelte Zählerstände von
                // Gasmengen*. An SLP delivery point is allocated daily, but
                // that is a settlement grid the caller chooses, not a property
                // of the channel.
                _ => Some(R::Hour),
            },
            // Water — cold (8) and hot (9). Submetering is read daily at best.
            8 | 9 => Some(R::Day),
            // Abstract (0), Heizkostenverteiler (4) and everything unmodelled.
            _ => None,
        }
    }

    /// The physical unit this register counts in, derived from the value
    /// groups.
    ///
    /// A stored `unit` field would be a second source of truth that can
    /// contradict the code — a register tagged `KWh` with code `1-0:3.8.0` is
    /// kvarh however the field is set — so the unit is derived rather than
    /// carried.
    ///
    /// `None` for a code whose unit this crate cannot name: an abstract code
    /// (medium 0), a Heizkostenverteiler (medium 4, which counts dimensionless
    /// Verbrauchseinheiten), or an electricity Messgröße outside the active and
    /// reactive groups.
    ///
    /// ```rust
    /// use metering::obis::{ObisCode, RegisterUnit};
    ///
    /// assert_eq!(ObisCode::STROM_BEZUG_TOTAL.register_unit(),      Some(RegisterUnit::KiloWattHour));
    /// assert_eq!(ObisCode::STROM_BLINDARBEIT_Q1.register_unit(),   Some(RegisterUnit::KiloVarHour));
    /// assert_eq!(ObisCode::STROM_BEZUG_MAXIMUM.register_unit(),    Some(RegisterUnit::KiloWatt));
    /// assert_eq!(ObisCode::GAS_VOLUME_M3.register_unit(),          Some(RegisterUnit::CubicMetre));
    /// assert_eq!(ObisCode::WAERME_ENERGY.register_unit(),          Some(RegisterUnit::KiloWattHourThermal));
    /// // A Heizkostenverteiler counts units, not a physical quantity.
    /// assert_eq!("4-0:1.0.0".parse::<ObisCode>().unwrap().register_unit(), None);
    /// ```
    #[must_use]
    pub fn register_unit(&self) -> Option<RegisterUnit> {
        match self.a {
            // Electricity: the Messgröße picks active or reactive, the Messart
            // picks energy or power.
            1 => {
                let reactive = self.is_reactive();
                let active = matches!(self.c, 1 | 2);
                if !reactive && !active {
                    return None;
                }
                Some(match (reactive, self.is_maximum()) {
                    (false, false) => RegisterUnit::KiloWattHour,
                    (false, true) => RegisterUnit::KiloWatt,
                    (true, false) => RegisterUnit::KiloVarHour,
                    (true, true) => RegisterUnit::KiloVar,
                })
            }
            // Cooling and heat meters register thermal energy.
            5 | 6 => Some(RegisterUnit::KiloWattHourThermal),
            // Gas and water registers count a volume.
            7..=9 => Some(RegisterUnit::CubicMetre),
            _ => None,
        }
    }

    /// The **Lastgang** channel of this register — the same measurement as a
    /// series of energies per equidistant interval (D = 29).
    ///
    /// This is the relabelling a Zählerstandsgang → Lastgang conversion owes
    /// its output. `1-0:1.8.0` differenced into intervals is `1-0:1.29.0`, and
    /// carrying the Zählerstand code through instead makes every downstream
    /// register projection treat interval energies as cumulative readings.
    ///
    /// `None` off the electricity axis, and for a register the market defines
    /// no Lastgang for:
    ///
    /// - **Not medium 1.** For gas, value group C encodes Messgröße, Quelle,
    ///   Richtung *and* Qualifikation together and a profile is C = 99, so
    ///   D = 29 means nothing there — a distinction a medium-blind caller gets
    ///   wrong.
    /// - **Not an active or reactive Messgröße** (C = 1…8).
    /// - **Not the total register.** The Codeliste v2.5c §3.1 lists the
    ///   Zählerstand as `1-b:1.8.e` — tariff-differentiated — and the Lastgang
    ///   only as `1-b:1.29.0`. There is no tariff-differentiated Lastgang, so
    ///   an HT register has no Lastgang code and this refuses to invent one
    ///   rather than silently collapsing HT and NT onto the same channel.
    ///   Split a total Lastgang across registers with
    ///   [`Zaehlzeitdefinition`](crate::Zaehlzeitdefinition) instead.
    ///
    /// ```rust
    /// use metering::ObisCode;
    ///
    /// let bezug: ObisCode = "1-0:1.8.0".parse()?;
    /// assert_eq!(bezug.as_lastgang(), Some(ObisCode::STROM_BEZUG_LASTGANG));
    ///
    /// // Direction is preserved — the failure a hard-coded constant makes.
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
    /// The inverse of [`as_lastgang`](Self::as_lastgang), and defined for a
    /// tariff register too: the Codeliste lists `1-b:1.8.e` across the tariff
    /// range and `1-b:1.8.63` for the Fehlerregister.
    ///
    /// `None` under the same medium and Messgröße conditions as
    /// [`as_lastgang`](Self::as_lastgang).
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
    /// This is the honest label for what
    /// [`consumption_between`](crate::reading::consumption_between) computes: a
    /// Jahresablesung differenced against the previous one is a Vorschub, not a
    /// Lastgang, because the period is not part of an equidistant grid.
    ///
    /// `None` under the same conditions as [`as_lastgang`](Self::as_lastgang),
    /// except that a Vorschub, like a Zählerstand, exists per tariff.
    #[must_use]
    pub const fn as_vorschub(&self) -> Option<Self> {
        if !self.is_convertible_messart() {
            return None;
        }
        Some(Self { d: 9, ..*self })
    }

    /// Whether the Messart conversions above are defined for this code:
    /// electricity, and an active or reactive Messgröße.
    const fn is_convertible_messart(&self) -> bool {
        self.a == 1 && matches!(self.c, 1..=8)
    }

    /// `true` when value group F is [`STORAGE_UNUSED`] — the ordinary case, and
    /// the one where [`Display`](fmt::Display) omits the `*F` suffix.
    ///
    /// [`STORAGE_UNUSED`]: Self::STORAGE_UNUSED
    #[must_use]
    pub const fn has_unused_storage(&self) -> bool {
        self.f == Self::STORAGE_UNUSED
    }

    /// Parse from a string slice (lenient: `*F`, leading zeros and surrounding
    /// whitespace are all optional).
    ///
    /// # Errors
    ///
    /// Returns a [`ParseError`] when the string does not conform to OBIS format.
    pub fn parse(s: &str) -> Result<Self, ParseError> {
        s.parse()
    }

    /// The full six-group form, with `*F` always explicit — `1-0:1.8.0*255`.
    ///
    /// Equivalent to `format!("{code:#}")`. Use it when a downstream system
    /// insists on the long spelling; [`to_string`](ToString::to_string) is the
    /// canonical form and the one to key on.
    ///
    /// ```rust
    /// # use metering::ObisCode;
    /// let code = ObisCode::STROM_BEZUG_TOTAL;
    /// assert_eq!(code.to_string(),      "1-0:1.8.0");
    /// assert_eq!(code.to_full_string(), "1-0:1.8.0*255");
    /// ```
    #[must_use]
    pub fn to_full_string(&self) -> String {
        format!("{self:#}")
    }

    /// Rewrite any accepted spelling of an OBIS code as the canonical one.
    ///
    /// For consumers holding raw strings — a database column, a CSV cell, an
    /// MSCONS segment — that need one spelling per channel without building a
    /// value first. Idempotent: normalising a normalised code returns it
    /// unchanged.
    ///
    /// ```rust
    /// # use metering::ObisCode;
    /// assert_eq!(ObisCode::normalize("1-0:1.8.0*255")?, "1-0:1.8.0");
    /// assert_eq!(ObisCode::normalize("  1-0:01.8.0 ")?, "1-0:1.8.0");
    /// assert_eq!(ObisCode::normalize("1-0:1.8.0")?,     "1-0:1.8.0");
    /// // A real billing-period register keeps its F group.
    /// assert_eq!(ObisCode::normalize("1-0:1.8.0*1")?,   "1-0:1.8.0*1");
    /// # Ok::<(), metering::ParseError>(())
    /// ```
    ///
    /// # Errors
    ///
    /// Returns a [`ParseError`] when the string does not conform to OBIS format.
    pub fn normalize(s: &str) -> Result<String, ParseError> {
        Ok(s.parse::<Self>()?.to_string())
    }
}

/// The physical unit a meter register counts in.
///
/// Finer than [`MeasurementUnit`](crate::MeasurementUnit), which is the
/// *storage* vocabulary and has exactly two values. This distinguishes energy
/// from power and active from reactive, because a register does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "SCREAMING_SNAKE_CASE"))]
pub enum RegisterUnit {
    /// Active energy, kWh.
    KiloWattHour,
    /// Reactive energy, kvarh.
    KiloVarHour,
    /// Active power, kW — a maximum register.
    KiloWatt,
    /// Reactive power, kvar — a maximum register.
    KiloVar,
    /// Volume, m³ — gas and water.
    CubicMetre,
    /// Thermal energy, kWh_th — heat and cooling.
    KiloWattHourThermal,
}

impl RegisterUnit {
    /// Every variant, in declaration order.
    pub const ALL: [Self; 6] = [
        Self::KiloWattHour,
        Self::KiloVarHour,
        Self::KiloWatt,
        Self::KiloVar,
        Self::CubicMetre,
        Self::KiloWattHourThermal,
    ];

    /// Stable DB/wire label. Matches the `serde` tag and `FromStr` input.
    ///
    /// [`symbol`](Self::symbol) is the display form (`kWh`, `m³`); this is the
    /// code. They differ deliberately — a symbol carrying a superscript is not
    /// something to put in a database `CHECK` constraint.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::KiloWattHour => "KILO_WATT_HOUR",
            Self::KiloVarHour => "KILO_VAR_HOUR",
            Self::KiloWatt => "KILO_WATT",
            Self::KiloVar => "KILO_VAR",
            Self::CubicMetre => "CUBIC_METRE",
            Self::KiloWattHourThermal => "KILO_WATT_HOUR_THERMAL",
        }
    }

    /// The unit symbol, as printed on a meter display.
    #[must_use]
    pub const fn symbol(self) -> &'static str {
        match self {
            Self::KiloWattHour => "kWh",
            Self::KiloVarHour => "kvarh",
            Self::KiloWatt => "kW",
            Self::KiloVar => "kvar",
            Self::CubicMetre => "m³",
            Self::KiloWattHourThermal => "kWh_th",
        }
    }

    /// `true` when the register counts a quantity that accumulates — an energy
    /// or a volume — rather than an instantaneous power.
    ///
    /// Only a cumulative register can be differenced into a Lastgang; see
    /// [`crate::reading`].
    #[must_use]
    pub const fn is_cumulative(self) -> bool {
        !matches!(self, Self::KiloWatt | Self::KiloVar)
    }
}

crate::codes::string_codes! {
    RegisterUnit;
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
    /// Writes the canonical reduced form, omitting `*F` when F is
    /// [`STORAGE_UNUSED`](Self::STORAGE_UNUSED).
    ///
    /// The alternate flag `{:#}` forces the full six-group form — see
    /// [`to_full_string`](Self::to_full_string). Width, fill and alignment are
    /// honoured, so `{:>13}` lines a code up in a table.
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

impl From<ObisCode> for String {
    fn from(o: ObisCode) -> String {
        o.to_string()
    }
}

/// The shape [`ObisCode`] accepts, as rendered in a [`ParseError`].
const OBIS_FORMAT: &str = "A-B:C.D.E*F, e.g. 1-0:1.8.0 (the *F suffix is optional)";

impl FromStr for ObisCode {
    type Err = ParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        parse_obis(s).ok_or_else(|| ParseError::format("ObisCode", s, OBIS_FORMAT))
    }
}

impl TryFrom<&str> for ObisCode {
    type Error = ParseError;
    fn try_from(s: &str) -> Result<Self, Self::Error> {
        s.parse()
    }
}

// ── Lenient parser ────────────────────────────────────────────────────────────

/// Both spellings of `A-B:C.D.E*F`, plus leading zeros and surrounding
/// whitespace. Everything it accepts maps onto a single canonical
/// [`Display`](fmt::Display) output.
fn parse_obis(s: &str) -> Option<ObisCode> {
    let (a_str, rest) = s.trim().split_once('-')?;
    let (b_str, cd_ef) = rest.split_once(':')?;

    // C.D.E*F or C.D.E — the storage group is optional.
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

/// One value group: ASCII digits only.
///
/// `u8::from_str` alone would accept a `+` sign, so `+1-0:1.8.0*+255` parsed and
/// then rendered as `1-0:1.8.0` — a second spelling sneaking past the front
/// door. A group is digits or it is not a group.
fn group(s: &str) -> Option<u8> {
    if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    s.parse().ok()
}

// ── Serde: one string on the wire, readable from any deserialiser ─────────────

#[cfg(feature = "serde")]
mod serde_impl {
    use super::ObisCode;
    use serde::de::{self, Visitor};
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use std::fmt;

    impl Serialize for ObisCode {
        /// Writes the canonical [`Display`](fmt::Display) form, without an
        /// intermediate `String`.
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
        /// Accepts either spelling, from borrowing and non-borrowing
        /// deserialisers alike — `serde_json::from_reader`, bincode, postcard
        /// and MessagePack included. A visitor is what makes that work; a
        /// `try_from = "&str"` bridge would demand a borrowed `&str` and fail
        /// on all four.
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
        assert!(code.is_ht());
        assert!(!code.is_nt());
        assert_eq!(code.tariff_register(), Some(1));
    }

    #[test]
    fn parse_without_storage_number() {
        // *255 is optional — an absent F group means "not applicable".
        let code: ObisCode = "1-0:2.8.0".parse().unwrap();
        assert_eq!(code.f, 255);
        assert_eq!(code.c, 2);
        assert_eq!(code.d, 8);
        // C=2, D=8 = reverse active energy (Einspeisung)
        assert!(
            code.is_export(),
            "1-0:2.8.0 should be Einspeisung/export (C=2, D=8)"
        );
        assert!(code.is_einspeisung(), "should alias is_export()");
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

/// Value-group semantics, against the EDI@Energy *Codeliste der OBIS-Kennzahlen
/// und Medien* v2.5c (§2.1, §2.2, §2.3, §3.1).
///
/// The predicates read the German market's assignment of the value groups, not
/// the bare IEC group names, and each case below pins one of them.
#[cfg(test)]
mod mako_semantics_tests {
    use super::*;

    /// Direction is value group C alone. Requiring D = 8 reported the Lastgang —
    /// the code MSCONS PID 13018 carries, and the one a `MeterInterval` holds —
    /// as neither import nor export.
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
            assert!(c.is_einspeisung(), "{code} ({what}) aliases is_export");
            assert!(!c.is_import(), "{code} ({what}) is not Bezug");
        }
    }

    /// D is a Messart, not a direction. `1-0:3.9.0` is "Blindarbeit positiv,
    /// Vorschub" — it was modelled as an *export* register.
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

    /// A Lastgang is energy per interval (kWh); a Maximum is a power (kW).
    /// Billing the first as the second overstates the Leistungspreis basis.
    #[test]
    fn lastgang_and_maximum_are_different_registers() {
        let lastgang = ObisCode::STROM_BEZUG_LASTGANG;
        let maximum = ObisCode::STROM_BEZUG_MAXIMUM;

        assert_eq!(lastgang.to_string(), "1-0:1.29.0");
        assert_eq!(maximum.to_string(), "1-0:1.6.0");

        assert!(lastgang.is_lastgang() && !lastgang.is_maximum());
        assert!(maximum.is_maximum() && !maximum.is_lastgang());
        assert_ne!(lastgang, maximum);

        // The Lastgang is the 15-minute series; the Maximum is a single value
        // per billing period, so it has no interval resolution.
        use crate::resolution::IntervalResolution;
        assert_eq!(
            lastgang.default_resolution(),
            Some(IntervalResolution::QuarterHour)
        );
        assert_eq!(maximum.default_resolution(), None);
    }

    /// Blindleistung is C = 3…8, not just 3 and 4 — Q I…Q IV are C = 5…8.
    /// A quadrant register read as active energy puts kvarh in a kWh column.
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
        // ...and Wirkarbeit is not reactive.
        assert!(!ObisCode::STROM_BEZUG_TOTAL.is_reactive());
        assert!(!ObisCode::STROM_EINSPEISUNG_TOTAL.is_reactive());
    }

    /// The C group means something different per medium. Gas volume is
    /// `7-0:3.0.0`, and a C-only reactive check called it reactive energy.
    #[test]
    fn the_c_group_is_scoped_to_the_medium() {
        assert!(
            !ObisCode::GAS_VOLUME_M3.is_reactive(),
            "7-0:3.0.0 is a gas volume; C = 3 only means Blindleistung for A = 1"
        );
        assert!(ObisCode::GAS_VOLUME_M3.is_gas());

        // Nor is a gas or heat code an electricity direction.
        assert!(!ObisCode::GAS_VOLUME_M3.is_import());
        assert!(!ObisCode::WAERME_ENERGY.is_import());
    }

    /// E = 63 is the Fehlerregister, not tariff 63. Reporting it as a tariff
    /// invites a caller to bill a fault counter as consumption.
    #[test]
    fn fehlerregister_is_not_a_tariff() {
        let fehler: ObisCode = "1-0:1.8.63".parse().unwrap();
        assert!(fehler.is_fehlerregister());
        assert_eq!(fehler.tariff_register(), None);
        assert!(
            !fehler.is_total_register(),
            "E = 63 is not the total either"
        );
        assert_eq!(
            fehler.default_resolution(),
            None,
            "a fault counter is not a time series"
        );

        // Real tariffs still report themselves, across the post-2023 range.
        assert_eq!(ObisCode::STROM_BEZUG_HT.tariff_register(), Some(1));
        assert_eq!(ObisCode::STROM_BEZUG_NT.tariff_register(), Some(2));
        let t62: ObisCode = "1-0:1.8.62".parse().unwrap();
        assert_eq!(t62.tariff_register(), Some(62));
        assert!(!t62.is_fehlerregister());
        assert_eq!(ObisCode::STROM_BEZUG_TOTAL.tariff_register(), None);
    }
}

/// One channel, one string.
///
/// The bug these lock: `FromStr` defaulted the storage group to 255 and
/// `Display` always printed it, so `"1-0:1.8.0"` came back out as
/// `"1-0:1.8.0*255"`. A consumer keying stored rows on the string saw one
/// channel as two — and a correction written through the long spelling did not
/// supersede the reading written through the short one.
#[cfg(test)]
mod canonical_string_tests {
    use super::*;

    /// Codes as MSCONS carries them and as people type them. Each must survive
    /// `parse` → `to_string` unchanged; that is the property a stored key needs.
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
        // A genuine billing-period register keeps its F group, so the suffix
        // survives wherever it carries information.
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

    /// Two spellings, one merge key.
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

    /// Leading zeros and padding are accepted and normalised away, so an
    /// EDIFACT segment and a hand-typed cell land on the same key.
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
                ObisCode::normalize(s).unwrap_or_else(|e| panic!("{s}: {e}")),
                "1-0:1.8.0",
                "{s} must normalise onto the canonical spelling"
            );
        }
    }

    #[test]
    fn normalize_is_idempotent() {
        for s in CANONICAL {
            let once = ObisCode::normalize(s).unwrap();
            assert_eq!(ObisCode::normalize(&once).unwrap(), once, "{s}");
        }
    }

    /// The alternate flag is the escape hatch for systems that demand an
    /// explicit F, and it round-trips to the same value.
    #[test]
    fn full_form_is_available_and_parses_back() {
        let code = ObisCode::STROM_BEZUG_TOTAL;
        assert_eq!(code.to_full_string(), "1-0:1.8.0*255");
        assert_eq!(format!("{code:#}"), "1-0:1.8.0*255");
        assert_eq!(code.to_full_string().parse::<ObisCode>().unwrap(), code);
    }

    /// A non-default storage group is information, so it is never elided —
    /// eliding it would merge two genuinely different registers.
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

    /// A signed group is not a group. `u8::from_str` accepts `+1`, which would
    /// have let `+1-0:1.8.0*+255` in through the front door and back out under
    /// the canonical spelling.
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

    /// Ordering follows the value groups, so a channel listing sorts the way a
    /// reader expects rather than by string bytes.
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

    /// The value-group-A media list from the DLMS/COSEM Blue Book, which OMS
    /// Spec Vol. 2 adopts.
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

    /// Regression: every named constant must satisfy the predicate its name
    /// implies. `WAERME_ENERGY` was A = 8, which is cold water — so a heat
    /// register reported `is_water()` and inherited water's daily resolution.
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

        // ...and the resolution follows the medium, not the name.
        use crate::resolution::IntervalResolution;
        assert_eq!(
            ObisCode::WAERME_ENERGY.default_resolution(),
            Some(IntervalResolution::Hour),
            "a heat meter is read hourly, not daily like a water submeter"
        );
        assert_eq!(
            ObisCode::WASSER_KALT_VOLUME.default_resolution(),
            Some(IntervalResolution::Day)
        );
    }

    /// Water submetering is read daily at best — never on a 15-minute grid.
    #[test]
    fn water_defaults_to_daily_resolution() {
        use crate::resolution::IntervalResolution;
        let cold_water: ObisCode = "8-0:1.0.0".parse().unwrap();
        let heat: ObisCode = "6-0:1.0.0".parse().unwrap();
        assert_eq!(
            cold_water.default_resolution(),
            Some(IntervalResolution::Day)
        );
        assert_eq!(heat.default_resolution(), Some(IntervalResolution::Hour));
    }
}

#[cfg(test)]
mod default_resolution_tests {
    use super::*;
    use crate::resolution::IntervalResolution as R;

    fn code(s: &str) -> ObisCode {
        s.parse().unwrap_or_else(|_| panic!("{s} must parse"))
    }

    /// A channel that is not an equidistant series has no resolution, and the
    /// answer must not depend on whether the Messgröße is active or reactive:
    /// `1-0:5.6.0` and `1-0:1.6.0` are the same Maximum register one Messgröße
    /// apart, and both have none.
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
            assert_eq!(code(s).default_resolution(), Some(R::QuarterHour), "{s}");
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

    /// For the two gas conversion parameters, value group E **is** the
    /// averaging period. Reporting a monthly Brennwert as an hourly channel
    /// contradicted this file's own documentation of the same code.
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
        // An E this crate cannot read as a period is not guessed at.
        assert_eq!(code("7-0:54.0.5").default_resolution(), None);
    }

    /// Whatever it answers, it never claims a length a calendar period does
    /// not have — the invariant `IntervalResolution` exists to hold.
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
                            assert!(!matches!(r, R::Custom(_)), "{code} must not be Custom");
                        }
                    }
                }
            }
        }
    }
}
