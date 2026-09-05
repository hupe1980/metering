//! Gas unit conversion: m³ → kWh_Hs.
//!
//! ## Legal basis
//!
//! A gas meter registers m³. The kWh billed is derived, never measured, so the
//! conversion rests on the Eichrecht exceptions to §33 MessEG:
//!
//! - **§33 Abs. 1 MessEG**: a value for a Messgröße may only be used if it was
//!   determined with a Messgerät.
//! - **§25 Nr. 4 MessEV**: permits Brennwert values *"wenn sie nach den
//!   anerkannten Regeln der Technik ermittelt worden sind"*.
//! - **§25 Nr. 7 MessEV**: permits a value formed as a *"Summe, Differenz,
//!   Produkt oder Quotient"* of measured values — `V × Z × Hs` is the product
//!   case. The same exception carries every other derived quantity in this
//!   crate, under the condition that *"die Art der Berechnung und die
//!   verwendeten Werte für den vorgesehenen Verwendungszweck geeignet sind"*;
//!   see the crate-level docs.
//! - **DVGW G 685**: the anerkannte Regel der Technik referenced by §25 Nr. 4.
//!   Restructured in 2020 into parts, of which three matter here: **Teil 2**
//!   *Brennwert*, **Teil 3** *Volumen im Normzustand* (the Zustandszahl), and
//!   **Teil 6** *Kompressibilitätszahl (K-Zahl)*, which absorbed G 486.
//! - **DVGW G 260**: Gasbeschaffenheit, Hs-Bereich für Erdgas H/L.
//!
//! ## Formula
//!
//! ```text
//! kWh_Hs = V_m3 × Hs_kWh_per_m3 × Zustandszahl
//! ```
//!
//! where:
//! - `V_m3` — the **Betriebsvolumen** in m³, i.e. volume at meter measurement
//!   conditions. OBIS `7-0:3.0.0`
//!   ([`ObisCode::GAS_VOLUME_M3`](crate::ObisCode::GAS_VOLUME_M3)).
//! - `Hs_kWh_per_m3` — superior calorific value (Brennwert Ho / Hs) in kWh/m³
//!   as determined by the gas distributor for the supply area. OBIS
//!   `7-0:54.0.ee`, where E selects the averaging period.
//! - `Zustandszahl` — volume conversion factor (dimensionless, typically
//!   0.95–1.05) accounting for pressure and temperature at the meter. OBIS
//!   `7-0:52.0.22`. Either read off the Netzbetreiber's Höhenzonen table or
//!   computed with [`zustandszahl`] from the four inputs G 685-3 names.
//!
//! **Pass a Betriebsvolumen, not a Normvolumen.** `7-0:13.2.0` (Normvolumen
//! umgewertet) and `7-0:3.2.0` (Normvolumen gemessen) have already been
//! state-converted by the Mengenumwerter. Feeding one of those in applies the
//! Zustandszahl a second time and overstates the energy by the Zustandszahl's
//! deviation from 1 — a few percent, silently, on a billed quantity. For an
//! already-converted volume, pass `Decimal::ONE` as the Zustandszahl.
//!
//! ## Typical values for German natural gas (Erdgas H)
//!
//! | Parameter | Typical range | Unit |
//! |---|---|---|
//! | `hs_kwh_per_m3` | 9.5 – 12.0 | kWh/m³ |
//! | `zustandszahl` | 0.92 – 1.06 | dimensionless |
//!
//! ## Accuracy note
//!
//! All arithmetic uses [`rust_decimal::Decimal`] for exact decimal precision.
//! Never use `f64` for energy quantities — a 0.001% billing error on a 10 GWh/year
//! industrial customer is 100 kWh/year or ~EUR 10.

use rust_decimal::{Decimal, dec};

use crate::interval::MeasurementUnit;

/// Parameters for Gas m³ → kWh_Hs conversion.
///
/// Both are **operator data**, published per supply area and billing period —
/// which is why there is no `Default`. A typical-value Brennwert is a direct
/// multiplier on a billed quantity: a 10.55 stand-in against a real 11.20
/// understates every gas invoice in the portfolio by 6 %, with nothing in the
/// output to show for it. The same refusal as
/// [`LastgangConfig`](crate::reading::LastgangConfig)'s missing register width.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct GasConversionParams {
    /// Superior calorific value (Brennwert Ho / Hs) in kWh/m³.
    ///
    /// Published by the gas distributor per supply area — OBIS `7-0:54.0.ee`,
    /// where E selects the averaging period (16 hourly, 20 daily, 22 monthly).
    /// German Erdgas H runs roughly 9.5–12.0 kWh/m³.
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::decimal"))]
    pub hs_kwh_per_m3: Decimal,
    /// Volume conversion factor (Zustandszahl, dimensionless).
    ///
    /// Accounts for pressure and temperature at the meter — OBIS `7-0:52.0.22`,
    /// typically 0.92–1.06. From the Netzbetreiber's Höhenzonen table, or
    /// [`zustandszahl`]. Use [`already_converted`](Self::already_converted) for
    /// a volume a Mengenumwerter has already state-converted.
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::decimal"))]
    pub zustandszahl: Decimal,
}

impl GasConversionParams {
    /// Conversion parameters for a **Betriebsvolumen** — the volume at meter
    /// conditions, OBIS `7-0:3.0.0`.
    #[must_use]
    pub const fn new(hs_kwh_per_m3: Decimal, zustandszahl: Decimal) -> Self {
        Self {
            hs_kwh_per_m3,
            zustandszahl,
        }
    }

    /// Conversion parameters for a volume the Mengenumwerter has **already**
    /// state-converted — `7-0:13.2.0` Normvolumen umgewertet, or `7-0:3.2.0`
    /// Normvolumen gemessen.
    ///
    /// The Zustandszahl is `1`, because it has already been applied. Passing
    /// the real one a second time overstates the energy by its deviation from
    /// 1 — a few percent, silently, on a billed quantity.
    ///
    /// ```rust
    /// use metering::{GasConversionParams, normalize_to_kwh};
    /// use rust_decimal::dec;
    ///
    /// // 100 m³ of Normvolumen at 11.2 kWh/m³ is exactly 1 120 kWh.
    /// let params = GasConversionParams::already_converted(dec!(11.2));
    /// assert_eq!(normalize_to_kwh(dec!(100), "m3", Some(&params), None)?, dec!(1120.0));
    /// # Ok::<(), metering::ConversionError>(())
    /// ```
    #[must_use]
    pub const fn already_converted(hs_kwh_per_m3: Decimal) -> Self {
        Self {
            hs_kwh_per_m3,
            zustandszahl: Decimal::ONE,
        }
    }
}

/// Convert a Gas volume reading in m³ to energy in kWh_Hs.
///
/// Formula: `kWh_Hs = m3 × hs_kwh_per_m3 × zustandszahl`
///
/// # Example
/// ```rust
/// use metering::gas_m3_to_kwh_hs;
/// use rust_decimal::Decimal;
///
/// // 100 m³ × 10.55 kWh/m³ × 0.9764 = 1 030.102 kWh_Hs, exactly.
/// let kwh = gas_m3_to_kwh_hs(
///     Decimal::from(100u32),
///     Decimal::from_str_exact("10.55").unwrap(),
///     Decimal::from_str_exact("0.9764").unwrap(),
/// );
/// assert_eq!(kwh, Decimal::from_str_exact("1030.102000").unwrap());
/// ```
#[must_use]
pub fn gas_m3_to_kwh_hs(
    volume_m3: Decimal,
    hs_kwh_per_m3: Decimal,
    zustandszahl: Decimal,
) -> Decimal {
    volume_m3 * hs_kwh_per_m3 * zustandszahl
}

// ── G 685 rounding ────────────────────────────────────────────────────────────

/// How the final kWh amount of a G 685 thermal-energy calculation is rounded.
///
/// DVGW G 685 governs the calculation, but Netzbetreiber practice on the
/// *final* rounding demonstrably diverges (published Merkblätter show both
/// whole-kWh and two-decimal results), and the normative text is not freely
/// citable — so the final rounding is a configuration choice, not a
/// hard-coded claim.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "SCREAMING_SNAKE_CASE"))]
pub enum G685FinalRounding {
    /// No rounding — full Decimal precision (default; round at display time).
    #[default]
    None,
    /// Kaufmännisch to whole kWh (observed NB practice, e.g. eneregio).
    WholeKwh,
    /// Two decimal places (observed NB practice, e.g. Stadtwerke Mühlacker).
    TwoDecimals,
}

impl G685FinalRounding {
    /// Every rounding mode, in declaration order.
    pub const ALL: [Self; 3] = [Self::None, Self::WholeKwh, Self::TwoDecimals];

    /// Stable DB/wire label. Matches the `serde` tag and
    /// [`FromStr`](std::str::FromStr) input.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::None => "NONE",
            Self::WholeKwh => "WHOLE_KWH",
            Self::TwoDecimals => "TWO_DECIMALS",
        }
    }
}

crate::codes::string_codes! {
    G685FinalRounding;
}

/// Input rounding per published G 685 Netzbetreiber practice.
///
/// Consistently observed across NB Merkblätter zur thermischen Gasabrechnung:
/// Zustandszahl to **4** decimal places, Abrechnungsbrennwert to **3**
/// decimal places (kWh/m³). Both are applied to the inputs before the
/// multiplication so the stored calculation matches the published invoice
/// arithmetic digit for digit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct G685Rounding {
    /// Decimal places for the Zustandszahl (default 4).
    pub zustandszahl_dp: u32,
    /// Decimal places for the Abrechnungsbrennwert (default 3).
    pub brennwert_dp: u32,
    /// Final-amount rounding (default `None`).
    pub final_rounding: G685FinalRounding,
}

impl Default for G685Rounding {
    fn default() -> Self {
        Self {
            zustandszahl_dp: 4,
            brennwert_dp: 3,
            final_rounding: G685FinalRounding::None,
        }
    }
}

/// G 685 thermal-energy calculation with explicit rounding.
///
/// `kWh = V(m³) × round(Hs, brennwert_dp) × round(z, zustandszahl_dp)`,
/// then the configured final rounding. Rounding is *kaufmännisch* — half away
/// from zero — which is what the published Netzbetreiber Merkblätter compute
/// with; `Decimal`'s own default is the same rule, and it is stated here rather
/// than assumed.
#[must_use]
pub fn gas_m3_to_kwh_hs_rounded(
    volume_m3: Decimal,
    hs_kwh_per_m3: Decimal,
    zustandszahl: Decimal,
    rounding: G685Rounding,
) -> Decimal {
    use rust_decimal::RoundingStrategy::MidpointAwayFromZero;
    let hs = hs_kwh_per_m3.round_dp_with_strategy(rounding.brennwert_dp, MidpointAwayFromZero);
    let z = zustandszahl.round_dp_with_strategy(rounding.zustandszahl_dp, MidpointAwayFromZero);
    let kwh = volume_m3 * hs * z;
    match rounding.final_rounding {
        G685FinalRounding::None => kwh,
        G685FinalRounding::WholeKwh => kwh.round_dp_with_strategy(0, MidpointAwayFromZero),
        G685FinalRounding::TwoDecimals => kwh.round_dp_with_strategy(2, MidpointAwayFromZero),
    }
}

// ── Zustandszahl (DVGW G 685-3) ───────────────────────────────────────────────

/// Normzustand temperature: **273,15 K** (0 °C).
///
/// A defined value, not a measurement — DIN 1343, carried into DVGW G 685-3
/// *Gasabrechnung – Volumen im Normzustand*.
pub const NORMTEMPERATUR_K: Decimal = dec!(273.15);

/// Normzustand pressure: **1013,25 mbar**.
///
/// The other half of the Normzustand, and equally a defined value.
pub const NORMDRUCK_MBAR: Decimal = dec!(1013.25);

/// The Abrechnungstemperatur G 685-3 fixes for gas billing: **15 °C**.
///
/// A *Festwert*, not the temperature of any particular gas: the meter is not
/// obliged to measure one, so the rule fixes it and the Netzbetreiber does not
/// get to choose. `T_eff = 288,15 K`.
pub const ABRECHNUNGSTEMPERATUR_C: Decimal = dec!(15);

/// Highest gauge pressure at which `K = 1` may be assumed.
///
/// Below 1 bar the Kompressibilitätszahl of natural gas is within the
/// rounding of the Zustandszahl, which is why every Netzbetreiber Merkblatt for
/// household connections prints `K = 1`. Above it, K comes from G 685-6
/// (formerly G 486) and is an input like any other.
pub const K_EINS_GRENZE_MBAR: Decimal = dec!(1000);

/// What a [`zustandszahl`] is computed from.
///
/// Four inputs, none of them defaulted: the Zustandszahl multiplies a billed
/// quantity, so a wrong one is a percentage error on every invoice in the
/// Höhenzone. Two of the four are fixed by G 685-3 rather than chosen
/// ([`ABRECHNUNGSTEMPERATUR_C`], and `K = 1` below
/// [`K_EINS_GRENZE_MBAR`]), which is what [`niederdruck`](Self::niederdruck)
/// fills in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ZustandszahlParams {
    /// Mean air pressure of the Höhenzone at the meter, in mbar — `p_amb`.
    ///
    /// From [`hoehenzonen_luftdruck_mbar`], or measured by the Netzbetreiber
    /// in agreement with the Eichbehörde.
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::decimal"))]
    pub luftdruck_mbar: Decimal,
    /// Gauge pressure of the gas inside the meter, in mbar — `p_eff`.
    ///
    /// The Netzbetreiber's value for the pressure stage; 22 mbar is the usual
    /// figure for a household Niederdruck connection.
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::decimal"))]
    pub effektivdruck_mbar: Decimal,
    /// Abrechnungstemperatur in °C — `t`. See [`ABRECHNUNGSTEMPERATUR_C`].
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::decimal"))]
    pub abrechnungstemperatur_c: Decimal,
    /// Kompressibilitätszahl `K`, from DVGW G 685-6.
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::decimal"))]
    pub kompressibilitaetszahl: Decimal,
}

impl ZustandszahlParams {
    /// The two pressures, with the values G 685-3 fixes for a Niederdruck
    /// connection: `t = 15 °C` and `K = 1`.
    ///
    /// `None` at or above [`K_EINS_GRENZE_MBAR`], where `K = 1` stops being a
    /// safe assumption and G 685-6 has to be consulted — use [`new`](Self::new)
    /// with the K-Zahl for that case.
    #[must_use]
    pub fn niederdruck(luftdruck_mbar: Decimal, effektivdruck_mbar: Decimal) -> Option<Self> {
        (effektivdruck_mbar < K_EINS_GRENZE_MBAR).then_some(Self {
            luftdruck_mbar,
            effektivdruck_mbar,
            abrechnungstemperatur_c: ABRECHNUNGSTEMPERATUR_C,
            kompressibilitaetszahl: Decimal::ONE,
        })
    }

    /// All four inputs stated.
    #[must_use]
    pub const fn new(
        luftdruck_mbar: Decimal,
        effektivdruck_mbar: Decimal,
        abrechnungstemperatur_c: Decimal,
        kompressibilitaetszahl: Decimal,
    ) -> Self {
        Self {
            luftdruck_mbar,
            effektivdruck_mbar,
            abrechnungstemperatur_c,
            kompressibilitaetszahl,
        }
    }

    /// Absolute pressure at the meter: `p_amb + p_eff`, in mbar.
    ///
    /// The Effektivdruck is a *gauge* pressure — the amount by which the gas
    /// exceeds the surrounding air — so the absolute pressure the gas law wants
    /// is the sum, not either one.
    #[must_use]
    pub fn absolutdruck_mbar(&self) -> Decimal {
        self.luftdruck_mbar + self.effektivdruck_mbar
    }
}

/// Mean air pressure of a Höhenzone, in mbar: `1016 − 0,12 × H`.
///
/// G 685-3 has the Netzbetreiber divide the network into **Höhenzonen** and
/// bill each on one mean pressure, so that neighbouring customers are not
/// settled on different constants. A zone's stated mean height may not be more
/// than 50 m from its outermost boundary, which bounds the error this linear
/// approximation of the barometric formula can introduce.
///
/// ```rust
/// use metering::conversion::hoehenzonen_luftdruck_mbar;
/// use rust_decimal::dec;
///
/// assert_eq!(hoehenzonen_luftdruck_mbar(dec!(253)), dec!(985.64));
/// assert_eq!(hoehenzonen_luftdruck_mbar(dec!(0)), dec!(1016));
/// ```
#[must_use]
pub fn hoehenzonen_luftdruck_mbar(hoehe_m: Decimal) -> Decimal {
    dec!(1016) - dec!(0.12) * hoehe_m
}

/// The **Zustandszahl** `z`, which turns a Betriebsvolumen into a Normvolumen.
///
/// ```text
///        T_n           p_amb + p_eff        1
/// z =  ───────  ×  ───────────────────  ×  ───
///       T_eff             p_n               K
/// ```
///
/// Computed as the single quotient `(T_n × p) ÷ (T_eff × p_n × K)`, so the
/// products are exact and there is **one** rounding rather than three. `None`
/// when a denominator is not positive — an absolute zero or below, or a
/// non-positive K-Zahl, is not a gas state.
///
/// The result is at full width on purpose. The market's rounding of `z` is
/// [`G685Rounding::zustandszahl_dp`], and
/// [`gas_m3_to_kwh_hs_rounded`] applies it at the point of use; rounding here
/// as well would round twice, systematically.
///
/// ```rust
/// use metering::conversion::{
///     G685Rounding, ZustandszahlParams, gas_m3_to_kwh_hs_rounded,
///     hoehenzonen_luftdruck_mbar, zustandszahl,
/// };
/// use rust_decimal::dec;
///
/// // A household connection 253 m above sea level, 22 mbar Effektivdruck.
/// let params = ZustandszahlParams::niederdruck(
///     hoehenzonen_luftdruck_mbar(dec!(253)),
///     dec!(22),
/// ).expect("below one bar, so K = 1");
///
/// let z = zustandszahl(&params).expect("a positive gas state");
/// assert_eq!(z.round_dp(4), dec!(0.9427));
///
/// // 1 874 m³ over the year at an Abrechnungsbrennwert of 11,316 kWh/m³.
/// let kwh = gas_m3_to_kwh_hs_rounded(dec!(1874), dec!(11.316), z, G685Rounding::default());
/// assert_eq!(kwh.round_dp(2), dec!(19991.07));
/// ```
#[must_use]
pub fn zustandszahl(params: &ZustandszahlParams) -> Option<Decimal> {
    let t_eff = NORMTEMPERATUR_K + params.abrechnungstemperatur_c;
    let denominator = t_eff * NORMDRUCK_MBAR * params.kompressibilitaetszahl;
    let numerator = NORMTEMPERATUR_K * params.absolutdruck_mbar();
    (t_eff > Decimal::ZERO
        && params.kompressibilitaetszahl > Decimal::ZERO
        && numerator > Decimal::ZERO)
        .then(|| numerator / denominator)
}

/// Normalize a raw meter reading to kWh.
///
/// Handles the three shapes an ingest path actually receives:
///
/// | Source unit | Conversion |
/// |---|---|
/// | an energy unit ([`MeasurementUnit::parse_scaled`]) | rescale — kWh, Wh, MWh, GJ, MJ and the UN/ECE Rec 20 codes |
/// | a volume unit (m³, litres, `MTQ`) | `V × Hs × z`, needing `gas` |
/// | `"kW"` — a **power**, not an energy | `P × duration_h`, needing `duration_secs` |
///
/// `"kvar"` is deliberately **not** accepted. Integrating a reactive power over
/// an hour gives kvarh, which is a different dimension from kWh however similar
/// the arithmetic looks; a function named `normalize_to_kwh` returning it would
/// put a kvarh figure in a kWh column with nothing to catch it. Reactive
/// registers are identified by [`crate::ObisCode::is_reactive`] and their unit
/// by [`crate::obis::RegisterUnit`].
///
/// # Errors
///
/// Returns [`ConversionError`] rather than guessing. An unrecognised unit is
/// never assumed to be kWh already: `"MWh"` treated that way understates a
/// reading by a factor of a thousand, silently.
///
/// # Example
///
/// ```rust
/// use metering::{normalize_to_kwh, GasConversionParams};
/// use rust_decimal::{Decimal, dec};
///
/// // A gas volume needs the Brennwert and the Zustandszahl.
/// let gas = GasConversionParams { hs_kwh_per_m3: dec!(10.55), zustandszahl: dec!(0.98) };
/// let kwh = normalize_to_kwh(dec!(100), "m3", Some(&gas), None)?;
/// assert_eq!(kwh, dec!(1033.900));
///
/// // An energy unit only needs rescaling: 3.6 GJ is exactly 1000 kWh.
/// assert_eq!(normalize_to_kwh(dec!(3.6), "GJ", None, None)?, dec!(1000));
///
/// // A power needs the interval it was averaged over: 48 kW for 15 min = 12 kWh.
/// assert_eq!(normalize_to_kwh(dec!(48), "kW", None, Some(900))?, dec!(12));
///
/// // ...and an unknown unit is an error, not a silent pass-through.
/// assert!(normalize_to_kwh(dec!(1), "furlong", None, None).is_err());
/// # Ok::<(), metering::conversion::ConversionError>(())
/// ```
pub fn normalize_to_kwh(
    value: Decimal,
    unit: &str,
    gas: Option<&GasConversionParams>,
    duration_secs: Option<i64>,
) -> Result<Decimal, ConversionError> {
    let trimmed = unit.trim();
    if trimmed.eq_ignore_ascii_case("kw") {
        let secs = duration_secs.ok_or(ConversionError::MissingDuration)?;
        if secs <= 0 {
            return Err(ConversionError::MissingDuration);
        }
        // `P × secs ÷ 3600`, not `P × (secs ÷ 3600)`: the parenthesised
        // quotient does not terminate for most second counts, and rounding it
        // first carries the error into the product.
        return Ok(value * Decimal::from(secs) / Decimal::from(3600u32));
    }

    let scale = MeasurementUnit::parse_scaled(trimmed)
        .ok_or_else(|| ConversionError::UnknownUnit(trimmed.to_owned()))?;
    match scale.unit {
        MeasurementUnit::KiloWattHour => Ok(scale.apply(value)),
        MeasurementUnit::CubicMetre => {
            let params = gas.ok_or(ConversionError::MissingGasParameters)?;
            let m3 = scale.apply(value);
            Ok(gas_m3_to_kwh_hs(
                m3,
                params.hs_kwh_per_m3,
                params.zustandszahl,
            ))
        }
    }
}

/// Why a reading could not be normalised to kWh.
///
/// `#[non_exhaustive]`: a caller that wildcards an unfamiliar variant still
/// does the right thing — it reports a failure — so there is nothing to
/// protect. See the crate-level **Enum exhaustiveness** section.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum ConversionError {
    /// The unit string matched nothing this crate knows.
    #[error("unknown unit {0:?} — see MeasurementUnit::parse_scaled for the accepted set")]
    UnknownUnit(String),
    /// A volume was supplied without a Brennwert and Zustandszahl to convert it.
    #[error("a volume reading needs GasConversionParams (Brennwert and Zustandszahl)")]
    MissingGasParameters,
    /// A power was supplied without the interval length to integrate it over.
    #[error("a power reading needs a positive interval duration to become an energy")]
    MissingDuration,
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal::dec;

    #[test]
    fn gas_conversion_exact() {
        // 100 m³ × 10.55 kWh/m³ × 1.0 = 1055.00 kWh_Hs
        let kwh = gas_m3_to_kwh_hs(dec!(100), dec!(10.55), dec!(1.0));
        assert_eq!(kwh, dec!(1055.00));
    }

    #[test]
    fn gas_conversion_with_zustandszahl() {
        // 50 m³ × 10.80 kWh/m³ × 0.9800 = 529.20 kWh_Hs
        let kwh = gas_m3_to_kwh_hs(dec!(50), dec!(10.80), dec!(0.9800));
        assert_eq!(kwh, dec!(529.2000));
    }

    #[test]
    fn gas_conversion_zero_volume() {
        assert_eq!(gas_m3_to_kwh_hs(dec!(0), dec!(10.55), dec!(1.0)), dec!(0));
    }

    /// The Normzustand is its own fixed point: at 0 °C and 1013,25 mbar
    /// absolute, with `K = 1`, the Betriebsvolumen already **is** the
    /// Normvolumen.
    #[test]
    fn the_normzustand_has_a_zustandszahl_of_exactly_one() {
        let at_norm = ZustandszahlParams::new(NORMDRUCK_MBAR, dec!(0), dec!(0), Decimal::ONE);
        assert_eq!(zustandszahl(&at_norm), Some(Decimal::ONE));
    }

    /// The worked example every Netzbetreiber Merkblatt zur thermischen
    /// Gasabrechnung prints, end to end.
    ///
    /// A household connection in a Höhenzone of mean height 253 m, 22 mbar
    /// Effektivdruck: `p_amb = 1016 − 0,12 × 253 = 985,64 mbar`, absolute
    /// pressure 1007,64 mbar, and `z = 0,9427` at the four places G 685
    /// practice rounds to. 1 874 m³ at 11,316 kWh/m³ then settle at
    /// 19 991,07 kWh.
    #[test]
    fn the_g685_worked_example_reproduces() {
        let luftdruck = hoehenzonen_luftdruck_mbar(dec!(253));
        assert_eq!(luftdruck, dec!(985.64));

        let params = ZustandszahlParams::niederdruck(luftdruck, dec!(22))
            .expect("22 mbar is well below one bar");
        assert_eq!(params.absolutdruck_mbar(), dec!(1007.64));
        assert_eq!(params.abrechnungstemperatur_c, ABRECHNUNGSTEMPERATUR_C);
        assert_eq!(params.kompressibilitaetszahl, Decimal::ONE);

        let z = zustandszahl(&params).expect("a positive gas state");
        assert_eq!(z.round_dp(4), dec!(0.9427));

        let kwh = gas_m3_to_kwh_hs_rounded(dec!(1874), dec!(11.316), z, G685Rounding::default());
        assert_eq!(kwh.round_dp(2), dec!(19991.07));
    }

    /// `K = 1` is an assumption with a stated limit, so the constructor that
    /// makes it refuses to be used past that limit.
    #[test]
    fn the_k_equals_one_shortcut_stops_at_one_bar() {
        assert!(ZustandszahlParams::niederdruck(dec!(1013.25), dec!(999.9)).is_some());
        assert!(ZustandszahlParams::niederdruck(dec!(1013.25), K_EINS_GRENZE_MBAR).is_none());
        assert!(ZustandszahlParams::niederdruck(dec!(1013.25), dec!(4000)).is_none());
    }

    /// A gas state that cannot exist has no Zustandszahl, rather than a
    /// plausible-looking number.
    #[test]
    fn an_impossible_gas_state_has_no_zustandszahl() {
        // Absolute zero: the division would be by zero.
        let frozen = ZustandszahlParams::new(dec!(1013.25), dec!(0), dec!(-273.15), Decimal::ONE);
        assert_eq!(zustandszahl(&frozen), None);
        // Below absolute zero.
        let colder = ZustandszahlParams::new(dec!(1013.25), dec!(0), dec!(-300), Decimal::ONE);
        assert_eq!(zustandszahl(&colder), None);
        // A K-Zahl of zero or less is not a compressibility.
        let no_k = ZustandszahlParams::new(dec!(1013.25), dec!(0), dec!(15), Decimal::ZERO);
        assert_eq!(zustandszahl(&no_k), None);
        // A vacuum has no volume to convert.
        let vacuum = ZustandszahlParams::new(dec!(0), dec!(0), dec!(15), Decimal::ONE);
        assert_eq!(zustandszahl(&vacuum), None);
    }

    /// Higher ground is thinner air is less gas per cubic metre.
    ///
    /// The direction matters on an invoice: reading the Höhenzone off the wrong
    /// side of the table bills the customer for gas that was never delivered.
    #[test]
    fn a_higher_hoehenzone_has_a_smaller_zustandszahl() {
        let z_at = |h| {
            zustandszahl(
                &ZustandszahlParams::niederdruck(hoehenzonen_luftdruck_mbar(h), dec!(22))
                    .expect("niederdruck"),
            )
            .expect("a positive gas state")
        };
        assert!(z_at(dec!(0)) > z_at(dec!(253)));
        assert!(z_at(dec!(253)) > z_at(dec!(1000)));
    }

    /// A Normvolumen has already been state-converted, so the Zustandszahl to
    /// apply to it is `1`. Applying the real one again overstates the energy
    /// by its deviation from unity — silently, on a billed quantity.
    #[test]
    fn an_already_converted_volume_carries_no_zustandszahl() {
        let p = GasConversionParams::already_converted(dec!(11.2));
        assert_eq!(p.zustandszahl, Decimal::ONE);
        assert_eq!(p, GasConversionParams::new(dec!(11.2), Decimal::ONE));

        // 100 m³ Normvolumen at 11.2 kWh/m³ is exactly 1 120 kWh…
        let normvolumen = gas_m3_to_kwh_hs(dec!(100), p.hs_kwh_per_m3, p.zustandszahl);
        assert_eq!(normvolumen, dec!(1120.0));
        // …and applying a 0.98 Zustandszahl on top of it loses 2 %.
        let doubled = gas_m3_to_kwh_hs(dec!(100), dec!(11.2), dec!(0.98));
        assert!(doubled < normvolumen);
    }
}

// ── Warm water → heat energy (HeizkostenV §9 Abs. 2) ─────────────────────────

/// The `2,5` of HeizkostenV §9 Abs. 2 Satz 2 — a Zahlenwertgleichung constant.
///
/// A `dec!` literal rather than a parsed string: a parse that cannot fail does
/// not need a fallback, and every fallback here would have been a *different
/// number* applied silently to a billed quantity.
const WARMWASSER_FAKTOR: Decimal = dec!(2.5);

/// The assumed cold-water inlet temperature of §9 Abs. 2 Satz 2, in °C.
const KALTWASSER_TEMPERATUR_C: Decimal = dec!(10);

/// The `32` of HeizkostenV §9 Abs. 2 Satz 4, in kWh per m² and year.
const WARMWASSER_FLAECHENFAKTOR: Decimal = dec!(32);

/// §9 Abs. 2 Satz 6: *"bei brennwertbezogener Abrechnung von Erdgas mit 1,11 zu
/// multiplizieren"*.
const BRENNWERT_ERDGAS_FAKTOR: Decimal = dec!(1.11);

/// §9 Abs. 2 Satz 6: *"bei eigenständiger gewerblicher Wärmelieferung durch
/// 1,15 zu dividieren"*.
const GEWERBLICHE_WAERMELIEFERUNG_DIVISOR: Decimal = dec!(1.15);

/// §9 Abs. 2 Satz 6: *"bei dem Betrieb einer monovalenten Wärmepumpe mit 0,30
/// zu multiplizieren"*.
const MONOVALENTE_WAERMEPUMPE_FAKTOR: Decimal = dec!(0.30);

/// Adjustments applied to a §9 Abs. 2 result.
///
/// §9 Abs. 2 Satz 6 applies these to the result of *either* Zahlenwertgleichung
/// and does not make them exclusive, so more than one may hold at once.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct WarmWaterAdjustments {
    /// *"bei brennwertbezogener Abrechnung von Erdgas mit 1,11 zu multiplizieren"*.
    pub brennwert_erdgas: bool,
    /// *"bei eigenständiger gewerblicher Wärmelieferung durch 1,15 zu dividieren"*.
    ///
    /// **Eigenständig** is a term of art (cf. §1 Abs. 1 Nr. 2); ordinary
    /// commercial heat supply does not qualify.
    pub eigenstaendige_gewerbliche_waermelieferung: bool,
    /// *"bei dem Betrieb einer monovalenten Wärmepumpe mit 0,30 zu multiplizieren"*.
    pub monovalente_waermepumpe: bool,
}

impl WarmWaterAdjustments {
    /// No adjustment.
    pub const NONE: Self = Self {
        brennwert_erdgas: false,
        eigenstaendige_gewerbliche_waermelieferung: false,
        monovalente_waermepumpe: false,
    };

    fn apply(self, base: Decimal) -> Decimal {
        let mut q = base;
        if self.brennwert_erdgas {
            q *= BRENNWERT_ERDGAS_FAKTOR;
        }
        if self.eigenstaendige_gewerbliche_waermelieferung {
            q /= GEWERBLICHE_WAERMELIEFERUNG_DIVISOR;
        }
        if self.monovalente_waermepumpe {
            q *= MONOVALENTE_WAERMEPUMPE_FAKTOR;
        }
        q
    }
}

/// Heat attributable to a central warm-water system from the **metered volume**,
/// per HeizkostenV §9 Abs. 2 Satz 2.
///
/// ```text
/// Q [kWh/a] = 2.5 × V [m³] × (t_w [°C] − 10)
/// ```
///
/// §9 Abs. 2 Satz 1 requires the heat quantity to be **measured with a
/// Wärmezähler**. This equation is the fallback admitted only where measurement
/// *"nur mit einem unzumutbar hohen Aufwand"* is possible.
///
/// It is a *Zahlenwertgleichung* — a numerical-value equation, not dimensionally
/// consistent — so 2.5 carries no unit. §9 Abs. 2 Satz 3 Nr. 1 defines it as
/// covering the Erzeugeraufwandszahl des Wärmeerzeugers, the mittlere spezifische
/// Wärmekapazität des Wassers, the Wärmeverluste für Warmwasserspeicher,
/// Verteilung einschließlich Zirkulation, and Messdatenerhebungen zum
/// Warmwasserverbrauch. Because the Erzeugeraufwandszahl is inside the constant,
/// **Q is generator-input heat, not delivered useful heat**.
///
/// `mean_temp_c` is *"die gemessene oder geschätzte mittlere Temperatur"* — the
/// regulation permits an estimate and prescribes neither a default nor a cap.
///
/// # Example
///
/// ```rust
/// use metering::{warm_water_heat_kwh, WarmWaterAdjustments};
/// use rust_decimal::Decimal;
///
/// // 40 m³ of warm water at 60 °C
/// let q = warm_water_heat_kwh(
///     Decimal::from(40u32),
///     Decimal::from(60u32),
///     WarmWaterAdjustments::NONE,
/// );
/// assert_eq!(q, Decimal::from(5000u32)); // 2.5 × 40 × 50
/// ```
#[must_use]
pub fn warm_water_heat_kwh(
    volume_m3: Decimal,
    mean_temp_c: Decimal,
    adjustments: WarmWaterAdjustments,
) -> Decimal {
    adjustments.apply(WARMWASSER_FAKTOR * volume_m3 * (mean_temp_c - KALTWASSER_TEMPERATUR_C))
}

/// Heat attributable to a central warm-water system from **floor area**, per
/// HeizkostenV §9 Abs. 2 Satz 4: `Q [kWh/a] = 32 × A_Wohn [m²]`.
///
/// Admitted only *"in Ausnahmefällen"* where **neither** the heat quantity **nor**
/// the warm-water volume can be measured — a narrower trigger than an unmetered
/// volume alone.
///
/// `flaeche_m2` is the *"Wohn- oder Nutzfläche"* supplied with warm water by the
/// central system. §9 Abs. 2 Satz 5 Nr. 1 defines 32 as covering the
/// Nutzwärmebedarf für Warmwasser, the Erzeugeraufwandszahl and
/// Messdatenerhebungen — note this is a **different** bundle from the 2.5 of
/// Satz 2, excluding Speicher-, Verteilungs- und Zirkulationsverluste.
///
/// Separate from [`warm_water_heat_kwh`] rather than an `Option` parameter: a
/// metered volume and a floor-area estimate are different evidentiary categories,
/// so the caller states which it holds.
#[must_use]
pub fn warm_water_heat_kwh_unmetered(
    flaeche_m2: Decimal,
    adjustments: WarmWaterAdjustments,
) -> Decimal {
    adjustments.apply(WARMWASSER_FLAECHENFAKTOR * flaeche_m2)
}

#[cfg(test)]
mod warm_water_tests {
    use super::*;

    fn d(s: &str) -> Decimal {
        Decimal::from_str_exact(s).unwrap()
    }

    /// The worked identity from HeizkostenV §9 Abs. 2 Satz 2.
    #[test]
    fn metered_warm_water_follows_the_statutory_formula() {
        // 2.5 × 40 m³ × (60 − 10) = 5000 kWh
        assert_eq!(
            warm_water_heat_kwh(
                Decimal::from(40u32),
                Decimal::from(60u32),
                WarmWaterAdjustments::NONE
            ),
            Decimal::from(5000u32)
        );
    }

    /// At the assumed cold-inlet temperature there is no apportionable heat.
    /// Below it the result stays negative, signalling a bad temperature input.
    #[test]
    fn at_and_below_cold_inlet_temperature() {
        assert_eq!(
            warm_water_heat_kwh(
                Decimal::from(40u32),
                Decimal::from(10u32),
                WarmWaterAdjustments::NONE
            ),
            Decimal::ZERO
        );
        assert!(
            warm_water_heat_kwh(
                Decimal::from(40u32),
                Decimal::from(5u32),
                WarmWaterAdjustments::NONE
            ) < Decimal::ZERO
        );
    }

    #[test]
    fn adjustments_match_the_statutory_factors() {
        let v = Decimal::from(40u32);
        let t = Decimal::from(60u32);
        let base = Decimal::from(5000u32);

        let brennwert = WarmWaterAdjustments {
            brennwert_erdgas: true,
            ..WarmWaterAdjustments::NONE
        };
        assert_eq!(warm_water_heat_kwh(v, t, brennwert), base * d("1.11"));

        let wp = WarmWaterAdjustments {
            monovalente_waermepumpe: true,
            ..WarmWaterAdjustments::NONE
        };
        assert_eq!(warm_water_heat_kwh(v, t, wp), base * d("0.30"));

        // Eigenständige gewerbliche Wärmelieferung divides.
        let gewerblich = WarmWaterAdjustments {
            eigenstaendige_gewerbliche_waermelieferung: true,
            ..WarmWaterAdjustments::NONE
        };
        assert_eq!(warm_water_heat_kwh(v, t, gewerblich), base / d("1.15"));
    }

    /// §9 Abs. 2 Satz 6 does not make the three grounds exclusive, so a
    /// heat-pump system supplied under eigenständige gewerbliche Wärmelieferung
    /// takes both adjustments.
    #[test]
    fn adjustments_compose() {
        let both = WarmWaterAdjustments {
            eigenstaendige_gewerbliche_waermelieferung: true,
            monovalente_waermepumpe: true,
            ..WarmWaterAdjustments::NONE
        };
        let q = warm_water_heat_kwh(Decimal::from(40u32), Decimal::from(60u32), both);
        assert_eq!(q, Decimal::from(5000u32) / d("1.15") * d("0.30"));
    }

    /// The adjustments apply to the floor-area equation too ("Satz 2 oder 4").
    #[test]
    fn unmetered_fallback_uses_floor_area_and_takes_adjustments() {
        // 32 × 75 m² = 2400 kWh
        assert_eq!(
            warm_water_heat_kwh_unmetered(Decimal::from(75u32), WarmWaterAdjustments::NONE),
            Decimal::from(2400u32)
        );
        let brennwert = WarmWaterAdjustments {
            brennwert_erdgas: true,
            ..WarmWaterAdjustments::NONE
        };
        assert_eq!(
            warm_water_heat_kwh_unmetered(Decimal::from(75u32), brennwert),
            Decimal::from(2400u32) * d("1.11")
        );
    }

    #[test]
    fn g685_rounded_matches_the_published_eneregio_example() {
        // eneregio Merkblatt: 895 m³ × combined factor 10,8494 (= z 0,9543 ×
        // Hs 11,369) = 9.710 kWh, rounded to whole kWh.
        let kwh = gas_m3_to_kwh_hs_rounded(
            Decimal::from(895u32),
            Decimal::from_str_exact("11.369").unwrap(),
            Decimal::from_str_exact("0.9543").unwrap(),
            G685Rounding {
                final_rounding: G685FinalRounding::WholeKwh,
                ..G685Rounding::default()
            },
        );
        assert_eq!(kwh, Decimal::from(9710u32));
    }

    #[test]
    fn g685_input_rounding_is_kaufmaennisch() {
        // z given with 5 places rounds to 4 (0.95435 → 0.9544, away from zero).
        let kwh = gas_m3_to_kwh_hs_rounded(
            Decimal::ONE,
            Decimal::from_str_exact("10.0005").unwrap(), // → 10.001 (3 dp)
            Decimal::from_str_exact("0.95435").unwrap(), // → 0.9544 (4 dp)
            G685Rounding::default(),
        );
        assert_eq!(
            kwh,
            Decimal::from_str_exact("10.001").unwrap() * Decimal::from_str_exact("0.9544").unwrap()
        );
    }
}
