//! Netzqualität — EN 50160 voltage characteristics and the VDE-AR-N 4100
//! Unsymmetrie limit.
//!
//! Limits are those of **EN 50160:2022** (measurement per EN 61000-4-30). The
//! standard is paywalled, so nothing is quoted from it: the figures are
//! parameters with documented presets ([`En50160Limits`]).
//!
//! ## Start here
//!
//! [`assess_en50160`] for a series of [`PowerQualityInterval`]s;
//! [`PhaseApparentPower`] for the Unsymmetrieleistung.
//!
//! ## EN 50160 is statistical
//!
//! Every limit is a **share of 10-minute means over a window**, not a
//! per-sample threshold:
//!
//! | Parameter | Limit | Share | Window |
//! |---|---|---|---|
//! | Supply voltage | `Un ± 10 %` | 95 % | each week, each phase |
//! | Supply voltage | `Un + 10 % / − 15 %` | 100 % | each week, each phase |
//! | Frequency | `50 Hz ± 1 %` | 99.5 % | one year |
//! | THD of voltage | `≤ 8 %` | 95 % | each week |
//!
//! Assessed **per ISO week (Monday 00:00 Berlin) and per phase, never
//! pooled**: pooling lets healthy phases or weeks hide a bad one.
//!
//! Not assessed: voltage unbalance `u₂ = U₂ / U₁` (needs phase angles), and
//! flicker, dips, swells, interruptions and harmonics by order (need
//! waveform-level measurement).
//!
//! [`En50160Limits::LOW_VOLTAGE`] is **low voltage** (`Un = 230 V`
//! phase-to-neutral); set medium- and high-voltage bands on [`En50160Limits`].

use rust_decimal::Decimal;
use time::OffsetDateTime;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

// ── PowerQualityInterval ──────────────────────────────────────────────────────

/// One power-quality measurement interval — EN 50160 is defined on
/// **10-minute mean** values of instantaneous quantities, unlike the
/// accumulated energy of a [`MeterInterval`](crate::MeterInterval).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct PowerQualityInterval {
    /// Interval start — an instant; any offset, compared as UTC.
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::rfc3339"))]
    pub from: OffsetDateTime,
    /// Interval end — an instant; any offset, compared as UTC.
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::rfc3339"))]
    pub to: OffsetDateTime,
    /// L1 phase voltage in Volt, mean over the interval.
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::decimal_option"))]
    pub voltage_l1_v: Option<Decimal>,
    /// L2 phase voltage in Volt. `None` on a single-phase meter.
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::decimal_option"))]
    pub voltage_l2_v: Option<Decimal>,
    /// L3 phase voltage in Volt. `None` on a single-phase meter.
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::decimal_option"))]
    pub voltage_l3_v: Option<Decimal>,
    /// Grid frequency in Hz, mean over the interval. Nominal 50.00 Hz.
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::decimal_option"))]
    pub frequency_hz: Option<Decimal>,
    /// Total harmonic distortion of the voltage, in percent (harmonics up to
    /// order 40).
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::decimal_option"))]
    pub thd_voltage_pct: Option<Decimal>,
}

impl PowerQualityInterval {
    /// An interval over `[from, to)` with every measurement absent.
    ///
    /// ```rust
    /// use metering::grid::power_quality::PowerQualityInterval;
    /// use rust_decimal::dec;
    /// use time::macros::datetime;
    ///
    /// let iv = PowerQualityInterval {
    ///     voltage_l1_v: Some(dec!(231.4)),
    ///     ..PowerQualityInterval::empty(
    ///         datetime!(2026-06-01 0:00 UTC),
    ///         datetime!(2026-06-01 0:10 UTC),
    ///     )
    /// };
    /// assert!(!iv.voltage_out_of_range(dec!(230), dec!(10)));
    /// ```
    #[must_use]
    pub const fn empty(from: OffsetDateTime, to: OffsetDateTime) -> Self {
        Self {
            from,
            to,
            voltage_l1_v: None,
            voltage_l2_v: None,
            voltage_l3_v: None,
            frequency_hz: None,
            thd_voltage_pct: None,
        }
    }

    /// The voltage of one phase, when measured.
    #[must_use]
    pub const fn voltage(&self, phase: Phase) -> Option<Decimal> {
        match phase {
            Phase::L1 => self.voltage_l1_v,
            Phase::L2 => self.voltage_l2_v,
            Phase::L3 => self.voltage_l3_v,
        }
    }

    /// `true` when **any** measured phase deviates from `nominal_v` by more
    /// than `threshold_pct` — a per-interval triage **indicator**, not an
    /// EN 50160 verdict.
    ///
    /// `|v − Un| × 100 > threshold × Un`, with no division. `false` for a
    /// non-positive `Un` or on overflow.
    #[must_use]
    pub fn voltage_out_of_range(&self, nominal_v: Decimal, threshold_pct: Decimal) -> bool {
        if nominal_v <= Decimal::ZERO {
            return false;
        }
        let Some(limit) = threshold_pct.checked_mul(nominal_v) else {
            return false;
        };
        Phase::ALL.iter().filter_map(|p| self.voltage(*p)).any(|v| {
            v.checked_sub(nominal_v)
                .and_then(|d| d.abs().checked_mul(Decimal::ONE_HUNDRED))
                .is_some_and(|d| d > limit)
        })
    }
}

// ── En50160Limits ─────────────────────────────────────────────────────────────

/// The EN 50160 limits to assess against; preset
/// [`LOW_VOLTAGE`](Self::LOW_VOLTAGE), no `Default`.
///
/// Shares are in **permille** (`950` = 95 %), tested exactly as
/// `within × 1000 ≥ samples × permille`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[non_exhaustive]
pub struct En50160Limits {
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::decimal"))]
    nominal_voltage_v: Decimal,
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::decimal"))]
    voltage_band_pct: Decimal,
    voltage_share_permille: u16,
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::decimal"))]
    voltage_absolute_upper_pct: Decimal,
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::decimal"))]
    voltage_absolute_lower_pct: Decimal,
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::decimal"))]
    frequency_band_pct: Decimal,
    frequency_share_permille: u16,
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::decimal"))]
    thd_max_pct: Decimal,
    thd_share_permille: u16,
}

impl En50160Limits {
    /// EN 50160 low voltage, `Un = 230 V` phase to neutral, with the limits of
    /// the module-level table (frequency: interconnected system).
    pub const LOW_VOLTAGE: Self = Self {
        nominal_voltage_v: Decimal::from_parts(230, 0, 0, false, 0),
        voltage_band_pct: Decimal::from_parts(10, 0, 0, false, 0),
        voltage_share_permille: 950,
        voltage_absolute_upper_pct: Decimal::from_parts(10, 0, 0, false, 0),
        voltage_absolute_lower_pct: Decimal::from_parts(15, 0, 0, false, 0),
        frequency_band_pct: Decimal::from_parts(1, 0, 0, false, 0),
        frequency_share_permille: 995,
        thd_max_pct: Decimal::from_parts(8, 0, 0, false, 0),
        thd_share_permille: 950,
    };

    /// The declared supply voltage `Un`, phase to neutral.
    #[must_use]
    pub const fn nominal_voltage(mut self, volts: Decimal) -> Self {
        self.nominal_voltage_v = volts;
        self
    }

    /// The `± percent` band and the share (permille) of each phase's weekly
    /// means it must hold.
    #[must_use]
    pub const fn voltage_band(mut self, pct: Decimal, share_permille: u16) -> Self {
        self.voltage_band_pct = pct;
        self.voltage_share_permille = share_permille;
        self
    }

    /// The absolute band every mean must respect, `+upper / −lower` percent.
    #[must_use]
    pub const fn voltage_absolute(mut self, upper_pct: Decimal, lower_pct: Decimal) -> Self {
        self.voltage_absolute_upper_pct = upper_pct;
        self.voltage_absolute_lower_pct = lower_pct;
        self
    }

    /// The `± percent` band around 50 Hz and its share (permille).
    #[must_use]
    pub const fn frequency_band(mut self, pct: Decimal, share_permille: u16) -> Self {
        self.frequency_band_pct = pct;
        self.frequency_share_permille = share_permille;
        self
    }

    /// The THD ceiling in percent and its share (permille).
    #[must_use]
    pub const fn thd(mut self, max_pct: Decimal, share_permille: u16) -> Self {
        self.thd_max_pct = max_pct;
        self.thd_share_permille = share_permille;
        self
    }

    /// `Un × pct ÷ 100`, saturating.
    fn of(base: Decimal, pct: Decimal) -> Decimal {
        base.saturating_mul(pct) / Decimal::ONE_HUNDRED
    }
}

// ── outcomes ──────────────────────────────────────────────────────────────────

/// How one parameter fared against its limit over one window.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[non_exhaustive]
pub struct LimitOutcome {
    /// Samples that carried this parameter.
    pub samples: u32,
    /// Samples inside the limit.
    pub within: u32,
    /// The share the limit requires, in permille.
    pub required_permille: u16,
    /// The sample furthest outside the limit, when there was one.
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::decimal_option"))]
    pub worst: Option<Decimal>,
}

impl LimitOutcome {
    /// `Some(true)` when `within × 1000 ≥ samples × required_permille`;
    /// **`None` when nothing was measured**.
    #[must_use]
    pub fn compliant(&self) -> Option<bool> {
        (self.samples > 0).then(|| {
            u64::from(self.within) * 1000
                >= u64::from(self.samples) * u64::from(self.required_permille)
        })
    }
}

/// The voltage outcomes of one phase in one week.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[non_exhaustive]
pub struct PhaseOutcome {
    /// The phase.
    pub phase: Phase,
    /// Inside `Un ± band` for the required share.
    pub voltage_band: LimitOutcome,
    /// Inside `Un + upper / − lower` without exception.
    pub voltage_absolute: LimitOutcome,
}

/// One ISO week of the assessment.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[non_exhaustive]
pub struct WeekOutcome {
    /// ISO week-numbering year.
    pub iso_year: i32,
    /// ISO week, 1..=53 — Monday to Sunday, Berlin local time.
    pub iso_week: u8,
    /// Seconds of the week the intervals cover.
    pub covered_secs: i64,
    /// Seconds the week lasts (167, 168 or 169 hours).
    pub week_secs: i64,
    /// One entry per phase, L1 to L3.
    pub phases: [PhaseOutcome; 3],
    /// Voltage THD at or below the maximum for the required share.
    pub thd_voltage: LimitOutcome,
}

impl WeekOutcome {
    /// `true` when the intervals cover the whole week.
    #[must_use]
    pub const fn is_complete(&self) -> bool {
        self.covered_secs >= self.week_secs
    }

    fn outcomes(&self) -> impl Iterator<Item = &LimitOutcome> {
        self.phases
            .iter()
            .flat_map(|p| [&p.voltage_band, &p.voltage_absolute])
            .chain([&self.thd_voltage])
    }
}

/// An EN 50160 verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "SCREAMING_SNAKE_CASE"))]
pub enum En50160Verdict {
    /// Every measured limit held and every week was complete.
    Compliant,
    /// A limit failed in some week, for some phase.
    NonCompliant,
    /// Nothing failed, but a week was incomplete or nothing was measured.
    Unknown,
}

/// The outcome of an EN 50160 assessment.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[non_exhaustive]
pub struct En50160Report {
    /// Each ISO week that holds an interval, in order.
    pub weeks: Vec<WeekOutcome>,
    /// Frequency inside `50 Hz ± band` over the whole series (EN 50160: a year).
    pub frequency: LimitOutcome,
}

impl En50160Report {
    /// [`NonCompliant`](En50160Verdict::NonCompliant) on any failed limit;
    /// otherwise [`Unknown`](En50160Verdict::Unknown) for an incomplete week or
    /// an empty series.
    #[must_use]
    pub fn verdict(&self) -> En50160Verdict {
        let all = || {
            self.weeks
                .iter()
                .flat_map(WeekOutcome::outcomes)
                .chain([&self.frequency])
        };
        if all().any(|o| o.compliant() == Some(false)) {
            return En50160Verdict::NonCompliant;
        }
        let measured = all().any(|o| o.samples > 0);
        if !measured || self.weeks.iter().any(|w| !w.is_complete()) {
            return En50160Verdict::Unknown;
        }
        En50160Verdict::Compliant
    }
}

// ── assess_en50160 ────────────────────────────────────────────────────────────

/// Assess 10-minute means against EN 50160, **per ISO week and per phase**.
///
/// Each interval belongs to the ISO week of its start on the Berlin wall
/// clock; within a week each phase is its own population. Intervals missing a
/// parameter do not contribute to it; a parameter never measured has no
/// verdict ([`LimitOutcome::compliant`] is `None`). Frequency is assessed over
/// the whole series.
///
/// ```rust
/// use metering::grid::power_quality::{En50160Limits, En50160Verdict, PowerQualityInterval, assess_en50160};
/// use rust_decimal::dec;
/// use time::{Duration, macros::datetime};
///
/// // ISO week 23 of 2026 (Mon 01.06. 00:00 CEST = Sun 31.05. 22:00 UTC).
/// let start = datetime!(2026-05-31 22:00 UTC);
/// let mut week: Vec<PowerQualityInterval> = (0..1008)
///     .map(|i| {
///         let from = start + Duration::minutes(i * 10);
///         PowerQualityInterval {
///             voltage_l1_v: Some(dec!(231)),
///             ..PowerQualityInterval::empty(from, from + Duration::minutes(10))
///         }
///     })
///     .collect();
/// week[500].voltage_l1_v = Some(dec!(260)); // one sample over +10 %
///
/// let report = assess_en50160(&week, &En50160Limits::LOW_VOLTAGE);
/// let l1 = &report.weeks[0].phases[0];
/// assert_eq!(l1.voltage_band.compliant(), Some(true), "one in 1 008 is within 5 %");
/// assert_eq!(l1.voltage_absolute.compliant(), Some(false), "but +10 % admits none");
/// assert_eq!(report.verdict(), En50160Verdict::NonCompliant);
/// ```
#[must_use]
pub fn assess_en50160(intervals: &[PowerQualityInterval], limits: &En50160Limits) -> En50160Report {
    let un = limits.nominal_voltage_v;
    let band = En50160Limits::of(un, limits.voltage_band_pct);
    let (band_lo, band_hi) = (un.saturating_sub(band), un.saturating_add(band));
    let abs_hi = un.saturating_add(En50160Limits::of(un, limits.voltage_absolute_upper_pct));
    let abs_lo = un.saturating_sub(En50160Limits::of(un, limits.voltage_absolute_lower_pct));
    let nominal_hz = Decimal::from(50u32);
    let freq_band = En50160Limits::of(nominal_hz, limits.frequency_band_pct);
    let (freq_lo, freq_hi) = (nominal_hz - freq_band, nominal_hz.saturating_add(freq_band));

    let mut weeks: std::collections::BTreeMap<(i32, u8), WeekTally> =
        std::collections::BTreeMap::new();
    let mut frequency = Tally::new(limits.frequency_share_permille);
    for iv in intervals {
        let (iso_year, iso_week, _) = crate::time::calendar::to_berlin(iv.from).to_iso_week_date();
        let week = weeks
            .entry((iso_year, iso_week))
            .or_insert_with(|| WeekTally::new(limits));
        week.covered_secs = week
            .covered_secs
            .saturating_add((iv.to - iv.from).whole_seconds().max(0));
        for (i, phase) in Phase::ALL.iter().enumerate() {
            if let Some(v) = iv.voltage(*phase) {
                week.band[i].push(v, band_lo, band_hi);
                week.absolute[i].push(v, abs_lo, abs_hi);
            }
        }
        if let Some(thd) = iv.thd_voltage_pct {
            week.thd.push(thd, Decimal::MIN, limits.thd_max_pct);
        }
        if let Some(f) = iv.frequency_hz {
            frequency.push(f, freq_lo, freq_hi);
        }
    }

    let weeks = weeks
        .into_iter()
        .map(|((iso_year, iso_week), t)| {
            let [b1, b2, b3] = t.band;
            let [a1, a2, a3] = t.absolute;
            let phase = |phase, b: Tally, a: Tally| PhaseOutcome {
                phase,
                voltage_band: b.finish(),
                voltage_absolute: a.finish(),
            };
            WeekOutcome {
                iso_year,
                iso_week,
                covered_secs: t.covered_secs,
                week_secs: week_secs(iso_year, iso_week),
                phases: [
                    phase(Phase::L1, b1, a1),
                    phase(Phase::L2, b2, a2),
                    phase(Phase::L3, b3, a3),
                ],
                thd_voltage: t.thd.finish(),
            }
        })
        .collect();
    En50160Report {
        weeks,
        frequency: frequency.finish(),
    }
}

/// The length of an ISO week on the Berlin clock, in seconds; a full 168 hours
/// when the week lies outside the supported years.
fn week_secs(iso_year: i32, iso_week: u8) -> i64 {
    const NOMINAL: i64 = 7 * 24 * 3600;
    let monday = time::Date::from_iso_week_date(iso_year, iso_week, time::Weekday::Monday).ok();
    let next = monday.and_then(|m| m.checked_add(time::Duration::days(7)));
    match (
        monday.and_then(|d| crate::DayBoundary::Strom.day(d)),
        next.and_then(|d| crate::DayBoundary::Strom.day(d)),
    ) {
        (Some(a), Some(b)) => (b.start() - a.start()).whole_seconds(),
        _ => NOMINAL,
    }
}

struct WeekTally {
    covered_secs: i64,
    band: [Tally; 3],
    absolute: [Tally; 3],
    thd: Tally,
}

impl WeekTally {
    fn new(limits: &En50160Limits) -> Self {
        let v = || Tally::new(limits.voltage_share_permille);
        Self {
            covered_secs: 0,
            band: [v(), v(), v()],
            absolute: [Tally::new(1000), Tally::new(1000), Tally::new(1000)],
            thd: Tally::new(limits.thd_share_permille),
        }
    }
}

/// Counts samples inside `[lo, hi]` and remembers the one furthest outside.
#[derive(Clone)]
struct Tally {
    samples: u32,
    within: u32,
    required_permille: u16,
    worst: Option<(Decimal, Decimal)>,
}

impl Tally {
    const fn new(required_permille: u16) -> Self {
        Self {
            samples: 0,
            within: 0,
            required_permille,
            worst: None,
        }
    }

    fn push(&mut self, value: Decimal, lo: Decimal, hi: Decimal) {
        self.samples = self.samples.saturating_add(1);
        if value >= lo && value <= hi {
            self.within = self.within.saturating_add(1);
            return;
        }
        // Distance from its own bound, not from nominal: the band is asymmetric.
        let outside = if value > hi {
            value.saturating_sub(hi)
        } else {
            lo.saturating_sub(value)
        };
        if self.worst.is_none_or(|(d, _)| outside > d) {
            self.worst = Some((outside, value));
        }
    }

    fn finish(self) -> LimitOutcome {
        LimitOutcome {
            samples: self.samples,
            within: self.within,
            required_permille: self.required_permille,
            worst: self.worst.map(|(_, v)| v),
        }
    }
}

// ── Unsymmetrie (VDE-AR-N 4100 Abschnitt 5.5) ────────────────────────────────

/// One of the three Außenleiter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Phase {
    /// L1.
    L1,
    /// L2.
    L2,
    /// L3.
    L3,
}

impl Phase {
    /// Every Außenleiter, in order.
    pub const ALL: [Self; 3] = [Self::L1, Self::L2, Self::L3];

    /// Stable DB/wire label; the `serde` tag and [`FromStr`](std::str::FromStr) input.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::L1 => "L1",
            Self::L2 => "L2",
            Self::L3 => "L3",
        }
    }
}

crate::ids::codes::string_codes! {
    Phase;
}

/// The Unsymmetrieleistung VDE-AR-N 4100 Abschnitt 5.5.2 limits: **4,6 kVA**.
///
/// VDE FNN Hinweis *Symmetrischer Anschluss und Betrieb in Kundenanlagen*:
/// *"zur Einhaltung dieser Symmetriegrenze der
/// Versorgungsspannung wurde bei einem Außenleiterstrom von 20 A ein
/// Leistungsgrenzwert von 4,6 kVA festgelegt"*. The default of
/// [`PhaseApparentPower::within_limit`]; the Hinweis says the value *"soll im
/// Rahmen einer FNN-Studie untersucht werden"*, so pass your own if it changes.
pub const UNSYMMETRIE_LIMIT_KVA: Decimal = Decimal::from_parts(46, 0, 0, false, 1);

/// Apparent power per Außenleiter, in kVA.
///
/// The limit is in **Scheinleistung**: at cos φ < 1 a device carries more kVA
/// than kW, so pass kVA, not kW.
///
/// Only devices that feed in or store count (Abschnitt 5.5.2, per the VDE FNN
/// Hinweis): *"Die Anforderungen zum
/// symmetrischen Betrieb gelten nur für Geräte die elektrische Energie
/// einspeisen oder speichern können, also Erzeugungsanlagen, Speicher,
/// Ladeeinrichtungen für Elektrofahrzeuge."* Sum those per Außenleiter — not
/// the grid meter's reading, which includes household load. A three-phase
/// device feeds symmetrically ([`symmetric`](Self::symmetric)).
///
/// ```rust
/// use metering::grid::power_quality::{PhaseApparentPower, Phase};
/// use rust_decimal::dec;
///
/// // A single-phase 3,7 kVA wallbox on L1, and nothing else.
/// let one = PhaseApparentPower::single_phase(Phase::L1, dec!(3.7));
/// assert_eq!(one.unbalance_kva(), dec!(3.7));
/// assert!(one.within_limit(None), "3,7 kVA is inside the 4,6 kVA limit");
///
/// // Three 4,6 kVA units, one per Außenleiter: 13,8 kVA installed, balanced.
/// let spread = PhaseApparentPower::default()
///     .plus(Phase::L1, dec!(4.6))
///     .and_then(|p| p.plus(Phase::L2, dec!(4.6)))
///     .and_then(|p| p.plus(Phase::L3, dec!(4.6)))
///     .expect("no overflow");
/// assert_eq!(spread.unbalance_kva(), dec!(0.0));
/// assert!(spread.within_limit(None));
///
/// // A 22 kVA wallbox charging single-phase at 7,2 kVA is not.
/// let single = PhaseApparentPower::single_phase(Phase::L1, dec!(7.2));
/// assert!(!single.within_limit(None));
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct PhaseApparentPower {
    /// Apparent power on L1, kVA.
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::decimal"))]
    pub l1_kva: Decimal,
    /// Apparent power on L2, kVA.
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::decimal"))]
    pub l2_kva: Decimal,
    /// Apparent power on L3, kVA.
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::decimal"))]
    pub l3_kva: Decimal,
}

impl PhaseApparentPower {
    /// One single-phase device of `kva` on `phase`.
    #[must_use]
    pub fn single_phase(phase: Phase, kva: Decimal) -> Self {
        let mut this = Self::default();
        *this.get_mut(phase) = kva;
        this
    }

    /// A three-phase device feeding `total_kva` symmetrically.
    ///
    /// Contributes `total ÷ 3` to each Außenleiter and **nothing** to the
    /// unbalance.
    #[must_use]
    pub fn symmetric(total_kva: Decimal) -> Self {
        let each = total_kva / Decimal::from(3u32);
        Self {
            l1_kva: each,
            l2_kva: each,
            l3_kva: each,
        }
    }

    /// This plus `kva` on `phase` (builder style); `None` on overflow.
    #[must_use]
    pub fn plus(mut self, phase: Phase, kva: Decimal) -> Option<Self> {
        let slot = self.get_mut(phase);
        *slot = slot.checked_add(kva)?;
        Some(self)
    }

    /// The apparent power on one Außenleiter.
    #[must_use]
    pub const fn get(&self, phase: Phase) -> Decimal {
        match phase {
            Phase::L1 => self.l1_kva,
            Phase::L2 => self.l2_kva,
            Phase::L3 => self.l3_kva,
        }
    }

    fn get_mut(&mut self, phase: Phase) -> &mut Decimal {
        match phase {
            Phase::L1 => &mut self.l1_kva,
            Phase::L2 => &mut self.l2_kva,
            Phase::L3 => &mut self.l3_kva,
        }
    }

    /// The **Unsymmetrieleistung**: the largest difference between any two
    /// Außenleiter, in kVA.
    /// As the VDE FNN Hinweis's worked examples apply it.
    #[must_use]
    pub fn unbalance_kva(&self) -> Decimal {
        let max = self.l1_kva.max(self.l2_kva).max(self.l3_kva);
        let min = self.l1_kva.min(self.l2_kva).min(self.l3_kva);
        max - min
    }

    /// `true` when the Unsymmetrieleistung is at or below the limit.
    ///
    /// `None` uses [`UNSYMMETRIE_LIMIT_KVA`]. Inclusive.
    #[must_use]
    pub fn within_limit(&self, limit_kva: Option<Decimal>) -> bool {
        self.unbalance_kva() <= limit_kva.unwrap_or(UNSYMMETRIE_LIMIT_KVA)
    }

    /// The kVA by which the Unsymmetrieleistung exceeds the limit; zero when
    /// within. `None` uses [`UNSYMMETRIE_LIMIT_KVA`].
    #[must_use]
    pub fn excess_kva(&self, limit_kva: Option<Decimal>) -> Decimal {
        (self.unbalance_kva() - limit_kva.unwrap_or(UNSYMMETRIE_LIMIT_KVA)).max(Decimal::ZERO)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal::dec;
    use time::{Duration, macros::datetime};

    /// Monday 2026-06-01 00:00 CEST — the start of ISO week 23.
    const WEEK_23: OffsetDateTime = datetime!(2026-05-31 22:00 UTC);

    fn at(start: OffsetDateTime, i: i64) -> PowerQualityInterval {
        let from = start + Duration::minutes(i * 10);
        PowerQualityInterval::empty(from, from + Duration::minutes(10))
    }

    /// A week of three-phase 10-minute means at nominal-ish voltage.
    fn week(start: OffsetDateTime) -> Vec<PowerQualityInterval> {
        (0..1008)
            .map(|i| PowerQualityInterval {
                voltage_l1_v: Some(dec!(231)),
                voltage_l2_v: Some(dec!(229)),
                voltage_l3_v: Some(dec!(232)),
                ..at(start, i)
            })
            .collect()
    }

    const LV: En50160Limits = En50160Limits::LOW_VOLTAGE;

    #[test]
    fn a_nominal_week_is_compliant() {
        let report = assess_en50160(&week(WEEK_23), &LV);
        assert_eq!(report.weeks.len(), 1);
        let w = &report.weeks[0];
        assert_eq!((w.iso_year, w.iso_week), (2026, 23));
        assert!(w.is_complete());
        for p in &w.phases {
            assert_eq!(p.voltage_band.samples, 1008);
            assert_eq!(p.voltage_band.compliant(), Some(true));
        }
        assert_eq!(report.verdict(), En50160Verdict::Compliant);
    }

    /// One phase in band 88 % of the week, the others 100 %: pooled that is
    /// 96 % and would pass; per phase, L2 and the week fail.
    #[test]
    fn one_phase_out_of_band_fails_that_phase_and_that_week() {
        let mut series = week(WEEK_23);
        // 121 of 1008 outside: 887 inside = 87.996 %.
        for iv in series.iter_mut().take(121) {
            iv.voltage_l2_v = Some(dec!(256));
        }
        let report = assess_en50160(&series, &LV);
        let w = &report.weeks[0];
        assert_eq!(w.phases[0].voltage_band.compliant(), Some(true));
        assert_eq!(w.phases[1].voltage_band.compliant(), Some(false));
        assert_eq!(w.phases[1].phase, Phase::L2);
        assert_eq!(w.phases[2].voltage_band.compliant(), Some(true));
        assert_eq!(report.verdict(), En50160Verdict::NonCompliant);

        let (within, samples) = w.phases.iter().fold((0u64, 0u64), |(a, b), p| {
            (
                a + u64::from(p.voltage_band.within),
                b + u64::from(p.voltage_band.samples),
            )
        });
        assert!(within * 1000 >= samples * 950, "pooled, it would pass");
    }

    /// A good week does not hide a bad one.
    #[test]
    fn each_iso_week_is_assessed_on_its_own() {
        let mut series = week(WEEK_23);
        let mut second = week(WEEK_23 + Duration::days(7));
        for iv in second.iter_mut().take(60) {
            iv.voltage_l1_v = Some(dec!(256));
        }
        series.extend(second);
        let report = assess_en50160(&series, &LV);
        assert_eq!(report.weeks.len(), 2);
        assert_eq!(
            report.weeks[0].phases[0].voltage_band.compliant(),
            Some(true)
        );
        assert_eq!(report.weeks[1].iso_week, 24);
        assert_eq!(
            report.weeks[1].phases[0].voltage_band.compliant(),
            Some(false)
        );
        assert_eq!(report.verdict(), En50160Verdict::NonCompliant);
        // Pooled over both weeks, 60 of 2016 is 2.98 % — inside the 5 %.
        let (within, samples) = report.weeks.iter().fold((0u64, 0u64), |(a, b), w| {
            (
                a + u64::from(w.phases[0].voltage_band.within),
                b + u64::from(w.phases[0].voltage_band.samples),
            )
        });
        assert!(within * 1000 >= samples * 950, "pooled, it would pass");
    }

    /// The exact boundary in integers: 95 % of 1 008 is 957,6, so 958 inside
    /// conforms and 957 does not.
    #[test]
    fn the_ninety_five_percent_boundary_is_exact() {
        let build = |outside: usize| {
            let mut s = week(WEEK_23);
            for iv in s.iter_mut().take(outside) {
                iv.voltage_l1_v = Some(dec!(260));
            }
            assess_en50160(&s, &LV).weeks[0].phases[0]
                .voltage_band
                .compliant()
        };
        assert_eq!(build(50), Some(true));
        assert_eq!(build(51), Some(false));
    }

    /// `worst` is the sample furthest outside its own bound: +10 % / −15 % of
    /// 230 V is 253 / 195,5 V, so 260 V (7 V over) outranks 195,4 V (0,1 V
    /// under), though the latter is further from nominal.
    #[test]
    fn the_absolute_band_is_asymmetric_and_worst_is_measured_from_its_bound() {
        let series = [
            PowerQualityInterval {
                voltage_l1_v: Some(dec!(195.4)),
                ..at(WEEK_23, 0)
            },
            PowerQualityInterval {
                voltage_l1_v: Some(dec!(260)),
                ..at(WEEK_23, 1)
            },
            PowerQualityInterval {
                voltage_l1_v: Some(dec!(200)),
                ..at(WEEK_23, 2)
            },
        ];
        let w = &assess_en50160(&series, &LV).weeks[0];
        assert_eq!(
            w.phases[0].voltage_absolute.within, 1,
            "200 V is inside −15 %"
        );
        assert_eq!(w.phases[0].voltage_absolute.worst, Some(dec!(260)));
    }

    /// An empty series is no verdict, not a compliant one; a partial week is
    /// an indication, not a statement.
    #[test]
    fn empty_and_partial_series_are_unknown() {
        let empty = assess_en50160(&[], &LV);
        assert!(empty.weeks.is_empty());
        assert_eq!(empty.frequency.compliant(), None);
        assert_eq!(empty.verdict(), En50160Verdict::Unknown);

        let day: Vec<_> = week(WEEK_23).into_iter().take(144).collect();
        let report = assess_en50160(&day, &LV);
        assert!(!report.weeks[0].is_complete());
        assert_eq!(report.verdict(), En50160Verdict::Unknown);
    }

    /// A single-phase meter has no L2/L3 verdict, not a passing one.
    #[test]
    fn unmeasured_phases_have_no_verdict() {
        let series: Vec<_> = (0..1008)
            .map(|i| PowerQualityInterval {
                voltage_l1_v: Some(dec!(231)),
                ..at(WEEK_23, i)
            })
            .collect();
        let report = assess_en50160(&series, &LV);
        let w = &report.weeks[0];
        assert_eq!(w.phases[1].voltage_band.compliant(), None);
        assert_eq!(w.thd_voltage.compliant(), None);
        assert_eq!(report.verdict(), En50160Verdict::Compliant);
    }

    /// The week across the autumn change is 169 hours; covering 168 of them
    /// is not the whole week.
    #[test]
    fn a_dst_week_is_measured_on_the_berlin_clock() {
        // ISO week 43 of 2026: Mon 19.10. 00:00 CEST = Sun 18.10. 22:00 UTC.
        let start = datetime!(2026-10-18 22:00 UTC);
        let report = assess_en50160(&week(start), &LV);
        let w = &report.weeks[0];
        assert_eq!(w.week_secs, 169 * 3600);
        assert!(!w.is_complete());
    }

    #[test]
    fn frequency_and_thd_have_their_own_shares() {
        let mut series: Vec<_> = (0..1008)
            .map(|i| PowerQualityInterval {
                frequency_hz: Some(dec!(50.0)),
                thd_voltage_pct: Some(dec!(3.0)),
                ..at(WEEK_23, i)
            })
            .collect();
        for iv in series.iter_mut().take(6) {
            iv.frequency_hz = Some(dec!(48.0));
            iv.thd_voltage_pct = Some(dec!(12.0));
        }
        let report = assess_en50160(&series, &LV);
        // 99,5 %: six of 1 008 (0,60 %) fail the frequency…
        assert_eq!(report.frequency.compliant(), Some(false));
        assert_eq!(report.frequency.worst, Some(dec!(48.0)));
        // …and pass the 95 % THD test.
        assert_eq!(report.weeks[0].thd_voltage.compliant(), Some(true));
    }

    #[test]
    fn limits_are_configurable() {
        let strict = LV.voltage_band(dec!(5), 950);
        let mut series = week(WEEK_23);
        for iv in series.iter_mut().take(200) {
            iv.voltage_l1_v = Some(dec!(245)); // +6,5 %
        }
        let band = |l: &En50160Limits| {
            assess_en50160(&series, l).weeks[0].phases[0]
                .voltage_band
                .compliant()
        };
        assert_eq!(band(&LV), Some(true));
        assert_eq!(band(&strict), Some(false));
    }

    #[test]
    fn the_triage_indicator_flags_single_samples() {
        let iv = PowerQualityInterval {
            voltage_l3_v: Some(dec!(254)),
            ..at(WEEK_23, 0)
        };
        assert!(iv.voltage_out_of_range(dec!(230), dec!(10)));
        assert!(!at(WEEK_23, 0).voltage_out_of_range(dec!(230), dec!(10)));
        assert!(!iv.voltage_out_of_range(Decimal::ZERO, dec!(10)));
    }
}

#[cfg(test)]
mod unsymmetrie_tests {
    use super::*;
    use rust_decimal::dec;

    /// The three worked examples of the VDE FNN Hinweis.
    #[test]
    fn the_published_examples_resolve_as_printed() {
        let one = PhaseApparentPower::single_phase(Phase::L1, dec!(3.7));
        assert_eq!(one.unbalance_kva(), dec!(3.7));
        assert!(one.within_limit(None));
        assert_eq!(one.excess_kva(None), dec!(0.0));

        let two = PhaseApparentPower::single_phase(Phase::L2, dec!(7.2));
        assert_eq!(two.unbalance_kva(), dec!(7.2));
        assert!(!two.within_limit(None));
        assert_eq!(two.excess_kva(None), dec!(2.6));

        let spread = PhaseApparentPower::default()
            .plus(Phase::L1, dec!(4.6))
            .and_then(|p| p.plus(Phase::L2, dec!(4.6)))
            .and_then(|p| p.plus(Phase::L3, dec!(4.6)))
            .unwrap();
        assert_eq!(
            PhaseApparentPower::single_phase(Phase::L1, Decimal::MAX).plus(Phase::L1, dec!(1)),
            None,
            "overflow is reported, not a panic"
        );
        assert_eq!(spread.unbalance_kva(), dec!(0.0));
        assert!(spread.within_limit(None));
    }

    #[test]
    fn a_symmetric_three_phase_device_is_never_an_unbalance() {
        for total in [dec!(3), dec!(11), dec!(30), dec!(300)] {
            let s = PhaseApparentPower::symmetric(total);
            assert_eq!(s.unbalance_kva(), Decimal::ZERO, "{total} kVA");
        }
        let mixed = PhaseApparentPower::symmetric(dec!(30))
            .plus(Phase::L3, dec!(5))
            .unwrap();
        assert_eq!(mixed.unbalance_kva(), dec!(5));
        assert!(!mixed.within_limit(None));
    }

    #[test]
    fn the_limit_is_inclusive_and_overridable() {
        assert!(PhaseApparentPower::single_phase(Phase::L1, dec!(4.6)).within_limit(None));
        let over = PhaseApparentPower::single_phase(Phase::L1, dec!(4.7));
        assert!(!over.within_limit(None));
        assert!(over.within_limit(Some(dec!(5.0))));
    }

    #[test]
    fn unbalance_is_a_spread_not_a_phase() {
        for phase in Phase::ALL {
            let p = PhaseApparentPower::single_phase(phase, dec!(6));
            assert_eq!(p.unbalance_kva(), dec!(6), "{phase}");
        }
    }

    /// The rule counts apparent power: at cos φ = 0,9 a device draws 10 % more
    /// kVA than kW.
    #[test]
    fn apparent_power_is_not_active_power() {
        let by_kw = PhaseApparentPower::single_phase(Phase::L1, dec!(4.5));
        let by_kva = PhaseApparentPower::single_phase(Phase::L1, dec!(4.5) / dec!(0.9));
        assert!(by_kw.within_limit(None));
        assert!(!by_kva.within_limit(None));
    }
}
