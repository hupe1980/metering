//! German Standard Load Profiles (Standardlastprofile, SLP).
//!
//! ## Legal basis
//!
//! - **VDEW Repräsentative Lastprofile** (1999): the original profiles.
//! - **BDEW "Hinweise zu den aktualisierten Standardlastprofilen Strom"**
//!   (17.03.2025): the first revision since 1999, built by BTU EVU Beratung on
//!   2018–2023 data. Their use is explicitly **voluntary** — *"Jedem
//!   Netzbetreiber steht es weiterhin frei, bei der Bilanzierung auf die
//!   aktualisierten Profile aus dem Jahr 2025, die alten Profile aus dem Jahr
//!   1999, eigene Profile oder eine Mischung der verschiedenen Optionen
//!   zurückzugreifen."*
//! - **§12 StromNZV / GaBi Gas 2.1 (BK7-24-01-008)**: the duty to apply standardised load
//!   profiles below 100 000 kWh/a (Strom) and 1.5 million kWh/a (Gas).
//!   ⚠️ Both ordinances were **repealed with effect from the end of
//!   31.12.2025** (Art. 15 Abs. 4 des Gesetzes vom 22.12.2023, BGBl. 2023 I
//!   Nr. 405); the substance now lives in BNetzA Festlegungen.
//!   (Neither StromGVV nor GasGVV ever governed load profiles — §18 of each is
//!   "Berechnungsfehler".)
//! - **GPKE / MaBiS**, in the consolidated Lesefassung of BNetzA **BK6-24-174**:
//!   SLP Marktlokationen are balanced and billed against a profile.
//!
//! ## Profile families
//!
//! | Family | Usage | Commodity |
//! |---|---|---|
//! | H0 | Residential households | Electricity |
//! | G0–G6 | Commercial, various sub-types | Electricity |
//! | L0–L2 | Agricultural (Landwirtschaft) | Electricity |
//! | P0 | Pumping stations | Electricity |
//! | H25/G25/L25/P25/S25 | The 2025 revision | Electricity |
//! | HEF/HMF/HKO | Households (heating / cooking gas) | Gas |
//! | GKO…GMF | Eleven Gewerbe types (TUM/FfE) | Gas |
//!
//! ## Usage
//!
//! The `LoadProfile` type classifies MaLos
//! and drives the billing-period aggregation method in SLP billing runs.

use rust_decimal::Decimal;
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// German Standard Load Profile identifier.
///
/// These are the official BDEW Standardlastprofile for electricity and gas.
/// Carried on the MaLo master record and on each billing period.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum LoadProfile {
    // ── Electricity profiles ──────────────────────────────────────────────────
    /// H0 — Haushalt (residential household).
    ///
    /// Most common profile. Used for residential customers without RLM.
    /// Demand pattern peaks in morning and evening.
    H0,

    /// G0 — Gewerbe allgemein (general commercial).
    ///
    /// Catch-all for commercial customers not covered by G1–G6.
    G0,

    /// G1 — Gewerbe wochentags (weekday-heavy commercial, 08:00–18:00 CET).
    ///
    /// Offices, small service businesses. High load Mon–Fri only.
    G1,

    /// G2 — Gewerbe mit starkem Verbrauch abends (evening-heavy commercial).
    ///
    /// Restaurants, entertainment. Load peak in evenings.
    G2,

    /// G3 — Gewerbe durchlaufend (continuous round-the-clock commercial).
    ///
    /// Bakeries, 24/7 operations. Nearly flat profile.
    G3,

    /// G4 — Laden/Friseur (retail shop with strong Saturday peak).
    G4,

    /// G5 — Bäckerei mit Backstube (bakery with overnight baking).
    ///
    /// Highest load in early morning hours.
    G5,

    /// G6 — Wochenendbetrieb (weekend-only operation, e.g. campsite).
    G6,

    /// L0 — Landwirtschaft allgemein (general agricultural).
    ///
    /// Mixed agricultural use. High load during harvest season.
    L0,

    /// L1 — Landwirtschaft mit Milchwirtschaft (dairy farming).
    ///
    /// Regular milking schedule. Load peaks around 05:00 and 17:00.
    L1,

    /// L2 — Landwirtschaft Sonstige (other agricultural without milking).
    L2,

    /// P0 — Pumpen (pumping stations).
    ///
    /// Relatively flat profile. Used for pumping/water supply.
    P0,

    // ── Aktualisierte BDEW-Profile 2025 ──────────────────────────────────────
    // BDEW "Hinweise zu den aktualisierten Standardlastprofilen Strom",
    // 17.03.2025. Twelve monthly seasons × three day types (WT/SA/FT) against
    // the bundeslandspezifischer Feiertagskalender, normed to 1 000 000 kWh/a.
    // See `DynamicSlpProfile` for the quoted specification.
    /// H25 — aktualisiertes Haushaltsprofil (successor to H0).
    ///
    /// Delivered "entdynamisiert": the Dynamisierungsfunktion MUST be applied
    /// (see [`Dynamization`]).
    H25,

    /// G25 — aktualisiertes Gewerbeprofil (single profile; replaces G0–G6).
    ///
    /// Carries no Dynamisierung — the function must NOT be applied.
    G25,

    /// L25 — aktualisiertes Landwirtschaftsprofil (single profile; replaces
    /// L0–L2). No Dynamisierung.
    L25,

    /// P25 — neues Kombinationsprofil PV (household delivery point with PV).
    ///
    /// Entdynamisiert — the H25 Dynamisierungsfunktion applies.
    P25,

    /// S25 — neues Kombinationsprofil PV + Speicher (household with PV and
    /// battery storage). Entdynamisiert — the Dynamisierungsfunktion applies.
    S25,

    // ── Gas profiles ─────────────────────────────────────────────────────────
    // The TUM/FfE profile types of the BDEW/VKU/GEODE Leitfaden "Abwicklung
    // von Standardlastprofilen Gas" — the temperature-dependent daily profiles
    // whose arithmetic lives in `crate::gas_slp`. Earlier releases carried
    // three invented codes ("EF", "MF", "GHD"); no gas SLP is named any of
    // those, and "GHD" in particular does not exist — the commercial sector is
    // eleven separate profile types.
    /// HEF — Haushalt, Einfamilienhaushalt (single-family, heating gas).
    #[cfg_attr(feature = "serde", serde(rename = "HEF"))]
    GasHEF,

    /// HMF — Haushalt, Mehrfamilienhaushalt (multi-family, heating gas).
    #[cfg_attr(feature = "serde", serde(rename = "HMF"))]
    GasHMF,

    /// HKO — Haushalt, Kochgas (cooking-gas-only household).
    #[cfg_attr(feature = "serde", serde(rename = "HKO"))]
    GasHKO,

    /// GKO — Gebietskörperschaften, Kreditinstitute und Versicherungen,
    /// Organisationen ohne Erwerbszweck, öffentliche Einrichtungen.
    #[cfg_attr(feature = "serde", serde(rename = "GKO"))]
    GasGKO,

    /// GHA — Einzel- und Großhandel.
    #[cfg_attr(feature = "serde", serde(rename = "GHA"))]
    GasGHA,

    /// GMK — Metall und Kfz.
    #[cfg_attr(feature = "serde", serde(rename = "GMK"))]
    GasGMK,

    /// GBD — sonstige betriebliche Dienstleistungen.
    #[cfg_attr(feature = "serde", serde(rename = "GBD"))]
    GasGBD,

    /// GGA — Gaststätten.
    #[cfg_attr(feature = "serde", serde(rename = "GGA"))]
    GasGGA,

    /// GBH — Beherbergung.
    #[cfg_attr(feature = "serde", serde(rename = "GBH"))]
    GasGBH,

    /// GWA — Wäschereien und chemische Reinigungen.
    #[cfg_attr(feature = "serde", serde(rename = "GWA"))]
    GasGWA,

    /// GGB — Gartenbau.
    #[cfg_attr(feature = "serde", serde(rename = "GGB"))]
    GasGGB,

    /// GBA — Backstuben (bakeries with a Backstube).
    #[cfg_attr(feature = "serde", serde(rename = "GBA"))]
    GasGBA,

    /// GPD — Papier und Druck.
    #[cfg_attr(feature = "serde", serde(rename = "GPD"))]
    GasGPD,

    /// GMF — haushaltsähnliche Gewerbebetriebe.
    #[cfg_attr(feature = "serde", serde(rename = "GMF"))]
    GasGMF,

    // ── Legacy / other ────────────────────────────────────────────────────────
    /// Custom profile — not a standard BDEW profile.
    /// The profile name is stored separately in the MaLo record.
    #[cfg_attr(feature = "serde", serde(rename = "CUSTOM"))]
    Custom,
}

impl LoadProfile {
    /// The canonical BDEW profile identifier string.
    ///
    /// Used in UTILMD `MR+Z07`/`MR+Z08` segments and the MaLo lastprofil field.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::H0 => "H0",
            Self::G0 => "G0",
            Self::G1 => "G1",
            Self::G2 => "G2",
            Self::G3 => "G3",
            Self::G4 => "G4",
            Self::G5 => "G5",
            Self::G6 => "G6",
            Self::L0 => "L0",
            Self::L1 => "L1",
            Self::L2 => "L2",
            Self::P0 => "P0",
            Self::H25 => "H25",
            Self::G25 => "G25",
            Self::L25 => "L25",
            Self::P25 => "P25",
            Self::S25 => "S25",
            Self::GasHEF => "HEF",
            Self::GasHMF => "HMF",
            Self::GasHKO => "HKO",
            Self::GasGKO => "GKO",
            Self::GasGHA => "GHA",
            Self::GasGMK => "GMK",
            Self::GasGBD => "GBD",
            Self::GasGGA => "GGA",
            Self::GasGBH => "GBH",
            Self::GasGWA => "GWA",
            Self::GasGGB => "GGB",
            Self::GasGBA => "GBA",
            Self::GasGPD => "GPD",
            Self::GasGMF => "GMF",
            Self::Custom => "CUSTOM",
        }
    }

    /// Every variant, in declaration order.
    pub const ALL: [Self; 32] = [
        Self::H0,
        Self::G0,
        Self::G1,
        Self::G2,
        Self::G3,
        Self::G4,
        Self::G5,
        Self::G6,
        Self::L0,
        Self::L1,
        Self::L2,
        Self::P0,
        Self::H25,
        Self::G25,
        Self::L25,
        Self::P25,
        Self::S25,
        Self::GasHEF,
        Self::GasHMF,
        Self::GasHKO,
        Self::GasGKO,
        Self::GasGHA,
        Self::GasGMK,
        Self::GasGBD,
        Self::GasGGA,
        Self::GasGBH,
        Self::GasGWA,
        Self::GasGGB,
        Self::GasGBA,
        Self::GasGPD,
        Self::GasGMF,
        Self::Custom,
    ];

    /// The canonical codes, in the same order as [`ALL`](Self::ALL).
    ///
    /// [`parse`](Self::parse) also accepts `"EF"` and `"MF"` — the codes
    /// earlier releases of this crate wrote for HEF and HMF — as lenient
    /// aliases; they normalise onto the canonical spelling.
    pub const CODES: &'static [&'static str] = &[
        "H0", "G0", "G1", "G2", "G3", "G4", "G5", "G6", "L0", "L1", "L2", "P0", "H25", "G25",
        "L25", "P25", "S25", "HEF", "HMF", "HKO", "GKO", "GHA", "GMK", "GBD", "GGA", "GBH", "GWA",
        "GGB", "GBA", "GPD", "GMF", "CUSTOM",
    ];

    /// Parse from the BDEW profile identifier string.
    ///
    /// Returns `None` for unknown profile codes. [`FromStr`](std::str::FromStr)
    /// is the same parse with a [`ParseError`](crate::ParseError) instead.
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_uppercase().as_str() {
            "H0" => Some(Self::H0),
            "G0" => Some(Self::G0),
            "G1" => Some(Self::G1),
            "G2" => Some(Self::G2),
            "G3" => Some(Self::G3),
            "G4" => Some(Self::G4),
            "G5" => Some(Self::G5),
            "G6" => Some(Self::G6),
            "L0" => Some(Self::L0),
            "L1" => Some(Self::L1),
            "L2" => Some(Self::L2),
            "P0" => Some(Self::P0),
            "H25" => Some(Self::H25),
            "G25" => Some(Self::G25),
            "L25" => Some(Self::L25),
            "P25" => Some(Self::P25),
            "S25" => Some(Self::S25),
            "HEF" => Some(Self::GasHEF),
            "HMF" => Some(Self::GasHMF),
            "HKO" => Some(Self::GasHKO),
            "GKO" => Some(Self::GasGKO),
            "GHA" => Some(Self::GasGHA),
            "GMK" => Some(Self::GasGMK),
            "GBD" => Some(Self::GasGBD),
            "GGA" => Some(Self::GasGGA),
            "GBH" => Some(Self::GasGBH),
            "GWA" => Some(Self::GasGWA),
            "GGB" => Some(Self::GasGGB),
            "GBA" => Some(Self::GasGBA),
            "GPD" => Some(Self::GasGPD),
            "GMF" => Some(Self::GasGMF),
            // Lenient aliases for the codes earlier releases wrote. "GHD" is
            // deliberately not one: no gas SLP is named GHD, and there is no
            // single profile it could map onto — the commercial sector is
            // eleven distinct types.
            "EF" => Some(Self::GasHEF),
            "MF" => Some(Self::GasHMF),
            // `Custom` is a real variant with a real code, so it must parse
            // back — `as_str` emits "CUSTOM", and a mapping whose inverse drops
            // a variant turns a stored profile into a parse failure on read.
            "CUSTOM" => Some(Self::Custom),
            _ => None,
        }
    }

    /// `true` when this is a residential profile.
    #[must_use]
    pub fn is_residential(self) -> bool {
        matches!(
            self,
            Self::H0
                | Self::H25
                | Self::P25
                | Self::S25
                | Self::GasHEF
                | Self::GasHMF
                | Self::GasHKO
        )
    }

    /// `true` for profiles delivered "entdynamisiert", to which the
    /// Dynamisierungsfunktion must be applied (H25, P25, S25).
    ///
    /// G25 and L25 carry none, verbatim: *"Das Profil enthält keine
    /// Dynamisierung und die Dynamisierungsfunktion ist hier nicht
    /// anzuwenden."* The 1999 profiles keep their historical handling — H0
    /// dynamized, G0–G6 and L0–L2 static.
    ///
    /// Source: BDEW *Hinweise zu den aktualisierten Standardlastprofilen
    /// Strom*, 17.03.2025, §§2.1–2.5.
    #[must_use]
    pub fn requires_dynamization(self) -> bool {
        matches!(self, Self::H0 | Self::H25 | Self::P25 | Self::S25)
    }

    /// `true` when this is a commercial profile — G0–G6, G25, or one of the
    /// eleven gas Gewerbe types.
    #[must_use]
    pub fn is_commercial(self) -> bool {
        matches!(
            self,
            Self::G0
                | Self::G1
                | Self::G2
                | Self::G3
                | Self::G4
                | Self::G5
                | Self::G6
                | Self::G25
                | Self::GasGKO
                | Self::GasGHA
                | Self::GasGMK
                | Self::GasGBD
                | Self::GasGGA
                | Self::GasGBH
                | Self::GasGWA
                | Self::GasGGB
                | Self::GasGBA
                | Self::GasGPD
                | Self::GasGMF
        )
    }

    /// `true` when this is an agricultural profile (L0–L2).
    #[must_use]
    pub fn is_agricultural(self) -> bool {
        matches!(self, Self::L0 | Self::L1 | Self::L2)
    }

    /// `true` when this is a gas SLP profile — the TUM/FfE daily profiles
    /// evaluated by [`crate::gas_slp`].
    #[must_use]
    pub fn is_gas(self) -> bool {
        matches!(
            self,
            Self::GasHEF
                | Self::GasHMF
                | Self::GasHKO
                | Self::GasGKO
                | Self::GasGHA
                | Self::GasGMK
                | Self::GasGBD
                | Self::GasGGA
                | Self::GasGBH
                | Self::GasGWA
                | Self::GasGGB
                | Self::GasGBA
                | Self::GasGPD
                | Self::GasGMF
        )
    }
}

impl std::fmt::Display for LoadProfile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl std::str::FromStr for LoadProfile {
    type Err = crate::error::ParseError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        LoadProfile::parse(s)
            .ok_or_else(|| crate::error::ParseError::one_of("LoadProfile", s, Self::CODES))
    }
}

// ── Dynamisierung ─────────────────────────────────────────────────────────────

/// A Dynamisierungsfunktion — a quartic in the day of the year.
///
/// ```text
/// f(t) = a·t⁴ + b·t³ + c·t² + d·t + e
/// ```
///
/// where `t` is the day of the year, `1` = 1 January. The coefficients are a
/// field rather than a constant because **which** quartic applies is a property
/// of the profile, and only one of them can be cited here — see
/// [`vdew_1999`](Self::vdew_1999).
///
/// ## Rounding
///
/// BDEW *Hinweise zu den aktualisierten Standardlastprofilen Strom*
/// (17.03.2025), §2.1, verbatim: *"Eine Rundung der Dynamisierungsfaktoren auf
/// vier Nachkommastellen wird empfohlen. Das Ergebnis wird auf drei
/// Nachkommastellen gerundet."* — factors to four decimal places, the dynamized
/// value to three. [`factor`](Self::factor) and [`apply`](Self::apply) do
/// exactly that.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct Dynamization {
    /// Quartic coefficients `(a, b, c, d, e)`, highest power first.
    pub coefficients: [f64; 5],
}

impl Dynamization {
    /// The 1999 VDEW dynamization polynomial, published with the H0 profile:
    ///
    /// ```text
    /// f(t) = −3.92e−10·t⁴ + 3.2e−7·t³ − 7.02e−5·t² + 2.1e−3·t + 1.24
    /// ```
    ///
    /// ## This is the 1999 function, and only that
    ///
    /// The 2025 Anwendungshilfe prints its Dynamisierungsfunktion **as an
    /// image**, so its coefficients cannot be read out of the published
    /// document, quoted, or verified against this one. They may well be the
    /// same quartic; this crate does not assert it either way.
    ///
    /// So `Dynamization` is a parameter throughout. An operator loading the
    /// licensed 2025 tables supplies the function that came with them —
    /// [`DynamicSlpProfile::dynamization`] — rather than inheriting a guess.
    /// A wrong dynamization is a silent few-percent error on every SLP
    /// settlement in the balance group, which is precisely the kind of claim
    /// that should not be hard-coded on an assumption.
    #[must_use]
    pub fn vdew_1999() -> Self {
        Self {
            coefficients: [-3.92e-10, 3.2e-7, -7.02e-5, 2.1e-3, 1.24],
        }
    }

    /// Dynamization factor for `day_of_year` (1..=366), rounded to 4 decimal
    /// places per the Anwendungshilfe recommendation.
    #[must_use]
    pub fn factor(&self, day_of_year: u16) -> Decimal {
        let t = f64::from(day_of_year);
        let [a, b, c, d, e] = self.coefficients;
        let f = a * t.powi(4) + b * t.powi(3) + c * t.powi(2) + d * t + e;
        Decimal::try_from(f).unwrap_or(Decimal::ONE).round_dp(4)
    }

    /// Apply the factor to a profile value; the result is rounded to 3
    /// decimal places per the Anwendungshilfe.
    #[must_use]
    pub fn apply(&self, profile_value: Decimal, day_of_year: u16) -> Decimal {
        (profile_value * self.factor(day_of_year)).round_dp(3)
    }
}

// ── 2025 dynamic profile tables ───────────────────────────────────────────────

/// Day types of the 2025 BDEW profiles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "SCREAMING_SNAKE_CASE"))]
pub enum SlpDayType {
    /// Werktag (Mon–Fri, not a public holiday).
    Werktag,
    /// Samstag (not a public holiday).
    Samstag,
    /// Sonn- und Feiertag (Bundesland-specific holiday calendar).
    SonnFeiertag,
}

/// A 2025-generation profile table.
///
/// BDEW *Hinweise zu den aktualisierten Standardlastprofilen Strom*
/// (17.03.2025), §1, states the shape verbatim:
///
/// > Alle neuen Profile arbeiten nun mit **zwölf Monaten (Saisons)**. […] Alle
/// > neuen Profile arbeiten mit **drei Typtagen**: Werktage (WT), Samstage (SA)
/// > sowie Sonn- und Feiertage (FT). […] Es gilt der **bundeslandspezifische
/// > Feiertagskalender** nach Definition des BDEW. […] Alle Profile sind auf
/// > **1 Mio. kWh** Jahresverbrauchsmenge normiert.
///
/// So the table is 12 × 3 × 96 values, and the day type is resolved against the
/// **delivery point's Bundesland** — which is what
/// [`crate::slp_day_type`] does and why it takes a
/// [`Bundesland`](crate::Bundesland).
///
/// The value tables themselves are licensed BDEW data and are **not** embedded
/// here; the operator loads them into this container. The library contributes
/// the shape, the lookup and the Dynamisierung rules.
#[derive(Debug, Clone, Default)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct DynamicSlpProfile {
    /// Which profile the table belongs to (H25/G25/L25/P25/S25).
    pub profile: Option<LoadProfile>,
    /// `values[(month, day_type)]` → 96 quarter-hour values.
    /// `month` is 1..=12.
    pub values: std::collections::BTreeMap<(u8, SlpDayType), Vec<Decimal>>,
    /// The Dynamisierungsfunktion that came with the table.
    ///
    /// Supplied rather than assumed: the 2025 Anwendungshilfe publishes its
    /// function as an image, so this crate cannot verify its coefficients — see
    /// [`Dynamization::vdew_1999`]. `None` on a profile that
    /// [`requires_dynamization`](LoadProfile::requires_dynamization) makes
    /// [`value_at`](Self::value_at) return `None` rather than silently
    /// returning an entdynamisiert value as if it were a real one.
    pub dynamization: Option<Dynamization>,
}

impl DynamicSlpProfile {
    /// The profile value for `month` (1..=12), `day_type`, `quarter` (0..96),
    /// with the Dynamisierungsfunktion applied when the profile requires one.
    ///
    /// Returns `None` when the table has no entry for the key, **or** when the
    /// profile needs dynamizing and no [`dynamization`](Self::dynamization) was
    /// supplied. The second case is deliberate: an entdynamisiert H25 value is
    /// not a load-profile value, and returning it as one understates winter and
    /// overstates summer by up to a quarter.
    #[must_use]
    pub fn value_at(
        &self,
        month: u8,
        day_type: SlpDayType,
        quarter: usize,
        day_of_year: u16,
    ) -> Option<Decimal> {
        let raw = self
            .values
            .get(&(month, day_type))
            .and_then(|day| day.get(quarter))
            .copied()?;
        // An unknown profile is treated as needing dynamization: the safe
        // failure is refusing to answer, not answering with a raw value.
        if self.profile.is_none_or(LoadProfile::requires_dynamization) {
            return Some(self.dynamization?.apply(raw, day_of_year));
        }
        Some(raw)
    }

    /// [`value_at`](Self::value_at) for a calendar date, resolving the month,
    /// the day type and the day of the year from it.
    ///
    /// This is the lookup an SLP settlement actually performs: it has a date, a
    /// Bundesland and a quarter-hour index, not a pre-computed day type.
    ///
    /// ```rust
    /// use metering::load_profile::{DynamicSlpProfile, Dynamization, SlpDayType};
    /// use metering::{Bundesland, LoadProfile};
    /// use rust_decimal::dec;
    /// use time::macros::date;
    ///
    /// let mut h25 = DynamicSlpProfile {
    ///     profile: Some(LoadProfile::H25),
    ///     dynamization: Some(Dynamization::vdew_1999()),
    ///     ..Default::default()
    /// };
    /// h25.values.insert((6, SlpDayType::SonnFeiertag), vec![dec!(100); 96]);
    ///
    /// // Fronleichnam 2026 is a Thursday — but a Sonn-/Feiertag in Bavaria.
    /// let bavarian = h25.value_on(date!(2026 - 06 - 04), Bundesland::By, 0);
    /// assert!(bavarian.is_some());
    /// // In Berlin the same date is a Werktag, and that table is not loaded.
    /// assert!(h25.value_on(date!(2026 - 06 - 04), Bundesland::Be, 0).is_none());
    /// ```
    #[must_use]
    pub fn value_on(
        &self,
        date: time::Date,
        land: crate::holiday::Bundesland,
        quarter: usize,
    ) -> Option<Decimal> {
        self.value_at(
            u8::from(date.month()),
            crate::holiday::slp_day_type(date, land),
            quarter,
            date.ordinal(),
        )
    }

    /// `true` when all 12 × 3 day tables are present with 96 values each.
    #[must_use]
    pub fn is_complete(&self) -> bool {
        let day_types = [
            SlpDayType::Werktag,
            SlpDayType::Samstag,
            SlpDayType::SonnFeiertag,
        ];
        (1u8..=12).all(|m| {
            day_types
                .iter()
                .all(|dt| self.values.get(&(m, *dt)).is_some_and(|v| v.len() == 96))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_all_profiles() {
        let profiles = [
            LoadProfile::H0,
            LoadProfile::G0,
            LoadProfile::G1,
            LoadProfile::G2,
            LoadProfile::G3,
            LoadProfile::G4,
            LoadProfile::G5,
            LoadProfile::G6,
            LoadProfile::L0,
            LoadProfile::L1,
            LoadProfile::L2,
            LoadProfile::P0,
            LoadProfile::GasHEF,
            LoadProfile::GasHMF,
            LoadProfile::GasHKO,
            LoadProfile::GasGKO,
            LoadProfile::GasGGA,
            LoadProfile::GasGMF,
        ];
        for p in &profiles {
            let s = p.as_str();
            let parsed = LoadProfile::parse(s).expect("should round-trip");
            assert_eq!(*p, parsed, "round-trip failed for {s}");
        }
    }

    #[test]
    fn residential_classification() {
        assert!(LoadProfile::H0.is_residential());
        assert!(LoadProfile::GasHEF.is_residential());
        assert!(LoadProfile::GasHKO.is_residential());
        assert!(
            !LoadProfile::GasGMF.is_residential(),
            "GMF is haushaltsähnlich, not a household"
        );
        assert!(!LoadProfile::G0.is_residential());
    }

    #[test]
    fn commercial_classification() {
        for p in [
            LoadProfile::G0,
            LoadProfile::G1,
            LoadProfile::G2,
            LoadProfile::G3,
            LoadProfile::G4,
            LoadProfile::G5,
            LoadProfile::G6,
        ] {
            assert!(p.is_commercial(), "{p} should be commercial");
        }
        assert!(!LoadProfile::H0.is_commercial());
    }

    #[test]
    fn gas_classification() {
        let gas_codes = [
            "HEF", "HMF", "HKO", "GKO", "GHA", "GMK", "GBD", "GGA", "GBH", "GWA", "GGB", "GBA",
            "GPD", "GMF",
        ];
        for p in LoadProfile::ALL {
            assert_eq!(p.is_gas(), gas_codes.contains(&p.as_str()), "{p}");
        }
        assert!(LoadProfile::GasHEF.is_gas());
        assert!(LoadProfile::GasGKO.is_gas());
        assert!(!LoadProfile::H0.is_gas());
    }

    #[test]
    fn case_insensitive_parse() {
        assert_eq!(LoadProfile::parse("h0"), Some(LoadProfile::H0));
        assert_eq!(LoadProfile::parse("H0"), Some(LoadProfile::H0));
        assert_eq!(LoadProfile::parse("hef"), Some(LoadProfile::GasHEF));
        // The pre-0.18 codes are lenient aliases onto the canonical spelling.
        assert_eq!(LoadProfile::parse("EF"), Some(LoadProfile::GasHEF));
        assert_eq!(LoadProfile::parse("MF"), Some(LoadProfile::GasHMF));
        // ...but "GHD" never named a gas SLP and maps to nothing.
        assert_eq!(LoadProfile::parse("GHD"), None);
    }

    #[test]
    fn unknown_profile_returns_none() {
        assert_eq!(LoadProfile::parse("X9"), None);
        assert_eq!(LoadProfile::parse(""), None);
    }

    #[test]
    fn profiles_2025_round_trip_and_dynamization_flags() {
        for (code, dynamized) in [
            ("H25", true),
            ("G25", false),
            ("L25", false),
            ("P25", true),
            ("S25", true),
        ] {
            let p = LoadProfile::parse(code).expect(code);
            assert_eq!(p.as_str(), code);
            assert_eq!(p.requires_dynamization(), dynamized, "{code}");
        }
        // The 1999 handling is preserved: H0 dynamized, G0/L0 static.
        assert!(LoadProfile::H0.requires_dynamization());
        assert!(!LoadProfile::G0.requires_dynamization());
        assert!(!LoadProfile::L0.requires_dynamization());
    }

    #[test]
    fn vdew_1999_dynamization_matches_the_published_shape() {
        use rust_decimal::dec;
        let d = Dynamization::vdew_1999();
        // Factors are 4-decimal rounded; winter above 1, summer below 1.
        let jan = d.factor(15);
        let jul = d.factor(196);
        assert!(jan > Decimal::ONE, "winter factor {jan} > 1");
        assert!(jul < Decimal::ONE, "summer factor {jul} < 1");
        assert_eq!(jan, jan.round_dp(4));
        // Result rounding to 3 decimals (Anwendungshilfe, verbatim rule).
        let applied = d.apply(dec!(1.23456), 15);
        assert_eq!(applied, applied.round_dp(3));
    }

    #[test]
    fn dynamic_profile_lookup_applies_dynamization_only_where_required() {
        use rust_decimal::dec;
        let mut h25 = DynamicSlpProfile {
            profile: Some(LoadProfile::H25),
            dynamization: Some(Dynamization::vdew_1999()),
            ..Default::default()
        };
        h25.values
            .insert((1, SlpDayType::Werktag), vec![dec!(100); 96]);
        let v = h25.value_at(1, SlpDayType::Werktag, 0, 15).unwrap();
        assert_ne!(v, dec!(100), "H25 must be dynamized");

        let mut g25 = DynamicSlpProfile {
            profile: Some(LoadProfile::G25),
            ..Default::default()
        };
        g25.values
            .insert((1, SlpDayType::Werktag), vec![dec!(100); 96]);
        assert_eq!(
            g25.value_at(1, SlpDayType::Werktag, 0, 15).unwrap(),
            dec!(100),
            "G25 carries no Dynamisierung, and needs none supplied"
        );
        assert!(!g25.is_complete(), "one month/day-type is not a full table");
    }

    /// An entdynamisiert value is not a load-profile value. With no function
    /// supplied the lookup refuses rather than handing back the raw table
    /// entry, which would understate winter and overstate summer by up to a
    /// quarter.
    #[test]
    fn a_profile_needing_dynamization_refuses_without_a_function() {
        use rust_decimal::dec;
        let mut h25 = DynamicSlpProfile {
            profile: Some(LoadProfile::H25),
            dynamization: None,
            ..Default::default()
        };
        h25.values
            .insert((1, SlpDayType::Werktag), vec![dec!(100); 96]);
        assert_eq!(h25.value_at(1, SlpDayType::Werktag, 0, 15), None);

        // The same table with a function answers.
        h25.dynamization = Some(Dynamization::vdew_1999());
        assert!(h25.value_at(1, SlpDayType::Werktag, 0, 15).is_some());

        // An unknown profile is treated as needing one, for the same reason.
        let unknown = DynamicSlpProfile {
            profile: None,
            values: h25.values.clone(),
            dynamization: None,
        };
        assert_eq!(unknown.value_at(1, SlpDayType::Werktag, 0, 15), None);
    }

    /// The date-based lookup resolves month, day type and day-of-year together,
    /// against the delivery point's Bundesland — the calendar the BDEW
    /// Anwendungshilfe names.
    #[test]
    fn the_date_lookup_uses_the_bundesland_calendar() {
        use crate::Bundesland;
        use rust_decimal::dec;
        use time::macros::date;

        let mut h25 = DynamicSlpProfile {
            profile: Some(LoadProfile::H25),
            dynamization: Some(Dynamization::vdew_1999()),
            ..Default::default()
        };
        // Only the June Sonn-/Feiertag table is loaded.
        h25.values
            .insert((6, SlpDayType::SonnFeiertag), vec![dec!(100); 96]);

        // Fronleichnam 2026 is a Thursday, and a Feiertag only in some Länder.
        let fronleichnam = date!(2026 - 06 - 04);
        assert!(h25.value_on(fronleichnam, Bundesland::By, 0).is_some());
        assert!(
            h25.value_on(fronleichnam, Bundesland::Be, 0).is_none(),
            "in Berlin it is a Werktag, and that table is not loaded"
        );

        // The dynamization factor follows the day of the year, so two dates in
        // the same month and day type still differ.
        h25.values
            .insert((1, SlpDayType::SonnFeiertag), vec![dec!(100); 96]);
        let new_year = h25.value_on(date!(2026 - 01 - 01), Bundesland::By, 0);
        let midsummer = h25.value_on(date!(2026 - 06 - 07), Bundesland::By, 0);
        assert!(new_year > midsummer, "winter runs above summer");
    }
}
