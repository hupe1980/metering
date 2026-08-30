//! Gas standard load profiles — the SigLinDe/TUM profile arithmetic.
//!
//! ## Legal basis
//!
//! Unlike the 2025 electricity profiles, whose value tables are licensed, the
//! gas SLP procedure is **published in full**: the BDEW/VKU/GEODE Leitfaden
//! *"Abwicklung von Standardlastprofilen Gas"*, Anlage zur
//! Kooperationsvereinbarung Gas — current edition **KoV XV, Stand
//! 27.03.2026**, coefficients in Anlage 6 — prints the profile function, the
//! temperature weighting, the weekday factors and every coefficient set. The
//! formulas below are unchanged since the SigLinDe profiles were introduced in
//! 2015. Historically the duty to apply standard load profiles below
//! 1.5 million kWh/a stood in § 24 GasNZV, repealed with effect from the end
//! of 31.12.2025; the procedure itself continues under the KoV Leitfaden and
//! the BNetzA Festlegungen.
//!
//! ## The profile function
//!
//! Every gas SLP is a **daily** profile: one function value per Gastag, driven
//! by temperature. The unified SigLinDe form is a sigmoid plus two straight
//! lines, quoted verbatim from the Leitfaden:
//!
//! ```text
//! f_sigmoid(ϑ) = A / (1 + (B / (ϑ − ϑ₀))^C) + D          ϑ₀ = 40 °C
//! f_linear(ϑ)  = max{ mH·ϑ + bH ;  mW·ϑ + bW }
//! h(ϑ)         = f_sigmoid(ϑ) + f_linear(ϑ)
//! ```
//!
//! The `H` line is the Heizgas share, the `W` line the warm-water share, and
//! the older pure-sigmoid TUM profiles are the special case `mH = bH = mW =
//! bW = 0` — so one function covers both generations
//! ([`SigLinDe::pure_sigmoid`]).
//!
//! The daily quantity is then
//!
//! ```text
//! Q(D) = KW · h(ϑ_D) · F_WT
//! ```
//!
//! with the **Kundenwert** `KW` — the customer's consumption on a day where
//! `h = 1` — and the weekday factor `F_WT` ([`WeekdayFactors`]).
//!
//! ## The allocation temperature is a geometric series
//!
//! To model the heat stored in buildings, the temperature entering `h` is not
//! the day's forecast alone but a weighted mean over four days
//! ([`allocation_temperature`]):
//!
//! ```text
//! ϑ_allok = (ϑ_D + 0.5·ϑ_D₋₁ + 0.25·ϑ_D₋₂ + 0.125·ϑ_D₋₃) / 1.875
//! ```
//!
//! ## What is deliberately not here
//!
//! - **Coefficient tables for every profile.** The Leitfaden publishes them —
//!   15 profile types (HEF, HMF, HKO, eleven Gewerbe sector types and the GHD
//!   Summenlastprofil) in two variants each, `33` and `34`, which differ in
//!   how much of the demand the linear part carries. Operators load the set
//!   they balance with. One published reference set, [`SigLinDe::DE_HEF34`],
//!   is embedded so the implementation can be verified against the printed
//!   numbers.
//! - **The analytical procedure's decomposition step.** Splitting a metered
//!   Restlast across delivery points needs the whole network's data and is a
//!   settlement-system concern; the synthetic arithmetic here is also what the
//!   analytical procedure uses for its Basismengen.
//! - **Korrektur-/Optimierungsfaktoren**, which are operator-specific
//!   parameters applied on top of `Q(D)`.
//!
//! ## Example
//!
//! ```rust
//! use metering::gas_slp::{SigLinDe, WeekdayFactors, allocation_temperature};
//! use metering::{gas_daily_quantity, kundenwert};
//! use rust_decimal::dec;
//!
//! // Allocation temperature for tomorrow from four daily means.
//! let theta = allocation_temperature(dec!(5.0), dec!(2.5), dec!(2.5), dec!(5.0));
//! assert_eq!(theta, dec!(4)); // (8·5 + 4·2.5 + 2·2.5 + 5) / 15
//!
//! // The published single-family-home profile, normalised to h(8 °C) = 1.
//! let h = SigLinDe::DE_HEF34.h_value(theta);
//!
//! // A Kundenwert of 60.3423 kWh/day and no weekday factor (households
//! // carry none) give the day's allocation quantity.
//! let q = gas_daily_quantity(dec!(60.3423), h, dec!(1));
//! assert!(q > dec!(90), "a 4 °C day draws well above the h = 1 level");
//! # let _ = kundenwert(dec!(1), dec!(1));
//! ```

use rust_decimal::Decimal;
use time::{Date, Weekday};

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use crate::holiday::{Bundesland, Holiday};

// ── SigLinDe ──────────────────────────────────────────────────────────────────

/// A gas SLP profile function — sigmoid plus linear parts (SigLinDe).
///
/// Coefficients are `f64`, like [`crate::load_profile::Dynamization`]: the
/// sigmoid needs a non-integer power, which decimal arithmetic cannot express.
/// The function *value* crosses back into [`Decimal`] through
/// [`h_value`](Self::h_value), rounded once — nothing downstream touches a
/// float.
///
/// The Leitfaden publishes coefficients with 7 decimal places; supply them as
/// printed.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct SigLinDe {
    /// Sigmoid amplitude `A`.
    pub a: f64,
    /// Sigmoid slope parameter `B` (negative in every published set).
    pub b: f64,
    /// Sigmoid exponent `C`.
    pub c: f64,
    /// Sigmoid offset `D` — the temperature-independent base share.
    pub d: f64,
    /// Reference temperature `ϑ₀` in °C — 40.0 in every published set.
    pub theta0: f64,
    /// Heizgas line slope `mH`.
    pub m_h: f64,
    /// Heizgas line intercept `bH` (value at 0 °C).
    pub b_h: f64,
    /// Warm-water line slope `mW`.
    pub m_w: f64,
    /// Warm-water line intercept `bW`.
    pub b_w: f64,
}

impl SigLinDe {
    /// Decimal places of a profile function value.
    ///
    /// The Leitfaden's worked tables print h-values with six decimals; the
    /// coefficients themselves carry seven.
    pub const H_VALUE_DP: u32 = 6;

    /// The published **DE_HEF34** coefficient set — SigLinDe,
    /// Einfamilienhaushalt (`HEF`), bundesweit (`DE`), variant `34`.
    ///
    /// The trailing two digits are the **variant**, not a Bundesland code: the
    /// Leitfaden publishes each of the fifteen profiles as `33` and `34`,
    /// which differ in how much of the demand the linear part carries. Quoted
    /// from Anlage 6, where the same row states `h(8 °C) = 1.00000` — the
    /// normalisation this module's tests verify. Households carry no weekday
    /// dependence, so its factors are all `1.0000`.
    ///
    /// No EDI code is asserted: the UTILMD code for a delivery point's profile
    /// depends on the Klasse and Ausprägung the Netzbetreiber assigns
    /// (EDI@Energy *Codeliste TUM- und BDEW-SLP Gas*), which is master data
    /// this crate does not hold.
    pub const DE_HEF34: Self = Self {
        a: 1.381_966_3,
        b: -37.412_415_5,
        c: 6.172_317_9,
        d: 0.039_628_4,
        theta0: 40.0,
        m_h: -0.067_215_9,
        b_h: 1.116_713_8,
        m_w: -0.001_998_2,
        b_w: 0.135_507_0,
    };

    /// A pure sigmoid profile — no linear parts, and the standard reference
    /// temperature of 40 °C.
    ///
    /// This is the pre-2015 TUM form, and also how the Leitfaden publishes
    /// **HKO**, the Kochgasprofil: a connection serving only a cooker or an
    /// instantaneous water heater has no space-heating and no storage draw, so
    /// `mH = bH = mW = bW = 0` and the curve is nearly flat in temperature.
    #[must_use]
    pub const fn pure_sigmoid(a: f64, b: f64, c: f64, d: f64) -> Self {
        Self {
            a,
            b,
            c,
            d,
            theta0: 40.0,
            m_h: 0.0,
            b_h: 0.0,
            m_w: 0.0,
            b_w: 0.0,
        }
    }

    /// The raw profile function value `h(ϑ)` for a temperature in °C.
    ///
    /// Floored at zero: `h` is a consumption share, and the linear parts can
    /// dip below zero at temperatures far outside the fitted range.
    ///
    /// At and above `ϑ₀` the sigmoid takes its limit `D`: the term
    /// `(B/(ϑ−ϑ₀))^C` grows without bound as `ϑ` approaches `ϑ₀` from below
    /// (`B` is negative), and for `ϑ > ϑ₀` the fractional power of a negative
    /// base is undefined — the published profiles are fitted for temperatures
    /// well below 40 °C.
    #[must_use]
    pub fn h(&self, theta_c: f64) -> f64 {
        let sigmoid = if theta_c < self.theta0 {
            let ratio = self.b / (theta_c - self.theta0);
            if ratio > 0.0 && ratio.is_finite() {
                self.a / (1.0 + ratio.powf(self.c)) + self.d
            } else {
                self.d
            }
        } else {
            self.d
        };
        let linear = (self.m_h * theta_c + self.b_h).max(self.m_w * theta_c + self.b_w);
        (sigmoid + linear).max(0.0)
    }

    /// [`h`](Self::h) as a [`Decimal`], rounded to [`H_VALUE_DP`] places —
    /// the form that enters the exact `Q = KW · h · F` arithmetic.
    ///
    /// [`H_VALUE_DP`]: Self::H_VALUE_DP
    #[must_use]
    pub fn h_value(&self, theta_c: Decimal) -> Decimal {
        use rust_decimal::prelude::ToPrimitive as _;
        let theta = theta_c.to_f64().unwrap_or(0.0);
        Decimal::try_from(self.h(theta))
            .unwrap_or(Decimal::ZERO)
            .round_dp(Self::H_VALUE_DP)
    }
}

// ── allocation temperature ────────────────────────────────────────────────────

/// The allocation temperature for the Gastag `D` — a geometric series over
/// four daily mean temperatures, quoted from the Leitfaden:
///
/// ```text
/// ϑ_allok = (ϑ_D + 0.5·ϑ_D₋₁ + 0.25·ϑ_D₋₂ + 0.125·ϑ_D₋₃) / 1.875
/// ```
///
/// `t_d` is the forecast for the delivery day, `t_d1`–`t_d3` the three days
/// before it. The **weights** are exact — they are eighths, so the formula is
/// `(8ϑ_D + 4ϑ_D₋₁ + 2ϑ_D₋₂ + ϑ_D₋₃) / 15` in integers and no float is
/// involved. The **division** is not: `8/15` does not terminate, so for most
/// inputs the result is a `Decimal` rounded once to its full width, and
/// `ϑ_allok × 15` does not recover the numerator digit for digit.
///
/// It is left at that width rather than cut to a fixed number of places,
/// because nothing downstream can tell: the only consumer is
/// [`SigLinDe::h_value`], which crosses into `f64` immediately and rounds to
/// [`SigLinDe::H_VALUE_DP`]. Round it yourself before printing or storing one —
/// see the crate-level **What "exact" means here**.
///
/// Daily means themselves are formed over the **Gastag** where the operator
/// balances on gas days — see [`crate::calendar::gas_day_start_utc`].
#[must_use]
pub fn allocation_temperature(
    t_d: Decimal,
    t_d1: Decimal,
    t_d2: Decimal,
    t_d3: Decimal,
) -> Decimal {
    let eight = Decimal::from(8u32);
    let four = Decimal::from(4u32);
    let two = Decimal::from(2u32);
    (eight * t_d + four * t_d1 + two * t_d2 + t_d3) / Decimal::from(15u32)
}

// ── weekday factors ───────────────────────────────────────────────────────────

/// Weekday factors `F_WT` for a Gewerbe profile, Monday through Sunday.
///
/// The Leitfaden gives them with four decimal places and requires the
/// standard week to sum to **7.0000** exactly — [`new`](Self::new) enforces
/// it. Household profiles (HEF/HMF/HKO) carry no weekday dependence:
/// [`NONE`](Self::NONE). The GHD Summenlastprofil does carry factors, but
/// close to `1` — they are a weighted mean over the eleven sector profiles,
/// whose working-week shapes largely cancel.
///
/// ## Feiertage take the Sunday factor
///
/// TU München's investigations found no separate holiday behaviour, so the
/// Leitfaden's recommendation is to treat a gesetzlicher Feiertag like a
/// Sunday, using the **bundesweit einheitliche** holidays. That is what
/// [`for_date`](Self::for_date) does with `land = None`; passing a
/// [`Bundesland`] applies that Land's full calendar instead, which the
/// Leitfaden also permits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct WeekdayFactors {
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::decimal_array"))]
    factors: [Decimal; 7],
}

impl WeekdayFactors {
    /// No weekday dependence — every factor `1`, as the household profiles
    /// are published.
    pub const NONE: Self = Self {
        factors: [Decimal::ONE; 7],
    };

    /// Factors for Monday through Sunday.
    ///
    /// `None` unless they sum to exactly 7 — the Leitfaden's consistency rule
    /// for the standard week. A set that fails it would silently rescale
    /// every week's allocation.
    #[must_use]
    pub fn new(factors: [Decimal; 7]) -> Option<Self> {
        let sum: Decimal = factors.iter().copied().sum();
        (sum == Decimal::from(7u32)).then_some(Self { factors })
    }

    /// The factor for a plain weekday, ignoring holidays.
    #[must_use]
    pub fn factor(&self, weekday: Weekday) -> Decimal {
        self.factors[weekday.number_days_from_monday() as usize]
    }

    /// The factor for a calendar date, with Feiertage taking the Sunday
    /// factor.
    ///
    /// `land = None` uses the nationwide holidays, the Leitfaden's
    /// recommendation; `Some(land)` uses that Land's statutory calendar.
    #[must_use]
    pub fn for_date(&self, date: Date, land: Option<Bundesland>) -> Decimal {
        let is_holiday = match land {
            Some(land) => land.is_holiday(date),
            None => Holiday::on(date).any(Holiday::is_nationwide),
        };
        if is_holiday {
            self.factor(Weekday::Sunday)
        } else {
            self.factor(date.weekday())
        }
    }
}

// ── Kundenwert and the daily quantity ────────────────────────────────────────

/// Decimal places of a Kundenwert, per the Leitfaden's worked examples
/// (`KW = 60.3423 kWh`).
pub const KUNDENWERT_DP: u32 = 4;

/// The Kundenwert — the customer's daily consumption at `h = 1`.
///
/// Computed from a metered consumption over a reference period and the sum of
/// `h(ϑ_i) · F_WT,i` over the same days:
///
/// ```text
/// KW = Q_measured / Σ (h(ϑ_i) · F_WT(i))
/// ```
///
/// Rounded to [`KUNDENWERT_DP`] places. `None` when the divisor is not
/// positive — a reference period whose profile sum is zero has no information
/// in it. The Leitfaden uses `KW = 1.0000 kWh` as the placeholder for a
/// delivery point with no usable history; that is a caller's policy choice,
/// not a fallback this function invents.
#[must_use]
pub fn kundenwert(measured_kwh: Decimal, h_times_f_sum: Decimal) -> Option<Decimal> {
    if h_times_f_sum <= Decimal::ZERO {
        return None;
    }
    Some((measured_kwh / h_times_f_sum).round_dp(KUNDENWERT_DP))
}

/// The synthetic daily quantity `Q(D) = KW · h(ϑ_D) · F_WT` in kWh.
///
/// Exact decimal arithmetic end to end; `h` enters through
/// [`SigLinDe::h_value`], which is where the one float-to-decimal crossing
/// happens.
#[must_use]
pub fn gas_daily_quantity(kundenwert: Decimal, h: Decimal, weekday_factor: Decimal) -> Decimal {
    kundenwert * h * weekday_factor
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal::dec;
    use time::macros::date;

    /// The published normalisation: the DE_HEF34 datasheet states
    /// `h(8 °C) = 1.00000` (at F_WT = 1). Reproducing it verifies A, B, C, D,
    /// ϑ₀ and both linear parts against the printed coefficients.
    #[test]
    fn de_hef34_reproduces_the_published_normalisation() {
        let h = SigLinDe::DE_HEF34.h(8.0);
        assert!(
            (h - 1.0).abs() < 5e-5,
            "h(8 °C) must be 1.00000 per the datasheet, got {h}"
        );
        assert_eq!(SigLinDe::DE_HEF34.h_value(dec!(8)).round_dp(4), dec!(1));
    }

    /// Gas draw falls monotonically with temperature through the heating
    /// range, and the warm-water line keeps summer demand above zero.
    #[test]
    fn the_profile_falls_with_temperature_but_not_to_zero() {
        let p = SigLinDe::DE_HEF34;
        let mut last = f64::INFINITY;
        for theta in [-15.0, -10.0, -5.0, 0.0, 5.0, 10.0, 15.0, 20.0, 25.0] {
            let h = p.h(theta);
            assert!(h < last, "h must fall with rising temperature at {theta}");
            assert!(h > 0.0, "h must stay positive at {theta}");
            last = h;
        }
        // Summer draw is the warm-water share — far below the winter level.
        assert!(p.h(25.0) < 0.2, "got {}", p.h(25.0));
        assert!(p.h(-15.0) > 2.0, "got {}", p.h(-15.0));
    }

    /// At and above ϑ₀ the sigmoid takes its limit D rather than a NaN from a
    /// fractional power of a negative base.
    #[test]
    fn temperatures_at_and_above_the_reference_are_defined() {
        let p = SigLinDe::DE_HEF34;
        for theta in [39.999, 40.0, 45.0] {
            let h = p.h(theta);
            assert!(h.is_finite(), "h({theta}) must be finite, got {h}");
            assert!(h >= 0.0);
        }
        // Far outside the fitted range the floor holds.
        assert_eq!(p.h(500.0), 0.0);
    }

    /// A pure sigmoid is the special case with zero linear parts — the TUM
    /// generation of profiles.
    #[test]
    fn a_pure_sigmoid_has_no_linear_share() {
        let tum = SigLinDe::pure_sigmoid(3.055_384_2, -36.965_006_5, 7.225_694_7, 0.044_841_6);
        // Warm side: only D remains once the sigmoid term vanishes.
        assert!((tum.h(35.0) - tum.d).abs() < 1e-3);
        // Cold side: sigmoid saturates towards A + D.
        assert!((tum.h(-35.0) - (tum.a + tum.d)).abs() < 0.05);
    }

    /// The Leitfaden's geometric series, and the exactness of its weights: they
    /// are eighths over 1.875, so no rounding is involved.
    #[test]
    fn the_allocation_temperature_is_the_geometric_series() {
        // Constant weather is a fixed point.
        assert_eq!(
            allocation_temperature(dec!(5), dec!(5), dec!(5), dec!(5)),
            dec!(5)
        );
        // (8·5 + 4·2.5 + 2·2.5 + 5) / 15 = 60 / 15 = 4 exactly.
        assert_eq!(
            allocation_temperature(dec!(5), dec!(2.5), dec!(2.5), dec!(5)),
            dec!(4)
        );
        // The delivery day dominates: more than half the weight is D and D−1.
        let cold_snap = allocation_temperature(dec!(-10), dec!(0), dec!(10), dec!(10));
        assert!(cold_snap < dec!(-2), "got {cold_snap}");
    }

    #[test]
    fn weekday_factors_must_sum_to_seven() {
        assert!(WeekdayFactors::new([Decimal::ONE; 7]).is_some());
        assert!(
            WeekdayFactors::new([
                dec!(1.0203),
                dec!(1.0253),
                dec!(1.0303),
                dec!(1.0253),
                dec!(1.0253),
                dec!(0.9500),
                dec!(0.9235),
            ])
            .is_some(),
            "a realistic Gewerbe set summing to 7.0000"
        );
        assert!(
            WeekdayFactors::new([dec!(1.1); 7]).is_none(),
            "7.7 is not a standard week"
        );
    }

    /// Feiertage take the Sunday factor — nationwide by default, or the
    /// Land's calendar when one is named.
    #[test]
    fn holidays_take_the_sunday_factor() {
        let factors = WeekdayFactors::new([
            dec!(1.0203),
            dec!(1.0253),
            dec!(1.0303),
            dec!(1.0253),
            dec!(1.0253),
            dec!(0.9500),
            dec!(0.9235),
        ])
        .unwrap();

        // An ordinary Thursday.
        assert_eq!(factors.for_date(date!(2026 - 06 - 11), None), dec!(1.0253));

        // Christi Himmelfahrt 2026 is a Thursday and nationwide.
        assert_eq!(factors.for_date(date!(2026 - 05 - 14), None), dec!(0.9235));

        // Fronleichnam is *not* nationwide: the default calendar leaves it a
        // Thursday, the Bavarian calendar makes it a Sunday.
        let fronleichnam = date!(2026 - 06 - 04);
        assert_eq!(factors.for_date(fronleichnam, None), dec!(1.0253));
        assert_eq!(
            factors.for_date(fronleichnam, Some(Bundesland::By)),
            dec!(0.9235)
        );

        // Households carry no weekday dependence at all.
        assert_eq!(
            WeekdayFactors::NONE.for_date(fronleichnam, Some(Bundesland::By)),
            Decimal::ONE
        );
    }

    /// KW = Q / Σ(h·F), rounded to four places like the Leitfaden's example
    /// (KW = 60.3423 kWh), and undefined for an empty reference period.
    #[test]
    fn kundenwert_follows_the_leitfaden_definition() {
        // 8 500 kWh over a reference period whose h·F sum is 140.8654.
        let kw = kundenwert(dec!(8500), dec!(140.8654)).unwrap();
        assert_eq!(kw, dec!(60.3413), "8500 / 140.8654 to four places");
        assert_eq!(kundenwert(dec!(8500), Decimal::ZERO), None);
        assert_eq!(kundenwert(dec!(8500), dec!(-1)), None);
    }

    /// Q(D) = KW · h · F — and the round trip closes: applying the Kundenwert
    /// back over the reference days reproduces the measured total.
    #[test]
    fn the_daily_quantity_reconstructs_the_reference_consumption() {
        let p = SigLinDe::DE_HEF34;
        let temps = [dec!(-2), dec!(1.5), dec!(4), dec!(9), dec!(13.5)];

        let hf_sum: Decimal = temps.iter().map(|&t| p.h_value(t)).sum();
        let measured = dec!(500);
        let kw = kundenwert(measured, hf_sum).unwrap();

        let reallocated: Decimal = temps
            .iter()
            .map(|&t| gas_daily_quantity(kw, p.h_value(t), Decimal::ONE))
            .sum();
        let error = (reallocated - measured).abs();
        assert!(
            error < dec!(0.01),
            "round trip must close to within rounding: {reallocated} vs {measured}"
        );
    }
}
