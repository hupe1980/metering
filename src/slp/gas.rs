//! Gas standard load profiles — the SigLinDe/TUM profile arithmetic.
//!
//! Source: BDEW/VKU/GEODE Leitfaden *"Abwicklung von Standardlastprofilen Gas"*,
//! Anlage zur Kooperationsvereinbarung Gas, **KoV XV, Stand 27.03.2026**
//! (coefficients in Anlage 6).
//!
//! Every gas SLP is a **daily** profile, one value per Gastag, driven by
//! temperature:
//!
//! ```text
//! f_sigmoid(ϑ) = A / (1 + (B / (ϑ − ϑ₀))^C) + D          ϑ₀ = 40 °C
//! f_linear(ϑ)  = max{ mH·ϑ + bH ;  mW·ϑ + bW }
//! h(ϑ)         = f_sigmoid(ϑ) + f_linear(ϑ)
//! Q(D)         = KW · h(ϑ_allok) · F_WT
//! ```
//!
//! `H` is the Heizgas line, `W` the warm-water line; the pure-sigmoid TUM
//! profiles are the case `mH = bH = mW = bW = 0`. `KW` is the [`kundenwert`],
//! `F_WT` the weekday factor ([`WeekdayFactors`]) and `ϑ_allok` the
//! four-day [`allocation_temperature`].
//!
//! Coefficient sets are loaded by the operator; [`SigLinDe::DE_HEF34`] is
//! embedded as a verified reference. Out of scope: the analytical procedure's
//! Restlast decomposition and operator-specific Korrektur-/Optimierungsfaktoren.
//!
//! ## Example
//!
//! ```rust
//! use metering::slp::gas::{SigLinDe, allocation_temperature, gas_daily_quantity};
//! use rust_decimal::dec;
//!
//! // The Leitfaden's worked example (§ 4.1.2): −0,2399 °C.
//! let theta = allocation_temperature(dec!(-2.0), dec!(0.5), dec!(3.4), dec!(3.6)).unwrap();
//! assert_eq!(theta, dec!(-0.23991));
//!
//! // The published single-family-home profile, normalised to h(8 °C) = 1.
//! let h = SigLinDe::DE_HEF34.h_value(dec!(4)).unwrap();
//!
//! // A Kundenwert of 60.3423 kWh/day and no weekday factor (households
//! // carry none) give the day's allocation quantity.
//! let q = gas_daily_quantity(dec!(60.3423), h, dec!(1)).unwrap();
//! assert!(q > dec!(90), "a 4 °C day draws well above the h = 1 level");
//! ```

use rust_decimal::{Decimal, dec};
use time::{Date, Weekday};

use crate::precision::{KUNDENWERT_DP, KUNDENWERT_STRATEGY};

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use crate::time::holiday::{Bundesland, Holiday};

// ── profile types ────────────────────────────────────────────────────────────

/// A gas Standardlastprofil type (Leitfaden SLP Gas Anlage 6): household
/// types, Gewerbe sector types and the GHD Summenlastprofil.
///
/// The aliases `EF` and `MF` (for HEF and HMF) are accepted when parsing and
/// never written out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum GasProfile {
    /// HEF — Haushalt, Einfamilienhaushalt.
    Hef,
    /// HMF — Haushalt, Mehrfamilienhaushalt.
    Hmf,
    /// HKO — Haushalt, Kochgas.
    Hko,
    /// GKO — Gebietskörperschaften, Kreditinstitute und Versicherungen,
    /// Organisationen ohne Erwerbszweck, öffentliche Einrichtungen.
    Gko,
    /// GHA — Einzel- und Großhandel.
    Gha,
    /// GMK — Metall und Kfz.
    Gmk,
    /// GBD — sonstige betriebliche Dienstleistungen.
    Gbd,
    /// GGA — Gaststätten.
    Gga,
    /// GBH — Beherbergung.
    Gbh,
    /// GWA — Wäschereien und chemische Reinigungen.
    Gwa,
    /// GGB — Gartenbau.
    Ggb,
    /// GBA — Backstuben.
    Gba,
    /// GPD — Papier und Druck.
    Gpd,
    /// GMF — haushaltsähnliche Gewerbebetriebe.
    Gmf,
    /// GHD — the **Summenlastprofil Gewerbe, Handel, Dienstleistung**, for a
    /// delivery point that fits none of the sector types (EDI@Energy
    /// *Codeliste TUM- und BDEW-SLP Gas* v1.1, §6.3, codes `HD3`/`HD4`).
    Ghd,
}

impl GasProfile {
    /// Every profile type, in the Leitfaden's order.
    pub const ALL: [Self; 15] = [
        Self::Hef,
        Self::Hmf,
        Self::Hko,
        Self::Gko,
        Self::Gha,
        Self::Gmk,
        Self::Gbd,
        Self::Gga,
        Self::Gbh,
        Self::Gwa,
        Self::Ggb,
        Self::Gba,
        Self::Gpd,
        Self::Gmf,
        Self::Ghd,
    ];

    /// The profile code.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Hef => "HEF",
            Self::Hmf => "HMF",
            Self::Hko => "HKO",
            Self::Gko => "GKO",
            Self::Gha => "GHA",
            Self::Gmk => "GMK",
            Self::Gbd => "GBD",
            Self::Gga => "GGA",
            Self::Gbh => "GBH",
            Self::Gwa => "GWA",
            Self::Ggb => "GGB",
            Self::Gba => "GBA",
            Self::Gpd => "GPD",
            Self::Gmf => "GMF",
            Self::Ghd => "GHD",
        }
    }

    /// `true` for the household types (HEF, HMF, HKO), which carry no weekday factors.
    #[must_use]
    pub const fn is_household(self) -> bool {
        matches!(self, Self::Hef | Self::Hmf | Self::Hko)
    }
}

crate::ids::codes::string_codes! {
    GasProfile, aliases = [("EF", Self::Hef), ("MF", Self::Hmf)];
}

// ── SigLinDe ──────────────────────────────────────────────────────────────────

/// A gas SLP profile function — sigmoid plus linear parts (SigLinDe).
///
/// Coefficients are `f64` (the sigmoid needs a non-integer power); supply
/// them as printed. The value crosses into [`Decimal`] through
/// [`h_value`](Self::h_value). A pure-sigmoid TUM profile has
/// `m_h = b_h = m_w = b_w = 0`.
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
    /// The published **DE_HEF34** coefficient set — SigLinDe,
    /// Einfamilienhaushalt (`HEF`), bundesweit (`DE`), variant `34`.
    ///
    /// The trailing digits are the variant (`33`/`34`, differing in the linear
    /// share), not a Bundesland code. Quoted from Anlage 6, which states
    /// `h(8 °C) = 1.00000` for this row. No EDI code is implied: that depends
    /// on the Klasse and Ausprägung the Netzbetreiber assigns.
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

    /// `h(ϑ)` in `f64` for a temperature in °C, unfloored.
    ///
    /// `None` at or above `ϑ₀`, where the fractional power of `B/(ϑ−ϑ₀)` is
    /// undefined.
    fn h_f64(&self, theta_c: f64) -> Option<f64> {
        if theta_c >= self.theta0 {
            return None;
        }
        let ratio = self.b / (theta_c - self.theta0);
        if !(ratio > 0.0 && ratio.is_finite()) {
            return None;
        }
        let sigmoid = self.a / (1.0 + ratio.powf(self.c)) + self.d;
        let linear = (self.m_h * theta_c + self.b_h).max(self.m_w * theta_c + self.b_w);
        let h = sigmoid + linear;
        h.is_finite().then_some(h)
    }

    /// The profile function value `h(ϑ)` for a temperature in °C, as a
    /// [`Decimal`] — the crate's one `f64 → Decimal` crossing.
    ///
    /// **Not rounded**, per Leitfaden SLP Gas (KoV XV) Anlage 5: h-values are
    /// computed *"mindestens mit einer Genauigkeit von 5 Nachkommastellen …
    /// Es erfolgt nach der Berechnung keine Rundung."* The § 4.1.2
    /// Tagesmengen reproduce only from the unrounded value.
    ///
    /// `None` (never a substitute zero) when the temperature does not convert
    /// to `f64`, is at or above `ϑ₀`, or when `h` would be negative.
    #[must_use]
    pub fn h_value(&self, theta_c: Decimal) -> Option<Decimal> {
        use rust_decimal::prelude::ToPrimitive as _;
        let h = self.h_f64(theta_c.to_f64()?)?;
        if h < 0.0 {
            return None;
        }
        Decimal::try_from(h).ok()
    }
}

// ── allocation temperature ────────────────────────────────────────────────────

/// The weights of the geometric series, as the Leitfaden applies them:
/// `8/15, 4/15, 2/15, 1/15` with four decimals, T2…T4 rounded *mathematisch*
/// ([`ALLOCATION_TEMPERATURE_WEIGHT_STRATEGY`](crate::precision::ALLOCATION_TEMPERATURE_WEIGHT_STRATEGY))
/// and T1 the remainder to 1,0000 (Leitfaden SLP Gas, Anlage 5).
pub const ALLOCATION_TEMPERATURE_WEIGHTS: [Decimal; 4] =
    [dec!(0.5333), dec!(0.2667), dec!(0.1333), dec!(0.0667)];

/// The allocation temperature for the Gastag `D` — a geometric series over
/// four daily mean temperatures with the Leitfaden's four-decimal weights
/// [`ALLOCATION_TEMPERATURE_WEIGHTS`]:
///
/// ```text
/// ϑ_allok = 0,5333·ϑ_D + 0,2667·ϑ_D₋₁ + 0,1333·ϑ_D₋₂ + 0,0667·ϑ_D₋₃
/// ```
///
/// `t_d` is the forecast for the delivery day, `t_d1`–`t_d3` the three days
/// before it, in °C (daily means over the Gastag, see
/// [`DayBoundary::Gas`](crate::DayBoundary::Gas)). Not rounded; reproduces
/// the worked example of § 4.1.2 (−0,2399 °C). `None` on overflow.
#[must_use]
pub fn allocation_temperature(
    t_d: Decimal,
    t_d1: Decimal,
    t_d2: Decimal,
    t_d3: Decimal,
) -> Option<Decimal> {
    [t_d, t_d1, t_d2, t_d3]
        .into_iter()
        .zip(ALLOCATION_TEMPERATURE_WEIGHTS)
        .try_fold(Decimal::ZERO, |sum, (t, w)| {
            sum.checked_add(t.checked_mul(w)?)
        })
}

// ── weekday factors ───────────────────────────────────────────────────────────

/// Weekday factors `F_WT` for a Gewerbe profile, Monday through Sunday.
///
/// The standard week must sum to **7.0000** exactly ([`new`](Self::new)
/// enforces it). Household profiles use [`NONE`](Self::NONE). A gesetzlicher
/// Feiertag takes the Sunday factor ([`for_date`](Self::for_date)).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct WeekdayFactors {
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::decimal_array"))]
    factors: [Decimal; 7],
}

impl WeekdayFactors {
    /// No weekday dependence — every factor `1` (household profiles).
    pub const NONE: Self = Self {
        factors: [Decimal::ONE; 7],
    };

    /// Factors for Monday through Sunday.
    ///
    /// `None` unless they sum to exactly 7.
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
    /// `land = None` uses the nationwide holidays (the Leitfaden's
    /// recommendation, Anlage 3 Tab. 24); `Some(land)` uses that Land's
    /// statutory calendar.
    #[must_use]
    pub fn for_date(&self, date: Date, land: Option<Bundesland>) -> Decimal {
        let is_holiday = match land {
            Some(land) => land.is_holiday(date),
            // No 24.12/31.12 rule for gas.
            None => Holiday::on(date)
                .any(|h| Holiday::NATIONWIDE.contains(&h) && h.is_nationwide(date.year())),
        };
        if is_holiday {
            self.factor(Weekday::Sunday)
        } else {
            self.factor(date.weekday())
        }
    }
}

// ── Kundenwert and the daily quantity ────────────────────────────────────────

/// The Kundenwert — the customer's daily consumption at `h = 1`.
///
/// ```text
/// KW = Q_measured / Σ (h(ϑ_i) · F_WT(i))      over a reference period, kWh
/// ```
///
/// Rounded to [`KUNDENWERT_DP`] places, *mathematisch* (half to even) —
/// Anlage 5: *"wird in kWh mit 4
/// Kommastellen angegeben und auf 0,0001 kWh mathematisch gerundet"*. `None`
/// when the divisor is not positive or on overflow. The Leitfaden's
/// placeholder `KW = 1.0000 kWh` for a point without history is the caller's
/// choice.
#[must_use]
pub fn kundenwert(measured_kwh: Decimal, h_times_f_sum: Decimal) -> Option<Decimal> {
    if h_times_f_sum <= Decimal::ZERO {
        return None;
    }
    Some(
        measured_kwh
            .checked_div(h_times_f_sum)?
            .round_dp_with_strategy(KUNDENWERT_DP, KUNDENWERT_STRATEGY),
    )
}

/// The synthetic daily quantity `Q(D) = KW · h(ϑ_D) · F_WT` in kWh.
///
/// `h` comes from [`SigLinDe::h_value`]. `None` on overflow.
#[must_use]
pub fn gas_daily_quantity(
    kundenwert: Decimal,
    h: Decimal,
    weekday_factor: Decimal,
) -> Option<Decimal> {
    kundenwert.checked_mul(h)?.checked_mul(weekday_factor)
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal::dec;
    use time::macros::date;

    fn h(p: &SigLinDe, theta: f64) -> f64 {
        p.h_f64(theta).expect("inside the fitted range")
    }

    /// A pure-sigmoid TUM profile: the struct with no linear parts.
    fn pure_sigmoid(a: f64, b: f64, c: f64, d: f64) -> SigLinDe {
        SigLinDe {
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

    /// The published normalisation: the DE_HEF34 datasheet states
    /// `h(8 °C) = 1.00000` (at F_WT = 1). Reproducing it verifies A, B, C, D,
    /// ϑ₀ and both linear parts against the printed coefficients.
    #[test]
    fn de_hef34_reproduces_the_published_normalisation() {
        let h = h(&SigLinDe::DE_HEF34, 8.0);
        assert!(
            (h - 1.0).abs() < 5e-5,
            "h(8 °C) must be 1.00000 per the datasheet, got {h}"
        );
        assert_eq!(
            SigLinDe::DE_HEF34
                .h_value(dec!(8))
                .unwrap()
                .round_dp_with_strategy(4, crate::precision::KAUFMAENNISCH),
            dec!(1)
        );
    }

    /// Gas draw falls monotonically with temperature through the heating
    /// range, and the warm-water line keeps summer demand above zero.
    #[test]
    fn the_profile_falls_with_temperature_but_not_to_zero() {
        let p = SigLinDe::DE_HEF34;
        let mut last = f64::INFINITY;
        for theta in [-15.0, -10.0, -5.0, 0.0, 5.0, 10.0, 15.0, 20.0, 25.0] {
            let v = h(&p, theta);
            assert!(v < last, "h must fall with rising temperature at {theta}");
            assert!(v > 0.0, "h must stay positive at {theta}");
            last = v;
        }
        assert!(h(&p, 25.0) < 0.2);
        assert!(h(&p, -15.0) > 2.0);
    }

    /// At and above ϑ₀ the sigmoid is undefined, and the answer is `None` —
    /// not its limit `D`, and not a floored zero.
    #[test]
    fn no_temperature_answers_with_a_silent_value() {
        let p = SigLinDe::DE_HEF34;
        assert!(p.h_value(dec!(39.9)).is_some());
        for theta in [dec!(40), dec!(45), dec!(500)] {
            assert_eq!(p.h_value(theta), None, "{theta}");
        }
        // A coefficient set whose linear part drives h below zero inside the
        // domain is refused there rather than floored at zero.
        let negative = SigLinDe {
            b_h: -5.0,
            b_w: -5.0,
            ..SigLinDe::DE_HEF34
        };
        assert_eq!(negative.h_value(dec!(30)), None);
    }

    /// A pure sigmoid is the special case with zero linear parts — the TUM
    /// generation of profiles.
    #[test]
    fn a_pure_sigmoid_has_no_linear_share() {
        let tum = pure_sigmoid(3.055_384_2, -36.965_006_5, 7.225_694_7, 0.044_841_6);
        assert!((h(&tum, 35.0) - tum.d).abs() < 1e-3);
        assert!((h(&tum, -35.0) - (tum.a + tum.d)).abs() < 0.05);
    }

    #[test]
    fn gas_profile_codes_round_trip_and_accept_the_old_aliases() {
        for p in GasProfile::ALL {
            assert_eq!(p.as_str().parse::<GasProfile>().ok(), Some(p));
        }
        assert_eq!("EF".parse::<GasProfile>().ok(), Some(GasProfile::Hef));
        assert_eq!(" ghd ".parse::<GasProfile>().ok(), Some(GasProfile::Ghd));
        assert_eq!(GasProfile::Hef.to_string(), "HEF");
        assert!("H0".parse::<GasProfile>().is_err(), "an electricity code");
        assert!(GasProfile::Hko.is_household());
        assert!(!GasProfile::Ghd.is_household());
    }

    /// The weights follow the Leitfaden's rule — T2…T4 = 4/15, 2/15, 1/15
    /// rounded *mathematisch* to four places, T1 the remainder to 1,0000 —
    /// and reproduce the worked example (§ 4.1.2, printed p. 43): −0,2399 °C.
    #[test]
    fn the_allocation_temperature_reproduces_the_leitfaden_example() {
        use crate::precision::{
            ALLOCATION_TEMPERATURE_WEIGHT_DP as DP, ALLOCATION_TEMPERATURE_WEIGHT_STRATEGY as S,
        };
        let tail: Vec<Decimal> = [4u32, 2, 1]
            .iter()
            .map(|n| (Decimal::from(*n) / Decimal::from(15u32)).round_dp_with_strategy(DP, S))
            .collect();
        let t1 = Decimal::ONE - tail.iter().copied().sum::<Decimal>();
        assert_eq!(
            [t1, tail[0], tail[1], tail[2]],
            ALLOCATION_TEMPERATURE_WEIGHTS
        );

        let theta = allocation_temperature(dec!(-2.0), dec!(0.5), dec!(3.4), dec!(3.6)).unwrap();
        assert_eq!(theta, dec!(-0.23991));
        assert_eq!(theta.round_dp_with_strategy(4, S), dec!(-0.2399));
        // Constant weather is a fixed point.
        assert_eq!(
            allocation_temperature(dec!(5), dec!(5), dec!(5), dec!(5)),
            Some(dec!(5))
        );
    }

    /// The Leitfaden's h-values at ϑ = −0,2 °C (PDF p. 57) and the Tagesmengen
    /// built on them, which reproduce only from the **unrounded** h.
    #[test]
    fn h_values_and_tagesmengen_reproduce_the_leitfaden() {
        let mathematisch =
            |d: Decimal, dp| d.round_dp_with_strategy(dp, crate::precision::MATHEMATISCH);
        let d14 = pure_sigmoid(3.185_019_1, -37.412_415_5, 6.172_317_9, 0.076_109_6);
        let ok4 = SigLinDe {
            a: 1.425_668_4,
            b: -36.659_050_4,
            c: 7.608_322_6,
            d: 0.037_111_6,
            theta0: 40.0,
            m_h: -0.080_935_9,
            b_h: 1.236_452_7,
            m_w: -0.000_762_8,
            b_w: 0.100_297_9,
        };
        let h_d14 = d14.h_value(dec!(-0.2)).unwrap();
        let h_4ok = ok4.h_value(dec!(-0.2)).unwrap();
        assert_eq!(mathematisch(h_d14, 5), dec!(2.01613));
        assert_eq!(mathematisch(h_4ok, 5), dec!(2.24285));
        let q1 = gas_daily_quantity(dec!(50), h_d14, dec!(1.0000)).unwrap();
        let q2 = gas_daily_quantity(dec!(400), h_4ok, dec!(1.0523)).unwrap();
        assert_eq!(mathematisch(q1, 4), dec!(100.8067));
        assert_eq!(mathematisch(q2, 4), dec!(944.0611));
    }

    /// The Leitfaden's Kundenwert examples (Anlage 1, Tab. 18–23).
    #[test]
    fn kundenwerte_reproduce_the_leitfaden() {
        assert_eq!(
            kundenwert(dec!(23185), dec!(329.5810012)),
            Some(dec!(70.3469))
        );
        assert_eq!(kundenwert(dec!(958), dec!(379.3563091)), Some(dec!(2.5253)));
        assert_eq!(
            kundenwert(dec!(223185), dec!(397.72740)),
            Some(dec!(561.1507))
        );
        // *mathematisch*: a tie rounds to even.
        assert_eq!(kundenwert(dec!(1.00005), Decimal::ONE), Some(dec!(1.0000)));
        assert_eq!(kundenwert(dec!(1.00015), Decimal::ONE), Some(dec!(1.0002)));
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

    /// KW = Q / Σ(h·F), rounded to four places; undefined for an empty
    /// reference period.
    #[test]
    fn kundenwert_follows_the_leitfaden_definition() {
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

        let hf_sum: Decimal = temps.iter().map(|&t| p.h_value(t).unwrap()).sum();
        let measured = dec!(500);
        let kw = kundenwert(measured, hf_sum).unwrap();

        let reallocated: Decimal = temps
            .iter()
            .map(|&t| gas_daily_quantity(kw, p.h_value(t).unwrap(), Decimal::ONE).unwrap())
            .sum();
        let error = (reallocated - measured).abs();
        assert!(
            error < dec!(0.01),
            "round trip must close to within rounding: {reallocated} vs {measured}"
        );
    }
}
