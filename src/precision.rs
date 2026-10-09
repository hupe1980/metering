//! Every rounding in the crate: decimal places and mode, in one table.
//!
//! A quantity is cut where it is formed (a non-terminating quotient, a product
//! written into a result) and nowhere else, always with a `*_DP` constant and
//! its `*_STRATEGY` from here; a bare `round_dp(` in `src/` fails
//! `tests/it/scanners/arithmetic_conventions.rs`. Where no source states a
//! mode, it is kaufmännisch — half away from zero (DIN 1333).
//!
//! | Constant | Places | Mode | Source |
//! |---|---|---|---|
//! | [`PERCENT_DP`] | 2 | half away from zero | none stated — kaufmännisch |
//! | [`BENUTZUNGSDAUER_DP`] | 2 | half away from zero | none stated — kaufmännisch |
//! | [`FORECAST_DP`] | 3 | half away from zero | none stated — kaufmännisch |
//! | [`SEASONAL_FACTOR_DP`] | 4 | half away from zero | none stated — kaufmännisch |
//! | [`FORECAST_ACCURACY_DP`] | 4 | half away from zero | none stated — kaufmännisch |
//! | [`AUSFALLARBEIT_DP`] | 3 | half away from zero | BilAReM Kap. 3 states none — kaufmännisch |
//! | [`SUBSTITUTE_DP`] | 6 | half away from zero | none stated — kaufmännisch |
//! | [`ALLOCATION_DP`] | 6 | **toward zero** | none stated — a session's pro-rata cumulative in [`split_session`](crate::allocation::session::split_session); keeps it monotone, so `Σ slot = last − first` |
//! | [`FORMULA_QUOTIENT_DP`] | 6 | **toward zero** | none stated (UTILTS AHB 1.1 / MIG 1.1e); a Z81/Z80 quotient |
//! | [`DYNAMIZATION_DP`] | 4 | half away from zero | BDEW AWH SLP Strom 2025 § 2.1 states the places ("auf vier Nachkommastellen"), not a mode |
//! | [`DYNAMIZED_VALUE_DP`] | 3 | half away from zero | same sentence: "Das Ergebnis wird auf drei Nachkommastellen gerundet" |
//! | [`KUNDENWERT_DP`] | 4 | **half to even** | Leitfaden SLP Gas (KoV XV) Anlage 5: "auf 0,0001 kWh mathematisch gerundet"; footnote 22 defines *mathematisch* as half-to-even (2,2500 ≈ 2,2) |
//! | [`ALLOCATION_TEMPERATURE_WEIGHT_DP`] | 4 | **half to even** | Leitfaden SLP Gas Anlage 5: weights "mit 4 Nachkommastellen", "T2 bis T10 mathematisch, T1 Rest zu 1,0000" |
//! | [`G685_ZUSTANDSZAHL_DP`], [`G685_BRENNWERT_DP`] | 4, 3 | half away from zero | published Netzbetreiber G 685 Merkblätter (kaufmännisch) |
//!
//! The SLP-Gas h-value is deliberately **not** rounded: Anlage 5 says *"Es
//! erfolgt nach der Berechnung keine Rundung"* (see
//! [`SigLinDe::h_value`](crate::slp::gas::SigLinDe::h_value)).

use rust_decimal::RoundingStrategy;

/// The crate's default mode where no source states one: half away from zero
/// (kaufmännisches Runden, DIN 1333).
pub const KAUFMAENNISCH: RoundingStrategy = RoundingStrategy::MidpointAwayFromZero;

/// The Leitfaden SLP Gas's *mathematisch*: half to even.
pub const MATHEMATISCH: RoundingStrategy = RoundingStrategy::MidpointNearestEven;

/// Decimal places a reported percentage is cut to (coverage, Mehr-/Mindermengen
/// share, Netzverlust share): **2**.
pub const PERCENT_DP: u32 = 2;
/// Mode for [`PERCENT_DP`].
pub const PERCENT_STRATEGY: RoundingStrategy = KAUFMAENNISCH;

/// Decimal places of the Benutzungsdauer (`kWh ÷ kW`, hours): **2**.
pub const BENUTZUNGSDAUER_DP: u32 = 2;
/// Mode for [`BENUTZUNGSDAUER_DP`].
pub const BENUTZUNGSDAUER_STRATEGY: RoundingStrategy = KAUFMAENNISCH;

/// Decimal places of an annual forecast figure (kWh): **3**.
pub const FORECAST_DP: u32 = 3;
/// Mode for [`FORECAST_DP`].
pub const FORECAST_STRATEGY: RoundingStrategy = KAUFMAENNISCH;

/// Decimal places of a forecast seasonal factor: **4**.
pub const SEASONAL_FACTOR_DP: u32 = 4;
/// Mode for [`SEASONAL_FACTOR_DP`].
pub const SEASONAL_FACTOR_STRATEGY: RoundingStrategy = KAUFMAENNISCH;

/// Decimal places of a forecast accuracy ratio — WAPE, MASE and the
/// annual-energy error: **4**.
pub const FORECAST_ACCURACY_DP: u32 = 4;
/// Mode for [`FORECAST_ACCURACY_DP`].
pub const FORECAST_ACCURACY_STRATEGY: RoundingStrategy = KAUFMAENNISCH;

/// Decimal places of an Ausfallarbeit whose theoretical power is a quotient
/// (the fluctuating Spitzabrechnung's `KF` and `P_VZ,ist ÷ G_VZ`), in kWh:
/// **3** — a watt-hour.
pub const AUSFALLARBEIT_DP: u32 = 3;
/// Mode for [`AUSFALLARBEIT_DP`].
pub const AUSFALLARBEIT_STRATEGY: RoundingStrategy = KAUFMAENNISCH;

/// Decimal places of a substitute value written into a series: **6**.
pub const SUBSTITUTE_DP: u32 = 6;
/// Mode for [`SUBSTITUTE_DP`].
pub const SUBSTITUTE_STRATEGY: RoundingStrategy = KAUFMAENNISCH;

/// Decimal places a session's pro-rata cumulative is cut to in
/// [`split_session`](crate::allocation::session::split_session): **6**.
pub const ALLOCATION_DP: u32 = 6;
/// Mode for [`ALLOCATION_DP`]: **toward zero**, so no slot difference goes
/// negative.
pub const ALLOCATION_STRATEGY: RoundingStrategy = RoundingStrategy::ToZero;

/// Decimal places a Berechnungsformel quotient (Z81 Dividend ÷ Z80 Divisor) is
/// cut to: **6** — the only cut in [`crate::allocation::formula`].
pub const FORMULA_QUOTIENT_DP: u32 = 6;
/// Mode for [`FORMULA_QUOTIENT_DP`]: **toward zero**, so a proportional split
/// never exceeds its pool.
pub const FORMULA_QUOTIENT_STRATEGY: RoundingStrategy = RoundingStrategy::ToZero;

/// Decimal places a dynamisation **factor** is cut to: **4**.
pub const DYNAMIZATION_DP: u32 = 4;
/// Mode for [`DYNAMIZATION_DP`].
pub const DYNAMIZATION_STRATEGY: RoundingStrategy = KAUFMAENNISCH;

/// Decimal places a **dynamised profile value** is cut to: **3**.
pub const DYNAMIZED_VALUE_DP: u32 = 3;
/// Mode for [`DYNAMIZED_VALUE_DP`].
pub const DYNAMIZED_VALUE_STRATEGY: RoundingStrategy = KAUFMAENNISCH;

/// Decimal places of an SLP-Gas Kundenwert (kWh): **4**.
pub const KUNDENWERT_DP: u32 = 4;
/// Mode for [`KUNDENWERT_DP`]: the Leitfaden's *mathematisch*.
pub const KUNDENWERT_STRATEGY: RoundingStrategy = MATHEMATISCH;

/// Decimal places of an allocation-temperature weight: **4**.
pub const ALLOCATION_TEMPERATURE_WEIGHT_DP: u32 = 4;
/// Mode for [`ALLOCATION_TEMPERATURE_WEIGHT_DP`]: the Leitfaden's *mathematisch*.
pub const ALLOCATION_TEMPERATURE_WEIGHT_STRATEGY: RoundingStrategy = MATHEMATISCH;

/// Default places of the G 685 Zustandszahl: **4**.
pub const G685_ZUSTANDSZAHL_DP: u32 = 4;
/// Default places of the G 685 Abrechnungsbrennwert (kWh/m³): **3**.
pub const G685_BRENNWERT_DP: u32 = 3;
/// Mode of every G 685 cut (inputs and the final amount).
pub const G685_STRATEGY: RoundingStrategy = KAUFMAENNISCH;
