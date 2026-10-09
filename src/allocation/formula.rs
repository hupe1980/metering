//! The Berechnungsformel: a Marktlokation's series computed from its
//! Messlokationen, as the UTILTS message writes it.
//!
//! The formula the Netzbetreiber transmits (UTILTS AHB 1.1, MIG 1.1e,
//! Prüfidentifikator 25001); this module is the vocabulary and the evaluator,
//! not the EDIFACT parser. Start at [`Formula`] and [`Formula::eval`].
//!
//! A [`Formula`] is a set of Rechenschritte keyed by [`StepId`] plus the step
//! that yields the result. Each step is one of four kinds:
//!
//! | [`Step`] | Operators | Value per interval |
//! |---|---|---|
//! | [`Sum`](Step::Sum) | Z69 Addition / Z70 Subtraktion, any number | `Σ ±operand` |
//! | [`Quotient`](Step::Quotient) | exactly one Z81 Dividend and one Z80 Divisor | `dividend ÷ divisor`, cut once |
//! | [`Product`](Step::Product) | Z82 Faktor, any number | `Π operand` |
//! | [`Positive`](Step::Positive) | exactly one Z83 Positivwert, over a step | `max(0, step)` |
//!
//! An operand is another step or a Messlokation ([`MeloOperand`]): its
//! Z87 Energieflussrichtung ([`FlowDirection`]) selects the register — never a
//! sign — and its Verlustfaktor Trafo (Z16), Verlustfaktor Leitung (ZB2) and
//! Aufteilungsfaktor (ZG6) multiply the register value **before** the step's
//! operation. MIG 1.1e, CAV 00033: *"Auf die Messwerte der Messlokation sind
//! erst der Verlustfaktor des Transformators und der Verlustfaktor der
//! Leitung, jeweils multiplikativ anzuwenden."*
//!
//! # Arithmetic
//!
//! Exact `Decimal` arithmetic per interval, with one cut: a Z81/Z80 quotient
//! is cut to [`FORMULA_QUOTIENT_DP`] places toward zero. A zero divisor gives
//! **0** for that interval, the business rule of the
//! BDEW Anwendungshilfe *Beispiele von Berechnungsformeln für das Solarpaket 1*
//! (Beispiel 3): *"Ist die Energiemenge einer Marktlokation zugeordneten
//! Messlokation = 0, so ist auch der Verbrauch der Marktlokation auf 0 zu
//! setzen."* Every interval where that rule applied is reported in the
//! [`Evaluation`].
//!
//! # Validation
//!
//! [`Formula::new`] refuses a missing or self-referencing step, a cycle, an
//! empty step, more than 99999 components, and — a crate rule the AHB does not
//! state — a step nothing references (it catches the orphaned step 4 in the
//! Anwendungshilfe's Beispiel 1, MaLo 1). [`Formula::from_components`] takes
//! the wire shape and refuses a step that mixes operator kinds.
//!
//! ```rust
//! use metering::allocation::formula::{Formula, SplitFactor};
//! use metering::{DayBoundary, Direction, MeloId, MeterInterval, QualityFlag, Resolution, Series};
//! use rust_decimal::dec;
//! use time::macros::datetime;
//!
//! let plant: MeloId = "DE0001234567890000000000000000001".parse()?;
//! let tenant: MeloId = "DE0001234567890000000000000000002".parse()?;
//! let one = |kwh| -> Result<Series, Box<dyn std::error::Error>> {
//!     let iv = MeterInterval::quarter_hour(datetime!(2026-06-01 12:00 UTC), kwh, QualityFlag::Measured)?;
//!     Ok(Series::new(Resolution::QUARTER_HOUR, DayBoundary::Strom, vec![iv])?)
//! };
//! let (generation, consumption) = (one(dec!(100))?, one(dec!(20))?);
//!
//! // AWH Beispiel 1: Malo2 Verbrauch = Pos(Melo2 Verbrauch − 10 % Melo1 Erzeugung).
//! let formula = Formula::constant_share(tenant, plant, SplitFactor::new(dec!(0.1))?)?;
//! let out = formula.eval(|melo, direction| match (*melo == plant, direction) {
//!     (true, Direction::Export) => Some(&generation),
//!     (false, Direction::Import) => Some(&consumption),
//!     _ => None,
//! })?;
//! assert_eq!(out.series.as_slice()[0].value(), dec!(10.0));
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::num::NonZeroU32;

use rust_decimal::Decimal;
use time::OffsetDateTime;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use crate::ids::MeloId;
use crate::precision::{FORMULA_QUOTIENT_DP, FORMULA_QUOTIENT_STRATEGY};
use crate::series::interval::{Direction, IntervalError, MeterInterval, QualityFlag};
use crate::series::{Series, SeriesError};

// ── codes ─────────────────────────────────────────────────────────────────────

/// The mathematical operator of one component (SG9 `CCI+++Z86`, CAV DE7111);
/// [`Formula::from_components`] groups components into a [`Step`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Operator {
    /// Z69 Addition.
    Addition,
    /// Z70 Subtraktion.
    Subtraktion,
    /// Z80 Divisor — the denominator of the step's quotient.
    Divisor,
    /// Z81 Dividend — the numerator of the step's quotient.
    Dividend,
    /// Z82 Faktor — one factor of a multiplication.
    Faktor,
    /// Z83 Positivwert — `max(0, x)` of the referenced step.
    Positivwert,
}

impl Operator {
    /// Every operator, in declaration order.
    pub const ALL: [Self; 6] = [
        Self::Addition,
        Self::Subtraktion,
        Self::Divisor,
        Self::Dividend,
        Self::Faktor,
        Self::Positivwert,
    ];

    /// The AHB code: `Z69`, `Z70`, `Z80`, `Z81`, `Z82`, `Z83`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Addition => "Z69",
            Self::Subtraktion => "Z70",
            Self::Divisor => "Z80",
            Self::Dividend => "Z81",
            Self::Faktor => "Z82",
            Self::Positivwert => "Z83",
        }
    }
}

/// Z87 Energieflussrichtung (CAV DE7111): which register of the Messlokation
/// the component reads — a register, never a sign.
///
/// MIG 1.1e, CAV 00035: the direction says whether
/// the measured energy *"zum Netz fließt (Erzeugung) oder vom Netz wegfließt
/// (Verbrauch)"*. [`direction`](Self::direction) maps it onto the crate's
/// [`Direction`], which is what [`Formula::eval`] asks the caller for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FlowDirection {
    /// Z71 Verbrauch — the Bezug register.
    Verbrauch,
    /// Z72 Erzeugung — the Einspeisung register.
    Erzeugung,
}

impl FlowDirection {
    /// Both directions, in declaration order.
    pub const ALL: [Self; 2] = [Self::Verbrauch, Self::Erzeugung];

    /// The AHB code: `Z71` or `Z72`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Verbrauch => "Z71",
            Self::Erzeugung => "Z72",
        }
    }

    /// The register: Verbrauch is [`Import`](Direction::Import) (Bezug),
    /// Erzeugung is [`Export`](Direction::Export) (Einspeisung).
    #[must_use]
    pub const fn direction(self) -> Direction {
        match self {
            Self::Verbrauch => Direction::Import,
            Self::Erzeugung => Direction::Export,
        }
    }
}

/// The factor qualifiers of a Messlokation component (SG9 CCI DE7037).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FactorKind {
    /// Z16 Verlustfaktor Trafo (value `CAV+Z28`): a [`LossFactor`].
    VerlustfaktorTrafo,
    /// ZB2 Verlustfaktor Leitung (value `CAV+Z28`): a [`LossFactor`].
    VerlustfaktorLeitung,
    /// ZG6 Aufteilungsfaktor Energiemenge (value `CAV+ZH6`): a
    /// [`SplitFactor`].
    Aufteilungsfaktor,
}

impl FactorKind {
    /// Every qualifier, in declaration order.
    pub const ALL: [Self; 3] = [
        Self::VerlustfaktorTrafo,
        Self::VerlustfaktorLeitung,
        Self::Aufteilungsfaktor,
    ];

    /// The AHB code: `Z16`, `ZB2` or `ZG6`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::VerlustfaktorTrafo => "Z16",
            Self::VerlustfaktorLeitung => "ZB2",
            Self::Aufteilungsfaktor => "ZG6",
        }
    }
}

crate::ids::codes::string_codes! {
    Operator;
    FlowDirection;
    FactorKind;
}

// ── factors ───────────────────────────────────────────────────────────────────

/// The places a factor may carry: AHB 1.1 condition \[912\], six.
const FACTOR_DP: u32 = 6;

/// A Verlustfaktor (Trafo Z16 or Leitung ZB2): **> 0**, **≠ 1**, at most
/// **6** decimal places (AHB 1.1 conditions \[914\], \[915\], \[912\]).
///
/// Above 1 is a surcharge, below 1 a deduction; it always multiplies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LossFactor(Decimal);

impl LossFactor {
    /// Validate a loss factor.
    ///
    /// # Errors
    ///
    /// [`FormulaError::InvalidLossFactor`] for a value ≤ 0, equal to 1, or with
    /// more than 6 significant decimal places.
    pub fn new(value: Decimal) -> Result<Self, FormulaError> {
        if value > Decimal::ZERO && value != Decimal::ONE && places(value) <= FACTOR_DP {
            Ok(Self(value.normalize()))
        } else {
            Err(FormulaError::InvalidLossFactor { value })
        }
    }

    /// The factor.
    #[must_use]
    pub const fn get(self) -> Decimal {
        self.0
    }
}

/// An Aufteilungsfaktor Energiemenge (ZG6, value `CAV+ZH6`): **> 0**,
/// **≤ 1**, at most **6** decimal places (AHB 1.1 conditions \[914\], \[969\],
/// \[912\]).
///
/// Multiplies the Messlokation's value (AWH: `Melo1, Z82, Z72, ZG6 0.1` is
/// *10 % Melo1 Erzeugung*); also the § 42b constant key of
/// [`AllocationKey::Constant`](crate::allocation::community::AllocationKey::Constant).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SplitFactor(Decimal);

impl SplitFactor {
    /// Validate a split factor.
    ///
    /// # Errors
    ///
    /// [`FormulaError::InvalidSplitFactor`] for a value ≤ 0, above 1, or with more
    /// than 6 significant decimal places.
    pub fn new(value: Decimal) -> Result<Self, FormulaError> {
        if value > Decimal::ZERO && value <= Decimal::ONE && places(value) <= FACTOR_DP {
            Ok(Self(value.normalize()))
        } else {
            Err(FormulaError::InvalidSplitFactor { value })
        }
    }

    /// The factor.
    #[must_use]
    pub const fn get(self) -> Decimal {
        self.0
    }
}

/// Significant decimal places: `1.040000` has two.
fn places(value: Decimal) -> u32 {
    value.normalize().scale()
}

#[cfg(feature = "serde")]
macro_rules! factor_serde {
    ($ty:ty) => {
        impl Serialize for $ty {
            fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                crate::wire::decimal::serialize(&self.0, serializer)
            }
        }

        impl<'de> Deserialize<'de> for $ty {
            fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                let value = crate::wire::decimal::deserialize(deserializer)?;
                Self::new(value).map_err(serde::de::Error::custom)
            }
        }
    };
}

#[cfg(feature = "serde")]
factor_serde!(LossFactor);
#[cfg(feature = "serde")]
factor_serde!(SplitFactor);

// ── operands and steps ────────────────────────────────────────────────────────

/// A Rechenschrittidentifikator (SEQ+Z37 DE1050): 1 to 99999 (AHB 1.1
/// condition \[913\]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(try_from = "u32", into = "u32"))]
pub struct StepId(NonZeroU32);

impl StepId {
    /// The largest identifier the AHB admits.
    pub const MAX: u32 = 99_999;

    /// Validate an identifier.
    ///
    /// # Errors
    ///
    /// [`FormulaError::InvalidStepId`] for 0 or anything above [`MAX`](Self::MAX).
    pub fn new(id: u32) -> Result<Self, FormulaError> {
        NonZeroU32::new(id)
            .filter(|n| n.get() <= Self::MAX)
            .map(Self)
            .ok_or(FormulaError::InvalidStepId { id })
    }

    /// The identifier.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0.get()
    }
}

impl TryFrom<u32> for StepId {
    type Error = FormulaError;

    fn try_from(id: u32) -> Result<Self, Self::Error> {
        Self::new(id)
    }
}

impl From<StepId> for u32 {
    fn from(id: StepId) -> Self {
        id.get()
    }
}

impl fmt::Display for StepId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// A component that references a Messlokation (`RFF+Z19`): the register it
/// reads and the factors applied to it before the step's operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct MeloOperand {
    /// The Messlokation.
    pub melo: MeloId,
    /// Z87 — which register; mandatory on every MeLo component.
    pub direction: FlowDirection,
    /// Z16 Verlustfaktor Trafo, if transmitted.
    pub loss_trafo: Option<LossFactor>,
    /// ZB2 Verlustfaktor Leitung, if transmitted.
    pub loss_line: Option<LossFactor>,
    /// ZG6 Aufteilungsfaktor Energiemenge, if transmitted.
    pub split: Option<SplitFactor>,
}

impl MeloOperand {
    /// The `direction` register of `melo`, with no factor.
    #[must_use]
    pub const fn new(melo: MeloId, direction: FlowDirection) -> Self {
        Self {
            melo,
            direction,
            loss_trafo: None,
            loss_line: None,
            split: None,
        }
    }

    /// With a Verlustfaktor Trafo (builder style).
    #[must_use]
    pub const fn trafo(mut self, factor: LossFactor) -> Self {
        self.loss_trafo = Some(factor);
        self
    }

    /// With a Verlustfaktor Leitung (builder style).
    #[must_use]
    pub const fn line(mut self, factor: LossFactor) -> Self {
        self.loss_line = Some(factor);
        self
    }

    /// With an Aufteilungsfaktor (builder style).
    #[must_use]
    pub const fn split(mut self, factor: SplitFactor) -> Self {
        self.split = Some(factor);
        self
    }

    /// Add one factor qualifier as the wire carries it: a [`FactorKind`] code
    /// and its value.
    ///
    /// # Errors
    ///
    /// [`FormulaError::DuplicateFactor`] when the qualifier is already set,
    /// or the factor's own refusal for a value outside its rules.
    pub fn with_factor(mut self, kind: FactorKind, value: Decimal) -> Result<Self, FormulaError> {
        let taken = match kind {
            FactorKind::VerlustfaktorTrafo => self.loss_trafo.is_some(),
            FactorKind::VerlustfaktorLeitung => self.loss_line.is_some(),
            FactorKind::Aufteilungsfaktor => self.split.is_some(),
        };
        if taken {
            return Err(FormulaError::DuplicateFactor {
                melo: self.melo,
                kind,
            });
        }
        match kind {
            FactorKind::VerlustfaktorTrafo => self.loss_trafo = Some(LossFactor::new(value)?),
            FactorKind::VerlustfaktorLeitung => self.loss_line = Some(LossFactor::new(value)?),
            FactorKind::Aufteilungsfaktor => self.split = Some(SplitFactor::new(value)?),
        }
        Ok(self)
    }

    /// `register × trafo × leitung × aufteilung`, or `None` on overflow.
    fn scale(&self, register: Decimal) -> Option<Decimal> {
        [
            self.loss_trafo.map(LossFactor::get),
            self.loss_line.map(LossFactor::get),
            self.split.map(SplitFactor::get),
        ]
        .into_iter()
        .flatten()
        .try_fold(register, Decimal::checked_mul)
    }
}

/// What a component references: a Messlokation (`RFF+Z19`) or another step
/// (`RFF+Z23`), never both.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "SCREAMING_SNAKE_CASE"))]
pub enum Operand {
    /// A Messlokation register.
    Melo(MeloOperand),
    /// The result of another step.
    Step(StepId),
}

impl From<MeloOperand> for Operand {
    fn from(melo: MeloOperand) -> Self {
        Self::Melo(melo)
    }
}

impl From<StepId> for Operand {
    fn from(step: StepId) -> Self {
        Self::Step(step)
    }
}

/// The sign a component carries inside a [`Step::Sum`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum Sign {
    /// Z69 Addition.
    #[cfg_attr(feature = "serde", serde(rename = "Z69"))]
    Plus,
    /// Z70 Subtraktion.
    #[cfg_attr(feature = "serde", serde(rename = "Z70"))]
    Minus,
}

impl Sign {
    /// The operator code this sign is written as.
    #[must_use]
    pub const fn operator(self) -> Operator {
        match self {
            Self::Plus => Operator::Addition,
            Self::Minus => Operator::Subtraktion,
        }
    }
}

/// One Rechenschritt, in one of the four kinds the MIG permits (CAV 00033
/// Bemerkung).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "SCREAMING_SNAKE_CASE"))]
pub enum Step {
    /// Z69 / Z70 components, at least one. Component order carries no meaning.
    Sum(Vec<(Sign, Operand)>),
    /// Exactly one Z81 Dividend and one Z80 Divisor.
    Quotient {
        /// Z81 — the numerator.
        dividend: Operand,
        /// Z80 — the denominator.
        divisor: Operand,
    },
    /// Z82 components, at least one.
    Product(Vec<Operand>),
    /// Z83 over the result of one step: `max(0, x)`.
    Positive(StepId),
}

impl Step {
    /// Every operand, in order.
    fn operands(&self) -> Vec<Operand> {
        match self {
            Self::Sum(terms) => terms.iter().map(|(_, o)| *o).collect(),
            Self::Quotient { dividend, divisor } => vec![*dividend, *divisor],
            Self::Product(factors) => factors.clone(),
            Self::Positive(step) => vec![Operand::Step(*step)],
        }
    }

    /// The number of SG8 components the step occupies on the wire.
    fn components(&self) -> usize {
        self.operands().len()
    }
}

/// One SG8 `SEQ+Z37` "Bestandteil des Rechenschritts" as the wire carries it:
/// the step it belongs to, its operator and its operand.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Component {
    /// The Rechenschrittidentifikator.
    pub step: StepId,
    /// The component's operator.
    pub operator: Operator,
    /// What it references.
    pub operand: Operand,
}

// ── errors ────────────────────────────────────────────────────────────────────

/// Why a formula could not be built or evaluated.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum FormulaError {
    /// A Rechenschrittidentifikator outside 1 to 99999.
    #[error("step id {id} is outside 1..=99999")]
    InvalidStepId {
        /// The rejected id.
        id: u32,
    },
    /// A Verlustfaktor that is ≤ 0, equal to 1, or has more than 6 places.
    #[error("loss factor {value} must be > 0, ≠ 1 and have at most 6 decimal places")]
    InvalidLossFactor {
        /// The rejected value.
        value: Decimal,
    },
    /// An Aufteilungsfaktor that is ≤ 0, above 1, or has more than 6 places.
    #[error("split factor {value} must be > 0, ≤ 1 and have at most 6 decimal places")]
    InvalidSplitFactor {
        /// The rejected value.
        value: Decimal,
    },
    /// A factor qualifier given twice on one component.
    #[error("{kind} is given twice on the component for {melo}")]
    DuplicateFactor {
        /// The component's Messlokation.
        melo: MeloId,
        /// The repeated qualifier.
        kind: FactorKind,
    },
    /// The result names no step of the formula.
    #[error("the result step {result} does not exist")]
    MissingResult {
        /// The named result.
        result: StepId,
    },
    /// A component references a step that does not exist.
    #[error("step {referrer} references the missing step {step}")]
    UnknownStep {
        /// The referencing step.
        referrer: StepId,
        /// The missing one.
        step: StepId,
    },
    /// A step references itself (AHB 1.1 condition \[9\]).
    #[error("step {step} references itself")]
    SelfReference {
        /// The step.
        step: StepId,
    },
    /// The steps reference each other in a cycle.
    #[error("the steps form a cycle: {cycle:?}")]
    Cycle {
        /// The cycle, starting and ending at the same step.
        cycle: Vec<StepId>,
    },
    /// A step nothing references — a crate rule; the AHB is silent.
    #[error("step {step} is not referenced by the result")]
    Unreferenced {
        /// The orphan.
        step: StepId,
    },
    /// A Sum or Product step with no component.
    #[error("step {step} has no component")]
    EmptyStep {
        /// The step.
        step: StepId,
    },
    /// More than 99999 components (MIG SG8 MaxWdh).
    #[error("the formula has {count} components, more than 99999")]
    TooManyComponents {
        /// The count.
        count: usize,
    },
    /// The components of one step match none of the four step kinds.
    #[error("step {step} combines the operators {operators:?}, which no step kind permits")]
    MixedStep {
        /// The step.
        step: StepId,
        /// Its operators, in component order.
        operators: Vec<Operator>,
    },
    /// A Z83 Positivwert over a Messlokation; the MIG applies it to the result
    /// of a referenced step.
    #[error("step {step} applies Positivwert to a Messlokation")]
    PositivwertOverMelo {
        /// The step.
        step: StepId,
    },
    /// [`Formula::proportional_share`]: the tenant is not among the
    /// participants, or one is named twice.
    #[error("{melo} is missing from, or repeated in, the participants")]
    Participants {
        /// The offending Messlokation.
        melo: MeloId,
    },
    /// The caller supplied no series for a register the formula reads.
    #[error("no series for {melo} {direction}")]
    MissingSeries {
        /// The Messlokation.
        melo: MeloId,
        /// The register.
        direction: FlowDirection,
    },
    /// A series is not on the same grid as the first one read.
    #[error(
        "the series for {melo} {direction} is on a different grid (first difference at {at:?})"
    )]
    GridMismatch {
        /// The Messlokation.
        melo: MeloId,
        /// The register.
        direction: FlowDirection,
        /// The first instant at which the grids differ; `None` when only the
        /// resolution or day boundary does.
        at: Option<OffsetDateTime>,
    },
    /// A register value below zero. MeLo values are non-negative (AWH p. 4);
    /// refusing a negative one is a crate rule.
    #[error("the {melo} {direction} value at {at} is negative")]
    NegativeValue {
        /// The Messlokation.
        melo: MeloId,
        /// The register.
        direction: FlowDirection,
        /// The interval start.
        at: OffsetDateTime,
    },
    /// A value exceeded `Decimal`'s range.
    #[error("step {step} overflows at {at}")]
    Overflow {
        /// The step.
        step: StepId,
        /// The interval start.
        at: OffsetDateTime,
    },
    /// The result series could not be formed (unreachable after the grid
    /// check).
    #[error(transparent)]
    Series(#[from] SeriesError),
    /// The result interval could not be formed (unreachable).
    #[error(transparent)]
    Interval(#[from] IntervalError),
}

// ── Formula ───────────────────────────────────────────────────────────────────

/// A validated Berechnungsformel: the Rechenschritte and the step that yields
/// the result (`SEQ+Z36 RFF+Z23`). See the [module docs](self).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(try_from = "RawFormula", into = "RawFormula"))]
pub struct Formula {
    steps: BTreeMap<StepId, Step>,
    result: StepId,
    /// Every step, each after the steps it reads.
    order: Vec<StepId>,
}

/// The unvalidated wire shape; deserialisation goes through [`Formula::new`].
#[cfg(feature = "serde")]
#[derive(Serialize, Deserialize)]
struct RawFormula {
    result: StepId,
    steps: BTreeMap<StepId, Step>,
}

#[cfg(feature = "serde")]
impl TryFrom<RawFormula> for Formula {
    type Error = FormulaError;

    fn try_from(raw: RawFormula) -> Result<Self, Self::Error> {
        Self::new(raw.result, raw.steps)
    }
}

#[cfg(feature = "serde")]
impl From<Formula> for RawFormula {
    fn from(formula: Formula) -> Self {
        Self {
            result: formula.result,
            steps: formula.steps,
        }
    }
}

/// The interval at which a zero divisor set a quotient to 0.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct ZeroDivisor {
    /// The quotient step.
    pub step: StepId,
    /// The interval start.
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::rfc3339"))]
    pub at: OffsetDateTime,
}

/// What [`Formula::eval`] returns: the MaLo series and every interval at which
/// the zero-divisor rule applied.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[non_exhaustive]
pub struct Evaluation {
    /// One interval per input interval, on the inputs' grid, with no OBIS
    /// channel and the worst quality of the inputs at that instant.
    pub series: Series,
    /// Each `(step, interval)` whose divisor was zero, ascending by instant.
    pub zero_divisors: Vec<ZeroDivisor>,
}

impl Formula {
    /// Validate a formula.
    ///
    /// # Errors
    ///
    /// A [`FormulaError`] naming the step: [`MissingResult`](FormulaError::MissingResult),
    /// [`UnknownStep`](FormulaError::UnknownStep),
    /// [`SelfReference`](FormulaError::SelfReference),
    /// [`Cycle`](FormulaError::Cycle), [`Unreferenced`](FormulaError::Unreferenced),
    /// [`EmptyStep`](FormulaError::EmptyStep) or
    /// [`TooManyComponents`](FormulaError::TooManyComponents).
    pub fn new(
        result: StepId,
        steps: impl IntoIterator<Item = (StepId, Step)>,
    ) -> Result<Self, FormulaError> {
        let steps: BTreeMap<StepId, Step> = steps.into_iter().collect();
        let count: usize = steps.values().map(Step::components).sum();
        if count > StepId::MAX as usize {
            return Err(FormulaError::TooManyComponents { count });
        }
        if !steps.contains_key(&result) {
            return Err(FormulaError::MissingResult { result });
        }
        for (&id, step) in &steps {
            let empty = match step {
                Step::Sum(terms) => terms.is_empty(),
                Step::Product(factors) => factors.is_empty(),
                Step::Quotient { .. } | Step::Positive(_) => false,
            };
            if empty {
                return Err(FormulaError::EmptyStep { step: id });
            }
            for target in references(step) {
                if target == id {
                    return Err(FormulaError::SelfReference { step: id });
                }
                if !steps.contains_key(&target) {
                    return Err(FormulaError::UnknownStep {
                        referrer: id,
                        step: target,
                    });
                }
            }
        }
        let order = topological(&steps, result)?;
        Ok(Self {
            steps,
            result,
            order,
        })
    }

    /// Build a formula from the wire's components: each [`Component`] carries
    /// its step id and operator, and the components sharing an id form one
    /// step.
    ///
    /// # Errors
    ///
    /// [`MixedStep`](FormulaError::MixedStep) for a group matching none of
    /// the four kinds (an operator beside another kind, a Z80 without its
    /// Z81, two Divisors), [`PositivwertOverMelo`](FormulaError::PositivwertOverMelo),
    /// and everything [`new`](Self::new) refuses.
    pub fn from_components(
        result: StepId,
        components: impl IntoIterator<Item = Component>,
    ) -> Result<Self, FormulaError> {
        let mut groups: BTreeMap<StepId, Vec<(Operator, Operand)>> = BTreeMap::new();
        let mut count = 0usize;
        for c in components {
            count += 1;
            groups
                .entry(c.step)
                .or_default()
                .push((c.operator, c.operand));
        }
        if count > StepId::MAX as usize {
            return Err(FormulaError::TooManyComponents { count });
        }
        let steps = groups
            .into_iter()
            .map(|(id, group)| Ok((id, step_of(id, &group)?)))
            .collect::<Result<Vec<_>, FormulaError>>()?;
        Self::new(result, steps)
    }

    /// One register, unchanged: a one-component Sum (AHB 1.1 condition \[15\]),
    /// what a status Z40 (*keine Rechenoperation*) stands for.
    ///
    /// # Errors
    ///
    /// None in practice; the signature is [`new`](Self::new)'s.
    pub fn melo(operand: MeloOperand) -> Result<Self, FormulaError> {
        Self::sum([operand])
    }

    /// The sum of several registers: Summenmessung, a portfolio total.
    ///
    /// # Errors
    ///
    /// [`EmptyStep`](FormulaError::EmptyStep) for no operand.
    pub fn sum(operands: impl IntoIterator<Item = MeloOperand>) -> Result<Self, FormulaError> {
        let one = first();
        let terms = operands
            .into_iter()
            .map(|o| (Sign::Plus, Operand::Melo(o)))
            .collect();
        Self::new(one, [(one, Step::Sum(terms))])
    }

    /// `total − Σ subtract`, signed: a residual load, or net grid exchange
    /// (`load − generation`, positive is draw).
    ///
    /// # Errors
    ///
    /// None in practice; the signature is [`new`](Self::new)'s.
    pub fn residual(
        total: MeloOperand,
        subtract: impl IntoIterator<Item = MeloOperand>,
    ) -> Result<Self, FormulaError> {
        let one = first();
        let terms = std::iter::once((Sign::Plus, Operand::Melo(total)))
            .chain(
                subtract
                    .into_iter()
                    .map(|o| (Sign::Minus, Operand::Melo(o))),
            )
            .collect();
        Self::new(one, [(one, Step::Sum(terms))])
    }

    /// § 42b constant allocation, the tenant's grid draw (AWH Beispiel 1):
    /// `Pos(tenant Verbrauch − factor × plant Erzeugung)` — step 1
    /// `Product[plant Z72 ×ZG6]`, step 2 `Sum[−1, +tenant Z71]`, step 3
    /// `Positive(2)`. Equals `net_grid_draw` of
    /// [`community::allocate`](crate::allocation::community::allocate) with
    /// [`AllocationKey::Constant`](crate::allocation::community::AllocationKey::Constant).
    ///
    /// # Errors
    ///
    /// None in practice; the signature is [`new`](Self::new)'s.
    pub fn constant_share(
        tenant: MeloId,
        plant: MeloId,
        factor: SplitFactor,
    ) -> Result<Self, FormulaError> {
        let [s1, s2, s3] = ids::<3>();
        let generation = MeloOperand::new(plant, FlowDirection::Erzeugung).split(factor);
        Self::new(
            s3,
            [
                (s1, Step::Product(vec![generation.into()])),
                (
                    s2,
                    Step::Sum(vec![
                        (Sign::Minus, s1.into()),
                        (Sign::Plus, verbrauch(tenant)),
                    ]),
                ),
                (s3, Step::Positive(s2)),
            ],
        )
    }

    /// § 42b consumption-proportional allocation, the tenant's grid draw (AWH
    /// Beispiel 3): `Pos(V − (V ÷ Σ participants V) × plant Erzeugung)`.
    ///
    /// # Errors
    ///
    /// [`Participants`](FormulaError::Participants) when `tenant` is not in
    /// `participants`, or a participant is named twice.
    pub fn proportional_share(
        tenant: MeloId,
        plant: MeloId,
        participants: &[MeloId],
    ) -> Result<Self, FormulaError> {
        let mut seen = BTreeSet::new();
        if let Some(twice) = participants.iter().find(|m| !seen.insert(**m)) {
            return Err(FormulaError::Participants { melo: *twice });
        }
        if !seen.contains(&tenant) {
            return Err(FormulaError::Participants { melo: tenant });
        }
        let [s1, s2, s3, s4, s5] = ids::<5>();
        let denominator = participants
            .iter()
            .map(|m| (Sign::Plus, verbrauch(*m)))
            .collect();
        Self::new(
            s5,
            [
                (s1, Step::Sum(denominator)),
                (
                    s2,
                    Step::Quotient {
                        dividend: verbrauch(tenant),
                        divisor: s1.into(),
                    },
                ),
                (
                    s3,
                    Step::Product(vec![
                        s2.into(),
                        MeloOperand::new(plant, FlowDirection::Erzeugung).into(),
                    ]),
                ),
                (
                    s4,
                    Step::Sum(vec![
                        (Sign::Plus, verbrauch(tenant)),
                        (Sign::Minus, s3.into()),
                    ]),
                ),
                (s5, Step::Positive(s4)),
            ],
        )
    }

    /// The steps, by id.
    #[must_use]
    pub const fn steps(&self) -> &BTreeMap<StepId, Step> {
        &self.steps
    }

    /// The step that yields the result.
    #[must_use]
    pub const fn result(&self) -> StepId {
        self.result
    }

    /// Every `(Messlokation, register)` the formula reads, once each.
    #[must_use]
    pub fn inputs(&self) -> BTreeSet<(MeloId, FlowDirection)> {
        self.steps
            .values()
            .flat_map(Step::operands)
            .filter_map(|o| match o {
                Operand::Melo(m) => Some((m.melo, m.direction)),
                Operand::Step(_) => None,
            })
            .collect()
    }

    /// Evaluate the formula per interval.
    ///
    /// `sources` returns the series of one register of one Messlokation; all
    /// must share one grid ([`resample`](crate::series::resample::resample())
    /// first where they do not).
    ///
    /// # Errors
    ///
    /// [`MissingSeries`](FormulaError::MissingSeries),
    /// [`GridMismatch`](FormulaError::GridMismatch) and
    /// [`NegativeValue`](FormulaError::NegativeValue) naming the Messlokation,
    /// and [`Overflow`](FormulaError::Overflow) naming the step and instant.
    pub fn eval<'a>(
        &self,
        sources: impl Fn(&MeloId, Direction) -> Option<&'a Series>,
    ) -> Result<Evaluation, FormulaError> {
        let keys: Vec<(MeloId, FlowDirection)> = self.inputs().into_iter().collect();
        let mut inputs: Vec<&Series> = Vec::with_capacity(keys.len());
        for &(melo, direction) in &keys {
            let series = sources(&melo, direction.direction())
                .ok_or(FormulaError::MissingSeries { melo, direction })?;
            if let Some(first) = inputs.first()
                && let Err(at) = same_grid(first, series)
            {
                return Err(FormulaError::GridMismatch {
                    melo,
                    direction,
                    at,
                });
            }
            if let Some(iv) = series.iter().find(|iv| iv.value() < Decimal::ZERO) {
                return Err(FormulaError::NegativeValue {
                    melo,
                    direction,
                    at: iv.from(),
                });
            }
            inputs.push(series);
        }
        let Some(&grid) = inputs.first() else {
            // Unreachable: every step chain ends in a Messlokation.
            return Err(FormulaError::MissingResult {
                result: self.result,
            });
        };

        let slot: BTreeMap<StepId, usize> = self
            .order
            .iter()
            .enumerate()
            .map(|(i, id)| (*id, i))
            .collect();
        let input: BTreeMap<(MeloId, FlowDirection), usize> =
            keys.iter().enumerate().map(|(i, k)| (*k, i)).collect();

        let mut out = Vec::with_capacity(grid.len());
        let mut zero_divisors = Vec::new();
        let mut values = vec![Decimal::ZERO; self.order.len()];
        for (k, reference) in grid.iter().enumerate() {
            let at = reference.from();
            let read = |operand: &Operand, values: &[Decimal]| -> Option<Decimal> {
                match operand {
                    Operand::Step(id) => values.get(*slot.get(id)?).copied(),
                    Operand::Melo(m) => {
                        let series = inputs.get(*input.get(&(m.melo, m.direction))?)?;
                        m.scale(series.as_slice().get(k)?.value())
                    }
                }
            };
            for (pos, id) in self.order.iter().enumerate() {
                let overflow = FormulaError::Overflow { step: *id, at };
                let value = match self.steps.get(id) {
                    Some(Step::Sum(terms)) => {
                        terms.iter().try_fold(Decimal::ZERO, |acc, (s, o)| {
                            let v = read(o, &values)?;
                            match s {
                                Sign::Plus => acc.checked_add(v),
                                Sign::Minus => acc.checked_sub(v),
                            }
                        })
                    }
                    Some(Step::Product(factors)) => factors
                        .iter()
                        .try_fold(Decimal::ONE, |acc, o| acc.checked_mul(read(o, &values)?)),
                    Some(Step::Quotient { dividend, divisor }) => {
                        match (read(dividend, &values), read(divisor, &values)) {
                            (Some(_), Some(d)) if d.is_zero() => {
                                zero_divisors.push(ZeroDivisor { step: *id, at });
                                Some(Decimal::ZERO)
                            }
                            (Some(n), Some(d)) => quotient(n, d),
                            _ => None,
                        }
                    }
                    Some(Step::Positive(step)) => {
                        read(&Operand::Step(*step), &values).map(|v| v.max(Decimal::ZERO))
                    }
                    None => None,
                };
                let slot_value = values.get_mut(pos).ok_or_else(|| overflow.clone())?;
                *slot_value = value.ok_or(overflow)?;
            }
            let result = values
                .get(slot.get(&self.result).copied().unwrap_or(usize::MAX))
                .copied()
                .ok_or(FormulaError::MissingResult {
                    result: self.result,
                })?;
            let quality = QualityFlag::worst_of(
                inputs
                    .iter()
                    .filter_map(|s| s.as_slice().get(k).map(MeterInterval::quality)),
            );
            out.push(MeterInterval::new(at, reference.to(), result, quality)?);
        }
        Ok(Evaluation {
            series: Series::new(grid.resolution(), grid.boundary(), out)?,
            zero_divisors,
        })
    }
}

/// `dividend ÷ divisor` cut to [`FORMULA_QUOTIENT_DP`] places toward zero,
/// exact even where `Decimal` division rounds its 28th digit up. `None` on
/// overflow or a zero divisor.
pub(crate) fn quotient(dividend: Decimal, divisor: Decimal) -> Option<Decimal> {
    let mut q = dividend
        .checked_div(divisor)?
        .round_dp_with_strategy(FORMULA_QUOTIENT_DP, FORMULA_QUOTIENT_STRATEGY);
    // Toward zero means `|q × divisor| ≤ |dividend|`; a rounded-up division
    // breaks it, and one unit back restores it.
    if q.checked_mul(divisor)?.abs() > dividend.abs() {
        let unit = Decimal::new(1, FORMULA_QUOTIENT_DP);
        q = if q.is_sign_negative() {
            q.checked_add(unit)?
        } else {
            q.checked_sub(unit)?
        };
    }
    Some(q)
}

/// `Ok` when `b` is on `a`'s grid; otherwise the first differing instant.
pub(crate) fn same_grid(a: &Series, b: &Series) -> Result<(), Option<OffsetDateTime>> {
    let span = |iv: &MeterInterval| (iv.from(), iv.to());
    if let Some((x, y)) = a
        .iter()
        .map(span)
        .zip(b.iter().map(span))
        .find(|(x, y)| x != y)
    {
        return Err(Some(x.0.min(y.0)));
    }
    if a.len() != b.len() {
        let extra = a
            .as_slice()
            .get(b.len())
            .or_else(|| b.as_slice().get(a.len()));
        return Err(extra.map(MeterInterval::from));
    }
    if a.resolution() != b.resolution() || a.boundary() != b.boundary() {
        return Err(None);
    }
    Ok(())
}

/// The steps a step reads.
fn references(step: &Step) -> Vec<StepId> {
    step.operands()
        .into_iter()
        .filter_map(|o| match o {
            Operand::Step(id) => Some(id),
            Operand::Melo(_) => None,
        })
        .collect()
}

/// Every step, each after the ones it reads; refuses a cycle and an orphan.
fn topological(
    steps: &BTreeMap<StepId, Step>,
    result: StepId,
) -> Result<Vec<StepId>, FormulaError> {
    #[derive(Clone, Copy, PartialEq)]
    enum Mark {
        Open,
        Done,
    }
    fn visit(
        id: StepId,
        steps: &BTreeMap<StepId, Step>,
        marks: &mut BTreeMap<StepId, Mark>,
        path: &mut Vec<StepId>,
        order: &mut Vec<StepId>,
    ) -> Result<(), FormulaError> {
        match marks.get(&id) {
            Some(Mark::Done) => return Ok(()),
            Some(Mark::Open) => {
                let start = path.iter().position(|s| *s == id).unwrap_or(0);
                let mut cycle = path.get(start..).unwrap_or_default().to_vec();
                cycle.push(id);
                return Err(FormulaError::Cycle { cycle });
            }
            None => {}
        }
        marks.insert(id, Mark::Open);
        path.push(id);
        for next in steps.get(&id).map(references).unwrap_or_default() {
            visit(next, steps, marks, path, order)?;
        }
        path.pop();
        marks.insert(id, Mark::Done);
        order.push(id);
        Ok(())
    }

    let mut marks = BTreeMap::new();
    let mut order = Vec::with_capacity(steps.len());
    visit(result, steps, &mut marks, &mut Vec::new(), &mut order)?;
    let reached: BTreeSet<StepId> = order.iter().copied().collect();
    // Cycles among the orphans first, so a cyclic formula is named a cycle.
    let orphans: Vec<StepId> = steps
        .keys()
        .filter(|id| !reached.contains(id))
        .copied()
        .collect();
    let mut scratch = Vec::new();
    for id in &orphans {
        visit(*id, steps, &mut marks, &mut Vec::new(), &mut scratch)?;
    }
    match orphans.first() {
        Some(step) => Err(FormulaError::Unreferenced { step: *step }),
        None => Ok(order),
    }
}

/// The step kind a wire group of `(operator, operand)` pairs forms.
fn step_of(id: StepId, group: &[(Operator, Operand)]) -> Result<Step, FormulaError> {
    use Operator as Op;
    let mixed = || FormulaError::MixedStep {
        step: id,
        operators: group.iter().map(|(op, _)| *op).collect(),
    };
    let all = |pred: fn(Op) -> bool| group.iter().all(|(op, _)| pred(*op));
    if all(|op| matches!(op, Op::Addition | Op::Subtraktion)) {
        return Ok(Step::Sum(
            group
                .iter()
                .map(|(op, o)| {
                    let sign = if *op == Op::Addition {
                        Sign::Plus
                    } else {
                        Sign::Minus
                    };
                    (sign, *o)
                })
                .collect(),
        ));
    }
    if all(|op| op == Op::Faktor) {
        return Ok(Step::Product(group.iter().map(|(_, o)| *o).collect()));
    }
    match group {
        [(Op::Positivwert, Operand::Step(step))] => Ok(Step::Positive(*step)),
        [(Op::Positivwert, Operand::Melo(_))] => {
            Err(FormulaError::PositivwertOverMelo { step: id })
        }
        [(Op::Dividend, n), (Op::Divisor, d)] | [(Op::Divisor, d), (Op::Dividend, n)] => {
            Ok(Step::Quotient {
                dividend: *n,
                divisor: *d,
            })
        }
        _ => Err(mixed()),
    }
}

/// Step 1, which always exists.
fn first() -> StepId {
    StepId(NonZeroU32::MIN)
}

/// Steps 1..=N for the named constructors.
fn ids<const N: usize>() -> [StepId; N] {
    std::array::from_fn(|i| {
        let n = u32::try_from(i).unwrap_or(0).saturating_add(1);
        StepId(NonZeroU32::new(n).unwrap_or(NonZeroU32::MIN))
    })
}

/// The Verbrauch register of `melo`, as an operand.
fn verbrauch(melo: MeloId) -> Operand {
    MeloOperand::new(melo, FlowDirection::Verbrauch).into()
}
