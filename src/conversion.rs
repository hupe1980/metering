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
//! - **§25 Nr. 7 MessEV**: permits a value formed as a *"Produkt"* of measured
//!   values, which is what V × Z × Hs is.
//! - **DVGW G 685**: the anerkannte Regel der Technik referenced by §25 Nr. 4.
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
//! - `Zustandszahl` — volume conversion factor (dimensionless, typically 0.95–1.05)
//!   accounting for pressure and temperature at the meter. OBIS `7-0:52.0.22`.
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

use rust_decimal::Decimal;

use crate::interval::MeasurementUnit;

/// Parameters for Gas m³ → kWh_Hs conversion.
#[derive(Debug, Clone)]
pub struct GasConversionParams {
    /// Superior calorific value (Brennwert Ho / Hs) in kWh/m³.
    ///
    /// Published monthly by the gas distributor per supply area.
    /// Source: Messstellenbetreiber / NB monthly data per supply area.
    pub hs_kwh_per_m3: Decimal,
    /// Volume conversion factor (Zustandszahl, dimensionless).
    ///
    /// Accounts for pressure and temperature at the meter.
    /// Neutral default when not separately metered: 1.0.
    pub zustandszahl: Decimal,
}

impl GasConversionParams {
    /// Default conversion parameters when no measurement data is available.
    ///
    /// Uses `Hs = 10.55 kWh/m³` (typical German Erdgas H average) and
    /// `Zustandszahl = 1.0` (neutral).
    #[must_use]
    pub fn default_erdgas_h() -> Self {
        Self {
            hs_kwh_per_m3: Decimal::from_str_exact("10.55").unwrap_or(Decimal::from(10u32)),
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
/// // 100 m³ × 10.55 kWh/m³ × 0.9764 = 1029.90 kWh_Hs (rounded)
/// let kwh = gas_m3_to_kwh_hs(
///     Decimal::from(100u32),
///     Decimal::from_str_exact("10.55").unwrap(),
///     Decimal::from_str_exact("0.9764").unwrap(),
/// );
/// assert!(kwh > Decimal::from(1000u32));
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
/// then the configured final rounding. Rounding is kaufmännisch
/// (`MidpointAwayFromZero`), the German commercial rule (§ 1 Abs. 4 analog).
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

/// Normalize a raw meter reading to kWh.
///
/// Handles the three shapes an ingest path actually receives:
///
/// | Source unit | Conversion |
/// |---|---|
/// | an energy unit ([`MeasurementUnit::parse_scaled`]) | rescale — kWh, Wh, MWh, GJ, MJ and the UN/ECE Rec 20 codes |
/// | a volume unit (m³, litres, `MTQ`) | `V × Hs × z`, needing `gas` |
/// | `"kW"` / `"kvar"` — a **power**, not an energy | `P × duration_h`, needing `duration_secs` |
///
/// # Errors
///
/// Returns [`ConversionError`] rather than guessing. The previous signature
/// returned a bare `Decimal` and fell through to *"assume already kWh"* for any
/// unit it did not recognise, so `"MWh"` — which the crate's own
/// [`MeasurementUnit::parse_scaled`] reads correctly — passed through
/// unconverted and understated the reading by a factor of a thousand, silently.
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
    if matches!(trimmed.to_lowercase().as_str(), "kw" | "kvar") {
        let secs = duration_secs.ok_or(ConversionError::MissingDuration)?;
        if secs <= 0 {
            return Err(ConversionError::MissingDuration);
        }
        let hours = Decimal::from(secs) / Decimal::from(3600u32);
        return Ok(value * hours);
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

    #[test]
    fn default_erdgas_h_params() {
        let p = GasConversionParams::default_erdgas_h();
        assert_eq!(p.hs_kwh_per_m3, dec!(10.55));
        assert_eq!(p.zustandszahl, Decimal::ONE);
    }
}

// ── Warm water → heat energy (HeizkostenV §9 Abs. 2) ─────────────────────────

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
            q *= Decimal::from_str_exact("1.11").unwrap_or(Decimal::ONE);
        }
        if self.eigenstaendige_gewerbliche_waermelieferung {
            q /= Decimal::from_str_exact("1.15").unwrap_or(Decimal::ONE);
        }
        if self.monovalente_waermepumpe {
            q *= Decimal::from_str_exact("0.30").unwrap_or(Decimal::ONE);
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
    let factor = Decimal::from_str_exact("2.5").unwrap_or(Decimal::from(2u32));
    let cold_inlet = Decimal::from(10u32);
    adjustments.apply(factor * volume_m3 * (mean_temp_c - cold_inlet))
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
    adjustments.apply(Decimal::from(32u32) * flaeche_m2)
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
