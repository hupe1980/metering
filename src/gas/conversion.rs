//! Gas unit conversion: m³ → kWh_Hs.
//!
//! ```text
//! kWh_Hs = V_m3 × Hs_kWh_per_m3 × Zustandszahl
//! ```
//!
//! - `V_m3` — the **Betriebsvolumen** (volume at meter conditions), OBIS
//!   `7-0:3.0.0` ([`ObisCode::GAS_VOLUME_M3`](crate::ObisCode::GAS_VOLUME_M3)).
//! - `Hs_kWh_per_m3` — the Brennwert (Ho / Hs) of the supply area, OBIS
//!   `7-0:54.0.ee`; Erdgas H runs roughly 9.5–12.0 kWh/m³.
//! - `Zustandszahl` — the dimensionless volume conversion factor, OBIS
//!   `7-0:52.0.22`, typically 0.92–1.06; from the Netzbetreiber's Höhenzonen
//!   table or [`zustandszahl`].
//!
//! **Pass a Betriebsvolumen, not a Normvolumen.** `7-0:13.2.0` and `7-0:3.2.0`
//! are already state-converted by the Mengenumwerter; applying the
//! Zustandszahl again skews the energy by its deviation from 1. Use
//! [`GasConversionParams::already_converted`] for them.
//!
//! ## Legal basis
//!
//! - **§33 Abs. 1 MessEG**: a value for a Messgröße may only be used if it was
//!   determined with a Messgerät.
//! - **§25 Nr. 4 MessEV**: permits Brennwert values *"wenn sie nach den
//!   anerkannten Regeln der Technik ermittelt worden sind"*.
//! - **§25 Nr. 7 MessEV**: permits a value formed as a *"Summe, Differenz,
//!   Produkt oder Quotient"* of measured values — `V × Z × Hs` is the product
//!   case, provided *"die Art der Berechnung und die
//!   verwendeten Werte für den vorgesehenen Verwendungszweck geeignet sind"*.
//! - **DVGW G 685** (the anerkannte Regel der Technik): **Teil 2** *Brennwert*,
//!   **Teil 3** *Volumen im Normzustand*, **Teil 6** *Kompressibilitätszahl*.
//! - **DVGW G 260**: Gasbeschaffenheit, Hs-Bereich für Erdgas H/L.

use rust_decimal::{Decimal, dec};

use crate::series::interval::Unit;

/// Parameters for Gas m³ → kWh_Hs conversion.
///
/// Both are operator data, published per supply area and billing period, so
/// there is no `Default`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub struct GasConversionParams {
    /// Brennwert (Ho / Hs) in kWh/m³ — OBIS `7-0:54.0.ee`, E selecting the
    /// averaging period (16 hourly, 20 daily, 22 monthly).
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::decimal"))]
    hs_kwh_per_m3: Decimal,
    /// Zustandszahl (dimensionless) — OBIS `7-0:52.0.22`.
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::decimal"))]
    zustandszahl: Decimal,
}

impl GasConversionParams {
    /// The Brennwert, kWh/m³.
    #[must_use]
    pub const fn hs_kwh_per_m3(&self) -> Decimal {
        self.hs_kwh_per_m3
    }

    /// The Zustandszahl.
    #[must_use]
    pub const fn zustandszahl(&self) -> Decimal {
        self.zustandszahl
    }

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
    /// The Zustandszahl is `1`, because it has already been applied.
    ///
    /// ```rust
    /// use metering::{gas::conversion::GasConversionParams, gas::conversion::normalize_to_kwh};
    /// use rust_decimal::dec;
    ///
    /// // 100 m³ of Normvolumen at 11.2 kWh/m³ is exactly 1 120 kWh.
    /// let params = GasConversionParams::already_converted(dec!(11.2));
    /// assert_eq!(normalize_to_kwh(dec!(100), "m3", Some(&params), None)?, dec!(1120.0));
    /// # Ok::<(), metering::gas::conversion::ConversionError>(())
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
/// `kWh_Hs = m3 × hs_kwh_per_m3 × zustandszahl`, unrounded. `None` on overflow.
///
/// ```rust
/// use metering::gas::conversion::gas_m3_to_kwh_hs;
/// use rust_decimal::Decimal;
///
/// // 100 m³ × 10.55 kWh/m³ × 0.9764 = 1 030.102 kWh_Hs, exactly.
/// let kwh = gas_m3_to_kwh_hs(
///     Decimal::from(100u32),
///     Decimal::from_str_exact("10.55").unwrap(),
///     Decimal::from_str_exact("0.9764").unwrap(),
/// );
/// assert_eq!(kwh, Some(Decimal::from_str_exact("1030.102000").unwrap()));
/// ```
#[must_use]
pub fn gas_m3_to_kwh_hs(
    volume_m3: Decimal,
    hs_kwh_per_m3: Decimal,
    zustandszahl: Decimal,
) -> Option<Decimal> {
    volume_m3
        .checked_mul(hs_kwh_per_m3)?
        .checked_mul(zustandszahl)
}

// ── G 685 rounding ────────────────────────────────────────────────────────────

/// How the final kWh amount of a G 685 thermal-energy calculation is rounded.
///
/// A configuration choice: published Netzbetreiber Merkblätter show both
/// whole-kWh and two-decimal results.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum G685FinalRounding {
    /// No rounding — full Decimal precision (round at display time).
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

crate::ids::codes::string_codes! {
    G685FinalRounding;
}

/// Input rounding per published G 685 Netzbetreiber practice.
///
/// The input places are applied before the multiplication, matching the
/// Merkblätter zur thermischen Gasabrechnung digit for digit. No `Default`:
/// start from [`PUBLISHED_PRACTICE`](Self::PUBLISHED_PRACTICE) and adjust.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub struct G685Rounding {
    zustandszahl_dp: u32,
    brennwert_dp: u32,
    final_rounding: G685FinalRounding,
}

impl G685Rounding {
    /// Zustandszahl to [`G685_ZUSTANDSZAHL_DP`](crate::precision::G685_ZUSTANDSZAHL_DP)
    /// places, Brennwert to [`G685_BRENNWERT_DP`](crate::precision::G685_BRENNWERT_DP),
    /// no final rounding.
    pub const PUBLISHED_PRACTICE: Self = Self {
        zustandszahl_dp: crate::precision::G685_ZUSTANDSZAHL_DP,
        brennwert_dp: crate::precision::G685_BRENNWERT_DP,
        final_rounding: G685FinalRounding::None,
    };

    /// Decimal places of the Zustandszahl.
    #[must_use]
    pub const fn zustandszahl_dp(mut self, dp: u32) -> Self {
        self.zustandszahl_dp = dp;
        self
    }

    /// Decimal places of the Abrechnungsbrennwert.
    #[must_use]
    pub const fn brennwert_dp(mut self, dp: u32) -> Self {
        self.brennwert_dp = dp;
        self
    }

    /// The Netzbetreiber's rounding of the final amount.
    #[must_use]
    pub const fn final_rounding(mut self, mode: G685FinalRounding) -> Self {
        self.final_rounding = mode;
        self
    }
}

/// G 685 thermal-energy calculation with explicit rounding.
///
/// `kWh = V(m³) × round(Hs, brennwert_dp) × round(z, zustandszahl_dp)`,
/// then the configured final rounding, all *kaufmännisch* (half away from
/// zero). `None` on overflow.
#[must_use]
pub fn gas_m3_to_kwh_hs_rounded(
    volume_m3: Decimal,
    hs_kwh_per_m3: Decimal,
    zustandszahl: Decimal,
    rounding: G685Rounding,
) -> Option<Decimal> {
    use crate::precision::G685_STRATEGY;
    let hs = hs_kwh_per_m3.round_dp_with_strategy(rounding.brennwert_dp, G685_STRATEGY);
    let z = zustandszahl.round_dp_with_strategy(rounding.zustandszahl_dp, G685_STRATEGY);
    let kwh = gas_m3_to_kwh_hs(volume_m3, hs, z)?;
    Some(match rounding.final_rounding {
        G685FinalRounding::None => kwh,
        G685FinalRounding::WholeKwh => kwh.round_dp_with_strategy(0, G685_STRATEGY),
        G685FinalRounding::TwoDecimals => kwh.round_dp_with_strategy(2, G685_STRATEGY),
    })
}

// ── Zustandszahl (DVGW G 685-3) ───────────────────────────────────────────────

/// Normzustand temperature: **273,15 K** (0 °C).
///
/// DIN 1343, as used by DVGW G 685-3.
pub const NORMTEMPERATUR_K: Decimal = dec!(273.15);

/// Normzustand pressure: **1013,25 mbar** (DIN 1343).
pub const NORMDRUCK_MBAR: Decimal = dec!(1013.25);

/// The Abrechnungstemperatur G 685-3 fixes for gas billing: **15 °C**.
///
/// A fixed value, not a measurement: `T_eff = 288,15 K`.
pub const ABRECHNUNGSTEMPERATUR_C: Decimal = dec!(15);

/// The gauge pressure at which the `K = 1` assumption stops — **1 000 mbar**,
/// exclusive. At or above it, K comes from G 685-6 via
/// [`ZustandszahlParams::new`].
pub const K_EINS_GRENZE_MBAR: Decimal = dec!(1000);

/// What a [`zustandszahl`] is computed from.
///
/// No defaults. [`below_one_bar`](Self::below_one_bar) fills in the two values
/// G 685-3 fixes ([`ABRECHNUNGSTEMPERATUR_C`], `K = 1`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub struct ZustandszahlParams {
    /// Mean air pressure of the Höhenzone, mbar — `p_amb`
    /// ([`hoehenzonen_luftdruck_mbar`] or the Netzbetreiber's value).
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::decimal"))]
    luftdruck_mbar: Decimal,
    /// Gauge pressure in the meter, mbar — `p_eff` (typically 22 mbar for a
    /// household Niederdruck connection).
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::decimal"))]
    effektivdruck_mbar: Decimal,
    /// Abrechnungstemperatur in °C — `t`. See [`ABRECHNUNGSTEMPERATUR_C`].
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::decimal"))]
    abrechnungstemperatur_c: Decimal,
    /// Kompressibilitätszahl `K`, from DVGW G 685-6.
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::decimal"))]
    kompressibilitaetszahl: Decimal,
}

impl ZustandszahlParams {
    /// The two pressures, with the two values G 685-3 fixes rather than
    /// measures: `t = 15 °C` and `K = 1`.
    ///
    /// `None` at or above [`K_EINS_GRENZE_MBAR`]; use [`new`](Self::new) with
    /// the G 685-6 K-Zahl there.
    #[must_use]
    pub fn below_one_bar(luftdruck_mbar: Decimal, effektivdruck_mbar: Decimal) -> Option<Self> {
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

    /// Mean air pressure of the Höhenzone, mbar — `p_amb`.
    #[must_use]
    pub const fn luftdruck_mbar(&self) -> Decimal {
        self.luftdruck_mbar
    }

    /// Gauge pressure in the meter, mbar — `p_eff`.
    #[must_use]
    pub const fn effektivdruck_mbar(&self) -> Decimal {
        self.effektivdruck_mbar
    }

    /// Abrechnungstemperatur, °C.
    #[must_use]
    pub const fn abrechnungstemperatur_c(&self) -> Decimal {
        self.abrechnungstemperatur_c
    }

    /// Kompressibilitätszahl `K`.
    #[must_use]
    pub const fn kompressibilitaetszahl(&self) -> Decimal {
        self.kompressibilitaetszahl
    }

    /// Absolute pressure at the meter: `p_amb + p_eff`, in mbar. `None` on
    /// overflow.
    #[must_use]
    pub fn absolutdruck_mbar(&self) -> Option<Decimal> {
        self.luftdruck_mbar.checked_add(self.effektivdruck_mbar)
    }
}

/// Mean air pressure of a Höhenzone, in mbar: `1016 − 0,12 × H`.
///
/// `H` is the Höhenzone's mean height in m (G 685-3: at most 50 m from the
/// zone's outermost boundary).
///
/// ```rust
/// use metering::gas::conversion::hoehenzonen_luftdruck_mbar;
/// use rust_decimal::dec;
///
/// assert_eq!(hoehenzonen_luftdruck_mbar(dec!(253)), Some(dec!(985.64)));
/// assert_eq!(hoehenzonen_luftdruck_mbar(dec!(0)), Some(dec!(1016)));
/// ```
///
/// `None` on overflow.
#[must_use]
pub fn hoehenzonen_luftdruck_mbar(hoehe_m: Decimal) -> Option<Decimal> {
    dec!(1016).checked_sub(dec!(0.12).checked_mul(hoehe_m)?)
}

/// The **Zustandszahl** `z`, which turns a Betriebsvolumen into a Normvolumen.
///
/// ```text
///        T_n           p_amb + p_eff        1
/// z =  ───────  ×  ───────────────────  ×  ───
///       T_eff             p_n               K
/// ```
///
/// Computed as the single quotient `(T_n × p) ÷ (T_eff × p_n × K)`, unrounded
/// ([`gas_m3_to_kwh_hs_rounded`] rounds at the point of use). `None` when
/// `T_eff`, `K` or the absolute pressure is not positive, or on overflow.
///
/// ```rust
/// use metering::gas::conversion::{
///     G685Rounding, ZustandszahlParams, gas_m3_to_kwh_hs_rounded,
///     hoehenzonen_luftdruck_mbar, zustandszahl,
/// };
/// use rust_decimal::dec;
///
/// // A household connection 253 m above sea level, 22 mbar Effektivdruck.
/// let params = ZustandszahlParams::below_one_bar(
///     hoehenzonen_luftdruck_mbar(dec!(253)).unwrap(),
///     dec!(22),
/// ).expect("below one bar, so K = 1");
///
/// let z = zustandszahl(&params).expect("a positive gas state");
/// assert_eq!(z.round_dp_with_strategy(4, metering::precision::G685_STRATEGY), dec!(0.9427));
///
/// // 1 874 m³ over the year at an Abrechnungsbrennwert of 11,316 kWh/m³.
/// let kwh = gas_m3_to_kwh_hs_rounded(dec!(1874), dec!(11.316), z, G685Rounding::PUBLISHED_PRACTICE).unwrap();
/// assert_eq!(kwh.round_dp_with_strategy(2, metering::precision::G685_STRATEGY), dec!(19991.07));
/// ```
#[must_use]
pub fn zustandszahl(params: &ZustandszahlParams) -> Option<Decimal> {
    let t_eff = NORMTEMPERATUR_K.checked_add(params.abrechnungstemperatur_c)?;
    let denominator = t_eff
        .checked_mul(NORMDRUCK_MBAR)?
        .checked_mul(params.kompressibilitaetszahl)?;
    let numerator = NORMTEMPERATUR_K.checked_mul(params.absolutdruck_mbar()?)?;
    if t_eff <= Decimal::ZERO
        || params.kompressibilitaetszahl <= Decimal::ZERO
        || numerator <= Decimal::ZERO
    {
        return None;
    }
    numerator.checked_div(denominator)
}

/// Normalize a raw meter reading to kWh.
///
///
/// | Source unit | Conversion |
/// |---|---|
/// | an energy unit ([`Unit::parse_scaled`]) | rescale — kWh, Wh, MWh, GJ, MJ and the UN/ECE Rec 20 codes |
/// | a volume unit (m³, litres, `MTQ`) | `V × Hs × z`, needing `gas` |
/// | `"kW"` — a **power**, not an energy | `P × duration_h`, needing `duration_secs` |
///
/// Reactive units (`kvar`, `kvarh`) are not accepted: kvarh is not kWh.
///
/// # Errors
///
/// [`ConversionError::UnknownUnit`] for a unit not listed above (never assumed
/// to be kWh), [`ConversionError::MissingGasParameters`] for a volume without
/// `gas`, [`ConversionError::MissingDuration`] for `kW` without a positive
/// `duration_secs`, [`ConversionError::Overflow`] on overflow.
///
/// # Example
///
/// ```rust
/// use metering::{gas::conversion::normalize_to_kwh, gas::conversion::GasConversionParams};
/// use rust_decimal::{Decimal, dec};
///
/// // A gas volume needs the Brennwert and the Zustandszahl.
/// let gas = GasConversionParams::new(dec!(10.55), dec!(0.98));
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
/// # Ok::<(), metering::gas::conversion::ConversionError>(())
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
        return value
            .checked_mul(Decimal::from(secs))
            .and_then(|e| e.checked_div(Decimal::from(3600u32)))
            .ok_or(ConversionError::Overflow);
    }

    let scale = Unit::parse_scaled(trimmed)
        .ok_or_else(|| ConversionError::UnknownUnit(trimmed.to_owned()))?;
    match scale.unit() {
        Unit::KiloWattHour => scale.apply(value).ok_or(ConversionError::Overflow),
        Unit::CubicMetre => {
            let params = gas.ok_or(ConversionError::MissingGasParameters)?;
            let m3 = scale.apply(value).ok_or(ConversionError::Overflow)?;
            gas_m3_to_kwh_hs(m3, params.hs_kwh_per_m3, params.zustandszahl)
                .ok_or(ConversionError::Overflow)
        }
        // `parse_scaled` only ever yields the two storage units.
        _ => Err(ConversionError::UnknownUnit(trimmed.to_owned())),
    }
}

/// Why a reading could not be normalised to kWh.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum ConversionError {
    /// The unit string matched nothing this crate knows.
    #[error("unknown unit {0:?} — see Unit::parse_scaled for the accepted set")]
    UnknownUnit(String),
    /// A volume was supplied without a Brennwert and Zustandszahl to convert it.
    #[error("a volume reading needs GasConversionParams (Brennwert and Zustandszahl)")]
    MissingGasParameters,
    /// A power was supplied without the interval length to integrate it over.
    #[error("a power reading needs a positive interval duration to become an energy")]
    MissingDuration,
    /// The rescaled value does not fit a `Decimal`.
    #[error("the converted value overflows")]
    Overflow,
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal::dec;

    #[test]
    fn gas_conversion_exact() {
        // 100 m³ × 10.55 kWh/m³ × 1.0 = 1055.00 kWh_Hs
        let kwh = gas_m3_to_kwh_hs(dec!(100), dec!(10.55), dec!(1.0));
        assert_eq!(kwh, Some(dec!(1055.00)));
        assert_eq!(gas_m3_to_kwh_hs(Decimal::MAX, dec!(10.55), dec!(1.0)), None);
    }

    #[test]
    fn gas_conversion_with_zustandszahl() {
        // 50 m³ × 10.80 kWh/m³ × 0.9800 = 529.20 kWh_Hs
        let kwh = gas_m3_to_kwh_hs(dec!(50), dec!(10.80), dec!(0.9800));
        assert_eq!(kwh, Some(dec!(529.2000)));
    }

    #[test]
    fn gas_conversion_zero_volume() {
        assert_eq!(
            gas_m3_to_kwh_hs(dec!(0), dec!(10.55), dec!(1.0)),
            Some(dec!(0))
        );
    }

    /// The Normzustand is its own fixed point: at 0 °C and 1013,25 mbar
    /// absolute, with `K = 1`, the Betriebsvolumen already **is** the
    /// Normvolumen.
    #[test]
    fn the_normzustand_has_a_zustandszahl_of_exactly_one() {
        let at_norm = ZustandszahlParams::new(NORMDRUCK_MBAR, dec!(0), dec!(0), Decimal::ONE);
        assert_eq!(zustandszahl(&at_norm), Some(Decimal::ONE));
    }

    /// The Merkblatt worked example: 253 m, 22 mbar → z = 0,9427; 1 874 m³ at
    /// 11,316 kWh/m³ → 19 991,07 kWh.
    #[test]
    fn the_g685_worked_example_reproduces() {
        let luftdruck = hoehenzonen_luftdruck_mbar(dec!(253)).unwrap();
        assert_eq!(luftdruck, dec!(985.64));

        let params = ZustandszahlParams::below_one_bar(luftdruck, dec!(22))
            .expect("22 mbar is well below one bar");
        assert_eq!(params.absolutdruck_mbar(), Some(dec!(1007.64)));
        assert_eq!(params.abrechnungstemperatur_c(), ABRECHNUNGSTEMPERATUR_C);
        assert_eq!(params.kompressibilitaetszahl(), Decimal::ONE);

        let z = zustandszahl(&params).expect("a positive gas state");
        assert_eq!(
            z.round_dp_with_strategy(4, crate::precision::G685_STRATEGY),
            dec!(0.9427)
        );

        let kwh = gas_m3_to_kwh_hs_rounded(
            dec!(1874),
            dec!(11.316),
            z,
            G685Rounding::PUBLISHED_PRACTICE,
        )
        .unwrap();
        assert_eq!(
            kwh.round_dp_with_strategy(2, crate::precision::G685_STRATEGY),
            dec!(19991.07)
        );
    }

    #[test]
    fn the_k_equals_one_shortcut_stops_at_one_bar() {
        assert!(ZustandszahlParams::below_one_bar(dec!(1013.25), dec!(999.9)).is_some());
        assert!(ZustandszahlParams::below_one_bar(dec!(1013.25), K_EINS_GRENZE_MBAR).is_none());
        assert!(ZustandszahlParams::below_one_bar(dec!(1013.25), dec!(4000)).is_none());
    }

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
    #[test]
    fn a_higher_hoehenzone_has_a_smaller_zustandszahl() {
        let z_at = |h| {
            zustandszahl(
                &ZustandszahlParams::below_one_bar(
                    hoehenzonen_luftdruck_mbar(h).unwrap(),
                    dec!(22),
                )
                .expect("below one bar"),
            )
            .expect("a positive gas state")
        };
        assert!(z_at(dec!(0)) > z_at(dec!(253)));
        assert!(z_at(dec!(253)) > z_at(dec!(1000)));
    }

    #[test]
    fn an_already_converted_volume_carries_no_zustandszahl() {
        let p = GasConversionParams::already_converted(dec!(11.2));
        assert_eq!(p.zustandszahl(), Decimal::ONE);
        assert_eq!(p, GasConversionParams::new(dec!(11.2), Decimal::ONE));

        // 100 m³ Normvolumen at 11.2 kWh/m³ is exactly 1 120 kWh…
        let normvolumen = gas_m3_to_kwh_hs(dec!(100), p.hs_kwh_per_m3(), p.zustandszahl()).unwrap();
        assert_eq!(normvolumen, dec!(1120.0));
        // …and applying a 0.98 Zustandszahl on top of it loses 2 %.
        let doubled = gas_m3_to_kwh_hs(dec!(100), dec!(11.2), dec!(0.98)).unwrap();
        assert!(doubled < normvolumen);
    }
}

#[cfg(test)]
mod g685_rounding_tests {
    use super::*;

    #[test]
    fn g685_rounded_matches_the_published_eneregio_example() {
        // eneregio Merkblatt: 895 m³ × combined factor 10,8494 (= z 0,9543 ×
        // Hs 11,369) = 9.710 kWh, rounded to whole kWh.
        let kwh = gas_m3_to_kwh_hs_rounded(
            Decimal::from(895u32),
            Decimal::from_str_exact("11.369").unwrap(),
            Decimal::from_str_exact("0.9543").unwrap(),
            G685Rounding::PUBLISHED_PRACTICE.final_rounding(G685FinalRounding::WholeKwh),
        );
        assert_eq!(kwh, Some(Decimal::from(9710u32)));
    }

    #[test]
    fn g685_input_rounding_is_kaufmaennisch() {
        // z given with 5 places rounds to 4 (0.95435 → 0.9544, away from zero).
        let kwh = gas_m3_to_kwh_hs_rounded(
            Decimal::ONE,
            Decimal::from_str_exact("10.0005").unwrap(), // → 10.001 (3 dp)
            Decimal::from_str_exact("0.95435").unwrap(), // → 0.9544 (4 dp)
            G685Rounding::PUBLISHED_PRACTICE,
        );
        assert_eq!(
            kwh,
            Some(
                Decimal::from_str_exact("10.001").unwrap()
                    * Decimal::from_str_exact("0.9544").unwrap()
            )
        );
    }
}
