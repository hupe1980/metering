//! The UTILTS Berechnungsformel, evaluated against hand-built vectors for
//! every formula the BDEW Anwendungshilfe *Beispiele von Berechnungsformeln
//! für das Solarpaket 1* (Version 1.0, 25.01.2024) prints.
//!
//! **Provenance.** The BDEW Anwendungshilfe *Beispiele von Berechnungsformeln
//! für das Solarpaket 1* publishes formulas and the factors 0.1 and 0.9, but no
//! numeric series. Every input below is hand-built and every expected value is
//! **derived** from the AWH formulas — exact rationals, cross-checked so each
//! AWH "Alternative" agrees with its main formula — not a published AWH
//! result. Where a quotient does not terminate, the expected value is the one
//! the crate's single cut gives (six places toward zero, once, at the
//! quotient), and the zero-divisor row follows the AWH business rule: the
//! quotient is 0, and the evaluation reports the interval.

use metering::allocation::formula::{
    Component, FactorKind, FlowDirection, Formula, FormulaError, LossFactor, MeloOperand, Operand,
    Operator, Sign, SplitFactor, Step, StepId,
};
use metering::{DayBoundary, Direction, MeloId, MeterInterval, QualityFlag, Resolution, Series};
use rust_decimal::{Decimal, dec};
use std::collections::BTreeMap;
use time::macros::datetime;
use time::{Duration, OffsetDateTime};

const BASE: OffsetDateTime = datetime!(2026-06-01 10:00 UTC);

fn melo(n: u32) -> MeloId {
    format!("DE0001234567890000000000000{n:06}")
        .parse()
        .unwrap()
}

fn id(n: u32) -> StepId {
    StepId::new(n).unwrap()
}

fn qh(values: &[Decimal]) -> Series {
    let ivs = values
        .iter()
        .enumerate()
        .map(|(i, v)| {
            MeterInterval::quarter_hour(
                BASE + Duration::minutes(15 * i as i64),
                *v,
                QualityFlag::Measured,
            )
            .unwrap()
        })
        .collect();
    Series::new(Resolution::QUARTER_HOUR, DayBoundary::Strom, ivs).unwrap()
}

/// The registers a test supplies, keyed by MeLo and direction.
struct Registers(BTreeMap<(MeloId, Direction), Series>);

impl Registers {
    fn new(entries: impl IntoIterator<Item = (MeloId, Direction, Vec<Decimal>)>) -> Self {
        Self(
            entries
                .into_iter()
                .map(|(m, d, v)| ((m, d), qh(&v)))
                .collect(),
        )
    }

    fn eval(&self, formula: &Formula) -> Result<Vec<Decimal>, FormulaError> {
        let out = formula.eval(|m, d| self.0.get(&(*m, d)))?;
        Ok(out.series.iter().map(MeterInterval::value).collect())
    }
}

// ── an expression builder, so the AWH formulas read as the AWH writes them ────

enum E {
    M(MeloOperand),
    Sum(Vec<(Sign, E)>),
    Prod(Vec<E>),
    Div(Box<E>, Box<E>),
    Pos(Box<E>),
}

fn plus(e: E) -> (Sign, E) {
    (Sign::Plus, e)
}
fn minus(e: E) -> (Sign, E) {
    (Sign::Minus, e)
}
fn pos(e: E) -> E {
    E::Pos(Box::new(e))
}

/// Compile to steps with ascending ids; a MeLo stays an inline operand.
fn compile(e: E, steps: &mut Vec<(StepId, Step)>) -> Operand {
    let step = match e {
        E::M(m) => return Operand::Melo(m),
        E::Sum(terms) => Step::Sum(
            terms
                .into_iter()
                .map(|(s, t)| (s, compile(t, steps)))
                .collect(),
        ),
        E::Prod(factors) => Step::Product(factors.into_iter().map(|f| compile(f, steps)).collect()),
        E::Div(n, d) => Step::Quotient {
            dividend: compile(*n, steps),
            divisor: compile(*d, steps),
        },
        E::Pos(inner) => match compile(*inner, steps) {
            Operand::Step(s) => Step::Positive(s),
            m @ Operand::Melo(_) => {
                let wrap = id(u32::try_from(steps.len()).unwrap() + 1);
                steps.push((wrap, Step::Sum(vec![(Sign::Plus, m)])));
                Step::Positive(wrap)
            }
        },
    };
    let n = id(u32::try_from(steps.len()).unwrap() + 1);
    steps.push((n, step));
    Operand::Step(n)
}

fn formula(e: E) -> Formula {
    let mut steps = Vec::new();
    let Operand::Step(result) = compile(e, &mut steps) else {
        panic!("a formula's result is a step");
    };
    Formula::new(result, steps).unwrap()
}

// ── the AWH scenario: MeLo 1 generates, MeLo 2/3/4 consume ───────────────────

fn e1() -> E {
    E::M(MeloOperand::new(melo(1), FlowDirection::Erzeugung))
}
fn e1x(f: Decimal) -> E {
    E::M(MeloOperand::new(melo(1), FlowDirection::Erzeugung).split(SplitFactor::new(f).unwrap()))
}
fn v(n: u32) -> E {
    E::M(MeloOperand::new(melo(n), FlowDirection::Verbrauch))
}

/// The inputs: E1, V2, V3, V4 per row.
fn awh_inputs() -> Registers {
    let col = |c: [Decimal; 7]| c.to_vec();
    Registers::new([
        (
            melo(1),
            Direction::Export,
            col([
                dec!(100),
                dec!(100),
                dec!(100),
                dec!(0),
                dec!(100),
                dec!(40),
                dec!(12.345),
            ]),
        ),
        (
            melo(2),
            Direction::Import,
            col([
                dec!(5),
                dec!(20),
                dec!(5),
                dec!(10),
                dec!(0),
                dec!(30),
                dec!(1),
            ]),
        ),
        (
            melo(3),
            Direction::Import,
            col([
                dec!(50),
                dec!(95),
                dec!(95),
                dec!(10),
                dec!(0),
                dec!(0),
                dec!(20),
            ]),
        ),
        (
            melo(4),
            Direction::Import,
            col([
                dec!(7),
                dec!(7),
                dec!(0),
                dec!(3),
                dec!(0),
                dec!(1),
                dec!(2.5),
            ]),
        ),
    ])
}

/// B1-M2 as the AWH encodes it (§ 2.1, p. 6–7): s1 = Product[Melo1 Z72 ×ZG6 0.1];
/// s2 = Sum[−s1, +Melo2 Z71]; s3 = Positive(s2).
fn b1_m2() -> Formula {
    Formula::constant_share(melo(2), melo(1), SplitFactor::new(dec!(0.1)).unwrap()).unwrap()
}

fn b1_m3() -> Formula {
    Formula::constant_share(melo(3), melo(1), SplitFactor::new(dec!(0.9)).unwrap()).unwrap()
}

/// B1-M1 (§ 2.3, p. 10–13), steps 1 to 9 as the AWH intends them.
fn b1_m1_steps(step5_reads: u32) -> Vec<(StepId, Step)> {
    let m1 = || Operand::Melo(MeloOperand::new(melo(1), FlowDirection::Erzeugung));
    let m1x = |f| {
        Operand::Melo(
            MeloOperand::new(melo(1), FlowDirection::Erzeugung).split(SplitFactor::new(f).unwrap()),
        )
    };
    let mv = |n| Operand::Melo(MeloOperand::new(melo(n), FlowDirection::Verbrauch));
    let s = |n| Operand::Step(id(n));
    vec![
        (id(1), Step::Product(vec![m1x(dec!(0.1))])),
        (
            id(2),
            Step::Sum(vec![(Sign::Minus, s(1)), (Sign::Plus, mv(2))]),
        ),
        (id(3), Step::Positive(id(2))),
        (id(4), Step::Product(vec![m1x(dec!(0.9))])),
        (
            id(5),
            Step::Sum(vec![(Sign::Minus, s(step5_reads)), (Sign::Plus, mv(3))]),
        ),
        (id(6), Step::Positive(id(5))),
        (
            id(7),
            Step::Sum(vec![(Sign::Plus, mv(2)), (Sign::Minus, s(3))]),
        ),
        (
            id(8),
            Step::Sum(vec![(Sign::Plus, mv(3)), (Sign::Minus, s(6))]),
        ),
        (
            id(9),
            Step::Sum(vec![
                (Sign::Plus, m1()),
                (Sign::Minus, s(7)),
                (Sign::Minus, s(8)),
            ]),
        ),
    ]
}

fn decs(v: [&str; 7]) -> Vec<Decimal> {
    v.iter().map(|s| s.parse().unwrap()).collect()
}

#[test]
fn b1_m2_and_m3_constant_ten_and_ninety_percent() {
    let r = awh_inputs();
    assert_eq!(
        r.eval(&b1_m2()).unwrap(),
        decs(["0", "10", "0", "10", "0", "26", "0"])
    );
    assert_eq!(
        r.eval(&b1_m3()).unwrap(),
        decs(["0", "5", "5", "10", "0", "0", "8.8895"])
    );
}

#[test]
fn b1_m1_and_its_alternative_agree() {
    let r = awh_inputs();
    let main = Formula::new(id(9), b1_m1_steps(4)).unwrap();
    let expected = decs(["45", "0", "5", "0", "100", "36", "0.2345"]);
    assert_eq!(r.eval(&main).unwrap(), expected);

    // Alt. 1: Pos(0.1·E1 − V2) + Pos(0.9·E1 − V3).
    let alt = formula(E::Sum(vec![
        plus(pos(E::Sum(vec![plus(e1x(dec!(0.1))), minus(v(2))]))),
        plus(pos(E::Sum(vec![plus(e1x(dec!(0.9))), minus(v(3))]))),
    ]));
    assert_eq!(r.eval(&alt).unwrap(), expected);
}

/// G-4: the message as printed (lfd. 49, `RFF+Z23:1`) leaves step 4 orphaned
/// and is refused; without step 4 it would compute the B1-M1-literal column —
/// the wrong MaLo 1 the defect produces, kept as a regression guard.
#[test]
fn b1_m1_literal_is_refused_for_its_orphan_step() {
    assert_eq!(
        Formula::new(id(9), b1_m1_steps(1)),
        Err(FormulaError::Unreferenced { step: id(4) })
    );
    let without_orphan: Vec<_> = b1_m1_steps(1)
        .into_iter()
        .filter(|(s, _)| *s != id(4))
        .collect();
    let literal = Formula::new(id(9), without_orphan).unwrap();
    assert_eq!(
        awh_inputs().eval(&literal).unwrap(),
        decs(["85", "80", "85", "0", "100", "36", "10.1105"])
    );
}

/// B1-M4 / B3-M4: status Z40, one register unchanged ([15]).
#[test]
fn b1_m4_is_one_register() {
    let f = Formula::melo(MeloOperand::new(melo(4), FlowDirection::Verbrauch)).unwrap();
    assert_eq!(
        awh_inputs().eval(&f).unwrap(),
        decs(["7", "7", "0", "3", "0", "1", "2.5"])
    );
}

#[test]
fn b2_m2_and_its_alternative_agree() {
    let r = awh_inputs();
    // Pos(V2 − 0.1·E1 − (0.9·E1 − (V3 − Pos(V3 − 0.9·E1))))
    let main = formula(pos(E::Sum(vec![
        plus(v(2)),
        minus(e1x(dec!(0.1))),
        minus(E::Sum(vec![
            plus(e1x(dec!(0.9))),
            minus(E::Sum(vec![
                plus(v(3)),
                minus(pos(E::Sum(vec![plus(v(3)), minus(e1x(dec!(0.9)))]))),
            ])),
        ])),
    ])));
    // Alt. 1: Pos(V2 − 0.1·E1 − Pos(0.9·E1 − V3))
    let alt = formula(pos(E::Sum(vec![
        plus(v(2)),
        minus(e1x(dec!(0.1))),
        minus(pos(E::Sum(vec![plus(e1x(dec!(0.9))), minus(v(3))]))),
    ])));
    let expected = decs(["0", "10", "0", "10", "0", "0", "0"]);
    assert_eq!(r.eval(&main).unwrap(), expected);
    assert_eq!(r.eval(&alt).unwrap(), expected);
}

#[test]
fn b2_m1_and_both_alternatives_agree() {
    let r = awh_inputs();
    let inner_m3 = || {
        E::Sum(vec![
            plus(v(3)),
            minus(pos(E::Sum(vec![plus(v(3)), minus(e1x(dec!(0.9)))]))),
        ])
    };
    // E1 − (V2 − Pos(V2 − 0.1·E1 − Pos(0.9·E1 − V3))) − (V3 − Pos(V3 − 0.9·E1))
    let main = formula(E::Sum(vec![
        plus(e1()),
        minus(E::Sum(vec![
            plus(v(2)),
            minus(pos(E::Sum(vec![
                plus(v(2)),
                minus(e1x(dec!(0.1))),
                minus(pos(E::Sum(vec![plus(e1x(dec!(0.9))), minus(v(3))]))),
            ]))),
        ])),
        minus(inner_m3()),
    ]));
    // Alt. 1: Pos(0.1·E1 − V2) + Pos(0.9·E1 − V3 − Pos(V2 − 0.1·E1))
    let alt1 = formula(E::Sum(vec![
        plus(pos(E::Sum(vec![plus(e1x(dec!(0.1))), minus(v(2))]))),
        plus(pos(E::Sum(vec![
            plus(e1x(dec!(0.9))),
            minus(v(3)),
            minus(pos(E::Sum(vec![plus(v(2)), minus(e1x(dec!(0.1)))]))),
        ]))),
    ]));
    // Alt. 2: Pos(E1 − V2 − (V3 − Pos(V3 − 0.9·E1)))
    let alt2 = formula(pos(E::Sum(vec![
        plus(e1()),
        minus(v(2)),
        minus(inner_m3()),
    ])));
    let expected = decs(["45", "0", "5", "0", "100", "10", "0.2345"]);
    assert_eq!(r.eval(&main).unwrap(), expected);
    assert_eq!(r.eval(&alt1).unwrap(), expected);
    assert_eq!(r.eval(&alt2).unwrap(), expected);
}

/// B3 (§ 4, p. 19): the proportional key. Rows 2 and 7 are non-terminating
/// quotients, cut once at six places toward zero; row 5 has a zero divisor
/// and follows the AWH rule (p. 18) — *"Ist die Energiemenge einer
/// Marktlokation zugeordneten Messlokation = 0, so ist auch der Verbrauch der
/// Marktlokation auf 0 zu setzen."* — reported per step and interval.
#[test]
fn b3_proportional_with_the_quotient_cut_and_the_zero_rule() {
    let r = awh_inputs();
    let m2 = Formula::proportional_share(melo(2), melo(1), &[melo(2), melo(3)]).unwrap();
    let m3 = Formula::proportional_share(melo(3), melo(1), &[melo(2), melo(3)]).unwrap();
    assert_eq!(
        r.eval(&m2).unwrap(),
        decs(["0", "2.6087", "0", "10", "0", "0", "0.412143445"])
    );
    assert_eq!(
        r.eval(&m3).unwrap(),
        decs(["0", "12.3914", "0", "10", "0", "0", "8.2428689"])
    );

    // B3-M1 = E1 − (V2 − B3-M2) − (V3 − B3-M3), one shared denominator step.
    let share = |n: u32| {
        pos(E::Sum(vec![
            plus(v(n)),
            minus(E::Prod(vec![
                E::Div(
                    Box::new(v(n)),
                    Box::new(E::Sum(vec![plus(v(2)), plus(v(3))])),
                ),
                e1(),
            ])),
        ]))
    };
    let m1 = formula(E::Sum(vec![
        plus(e1()),
        minus(E::Sum(vec![plus(v(2)), minus(share(2))])),
        minus(E::Sum(vec![plus(v(3)), minus(share(3))])),
    ]));
    // Row 2 and 7: the generation the cut quotients leave unallocated.
    assert_eq!(
        r.eval(&m1).unwrap(),
        decs(["45", "0.0001", "0", "0", "100", "10", "0.000012345"])
    );

    let out = m2.eval(|m, d| r.0.get(&(*m, d))).unwrap();
    assert_eq!(out.zero_divisors.len(), 1);
    assert_eq!(out.zero_divisors[0].step, id(2));
    assert_eq!(out.zero_divisors[0].at, BASE + Duration::minutes(60));
}

/// The balance invariant (B1, B2): `M1 + (V2 − M2) + (V3 − M3) = E1`.
#[test]
fn the_pv_balance_holds_on_every_row() {
    let r = awh_inputs();
    let m1 = r
        .eval(&Formula::new(id(9), b1_m1_steps(4)).unwrap())
        .unwrap();
    let m2 = r.eval(&b1_m2()).unwrap();
    let m3 = r.eval(&b1_m3()).unwrap();
    let e = |k: usize| r.0[&(melo(1), Direction::Export)].as_slice()[k].value();
    let c = |n, k: usize| r.0[&(melo(n), Direction::Import)].as_slice()[k].value();
    for k in 0..7 {
        assert_eq!(
            m1[k] + (c(2, k) - m2[k]) + (c(3, k) - m3[k]),
            e(k),
            "row {}",
            k + 1
        );
    }
}

// ── B. semantics ────────────────────────────────────────────────────────────

fn single(registers: &[(u32, Direction, Decimal)]) -> Registers {
    Registers::new(registers.iter().map(|(n, d, v)| (melo(*n), *d, vec![*v])))
}

fn a(n: u32) -> MeloOperand {
    MeloOperand::new(melo(n), FlowDirection::Verbrauch)
}

fn loss(f: Decimal) -> LossFactor {
    LossFactor::new(f).unwrap()
}

#[test]
fn l1_a_loss_factor_applies_before_the_operation() {
    let r = single(&[
        (1, Direction::Import, dec!(100)),
        (2, Direction::Import, dec!(50)),
    ]);
    let f = formula(E::Sum(vec![
        plus(E::M(a(1).trafo(loss(dec!(1.04))))),
        minus(E::M(a(2))),
    ]));
    assert_eq!(
        r.eval(&f).unwrap(),
        vec![dec!(54)],
        "52 would mean after the operation"
    );
}

#[test]
fn l2_both_loss_factors_multiply() {
    let r = single(&[(1, Direction::Import, dec!(100))]);
    let f = formula(E::Sum(vec![plus(E::M(
        a(1).trafo(loss(dec!(1.04))).line(loss(dec!(0.98))),
    ))]));
    assert_eq!(r.eval(&f).unwrap(), vec![dec!(101.92)]);
}

#[test]
fn l3_a_loss_factor_stays_on_its_operand() {
    let r = single(&[
        (1, Direction::Import, dec!(100)),
        (2, Direction::Import, dec!(100)),
    ]);
    let f = formula(E::Sum(vec![
        plus(E::M(a(1).trafo(loss(dec!(1.04))))),
        plus(E::M(a(2))),
    ]));
    assert_eq!(r.eval(&f).unwrap(), vec![dec!(204)]);
}

#[test]
fn p1_positivwert_clips_per_interval_inside_the_sum() {
    let r = single(&[
        (1, Direction::Import, dec!(0)),
        (2, Direction::Import, dec!(10)),
        (3, Direction::Import, dec!(30)),
        (4, Direction::Import, dec!(0)),
    ]);
    let f = formula(E::Sum(vec![
        plus(pos(E::Sum(vec![plus(E::M(a(1))), minus(E::M(a(2)))]))),
        plus(pos(E::Sum(vec![plus(E::M(a(3))), minus(E::M(a(4)))]))),
    ]));
    assert_eq!(
        r.eval(&f).unwrap(),
        vec![dec!(30)],
        "20 would mean clipped after the sum"
    );
}

#[test]
fn p2_positivwert_at_zero_is_zero() {
    let r = single(&[
        (1, Direction::Import, dec!(5)),
        (2, Direction::Import, dec!(5)),
    ]);
    let f = formula(pos(E::Sum(vec![plus(E::M(a(1))), minus(E::M(a(2)))])));
    assert_eq!(r.eval(&f).unwrap(), vec![dec!(0)]);
}

#[test]
fn s1_a_split_factor_multiplies() {
    let r = single(&[(1, Direction::Export, dec!(50))]);
    let m = MeloOperand::new(melo(1), FlowDirection::Erzeugung)
        .split(SplitFactor::new(dec!(0.2)).unwrap());
    assert_eq!(
        r.eval(&formula(E::Prod(vec![E::M(m)]))).unwrap(),
        vec![dec!(10)]
    );
}

#[test]
fn q1_the_dividend_is_the_numerator() {
    let r = single(&[
        (1, Direction::Import, dec!(1)),
        (2, Direction::Import, dec!(4)),
    ]);
    let f = formula(E::Div(Box::new(E::M(a(1))), Box::new(E::M(a(2)))));
    assert_eq!(r.eval(&f).unwrap(), vec![dec!(0.25)]);
}

/// Q2 under the AWH zero rule: a zero divisor gives 0 and is reported with
/// step and instant.
#[test]
fn q2_a_zero_divisor_is_zero_and_reported() {
    let r = single(&[
        (1, Direction::Import, dec!(1)),
        (2, Direction::Import, dec!(0)),
    ]);
    let f = formula(E::Div(Box::new(E::M(a(1))), Box::new(E::M(a(2)))));
    let out = f.eval(|m, d| r.0.get(&(*m, d))).unwrap();
    assert_eq!(out.series.as_slice()[0].value(), dec!(0));
    assert_eq!(out.zero_divisors.len(), 1);
    assert_eq!(
        (out.zero_divisors[0].step, out.zero_divisors[0].at),
        (f.result(), BASE)
    );
}

#[test]
fn x1_a_product_of_several() {
    let r = single(&[
        (1, Direction::Import, dec!(2)),
        (2, Direction::Import, dec!(3)),
        (3, Direction::Import, dec!(0.5)),
    ]);
    let f = formula(E::Prod(vec![E::M(a(1)), E::M(a(2)), E::M(a(3))]));
    assert_eq!(r.eval(&f).unwrap(), vec![dec!(3)]);
}

#[test]
fn r1_the_direction_selects_a_register_never_a_sign() {
    let r = single(&[
        (1, Direction::Import, dec!(7)),
        (1, Direction::Export, dec!(3)),
    ]);
    let verbrauch = Formula::melo(MeloOperand::new(melo(1), FlowDirection::Verbrauch)).unwrap();
    let erzeugung = Formula::melo(MeloOperand::new(melo(1), FlowDirection::Erzeugung)).unwrap();
    assert_eq!(r.eval(&verbrauch).unwrap(), vec![dec!(7)]);
    assert_eq!(r.eval(&erzeugung).unwrap(), vec![dec!(3)]);
}

/// The quotient cut is toward zero and exact: `|q × divisor| ≤ |dividend|`.
#[test]
fn the_quotient_is_cut_toward_zero_once() {
    let r = single(&[
        (1, Direction::Import, dec!(2)),
        (2, Direction::Import, dec!(3)),
    ]);
    let f = formula(E::Div(Box::new(E::M(a(1))), Box::new(E::M(a(2)))));
    assert_eq!(r.eval(&f).unwrap(), vec![dec!(0.666666)]);
}

// ── C. construction ────────────────────────────────────────────────────────

#[test]
fn lf_loss_factor_value_rules() {
    for bad in [dec!(1), dec!(0), dec!(-1.04), dec!(1.0400001)] {
        assert_eq!(
            LossFactor::new(bad),
            Err(FormulaError::InvalidLossFactor { value: bad }),
            "{bad}"
        );
    }
    for good in [dec!(1.040000), dec!(0.98), dec!(1.000001)] {
        assert!(LossFactor::new(good).is_ok(), "{good}");
    }
}

#[test]
fn sf_split_factor_value_rules() {
    assert!(SplitFactor::new(dec!(1)).is_ok(), "SF-1: 1 is allowed");
    for bad in [dec!(1.000001), dec!(0), dec!(0.1234567)] {
        assert_eq!(
            SplitFactor::new(bad),
            Err(FormulaError::InvalidSplitFactor { value: bad }),
            "{bad}"
        );
    }
}

#[test]
fn id_step_ids_run_from_one_to_99999() {
    assert_eq!(StepId::new(0), Err(FormulaError::InvalidStepId { id: 0 }));
    assert_eq!(
        StepId::new(100_000),
        Err(FormulaError::InvalidStepId { id: 100_000 })
    );
    assert_eq!(StepId::new(1).unwrap().get(), 1);
    assert_eq!(StepId::new(99_999).unwrap().get(), 99_999);
}

fn sum_of(step: u32) -> Step {
    Step::Sum(vec![(Sign::Plus, Operand::Step(id(step)))])
}

fn melo_sum() -> Step {
    Step::Sum(vec![(Sign::Plus, a(1).into())])
}

#[test]
fn g1_to_g4_the_graph_is_checked_at_construction() {
    assert_eq!(
        Formula::new(id(2), [(id(1), melo_sum())]),
        Err(FormulaError::MissingResult { result: id(2) })
    );
    assert_eq!(
        Formula::new(id(2), [(id(2), sum_of(2))]),
        Err(FormulaError::SelfReference { step: id(2) })
    );
    assert_eq!(
        Formula::new(id(1), [(id(1), sum_of(2)), (id(2), sum_of(1))]),
        Err(FormulaError::Cycle {
            cycle: vec![id(1), id(2), id(1)]
        })
    );
    assert_eq!(
        Formula::new(id(1), [(id(1), sum_of(3))]),
        Err(FormulaError::UnknownStep {
            referrer: id(1),
            step: id(3)
        })
    );
    assert_eq!(
        Formula::new(id(1), [(id(1), Step::Sum(Vec::new()))]),
        Err(FormulaError::EmptyStep { step: id(1) })
    );
}

fn component(step: u32, operator: Operator, operand: Operand) -> Component {
    Component {
        step: id(step),
        operator,
        operand,
    }
}

#[test]
fn g5_to_g8_wire_groups_must_form_one_step_kind() {
    let m = || Operand::Melo(a(1));
    // G-5: Z69 beside Z82.
    assert!(matches!(
        Formula::from_components(
            id(1),
            [
                component(1, Operator::Addition, m()),
                component(1, Operator::Faktor, m())
            ]
        ),
        Err(FormulaError::MixedStep { .. })
    ));
    // G-6: a Divisor without its Dividend, and two Divisors.
    assert!(matches!(
        Formula::from_components(id(1), [component(1, Operator::Divisor, m())]),
        Err(FormulaError::MixedStep { .. })
    ));
    assert!(matches!(
        Formula::from_components(
            id(1),
            [
                component(1, Operator::Divisor, m()),
                component(1, Operator::Divisor, m()),
                component(1, Operator::Dividend, m()),
            ]
        ),
        Err(FormulaError::MixedStep { .. })
    ));
    // G-7: Z83 beside Z69.
    assert!(matches!(
        Formula::from_components(
            id(2),
            [
                component(1, Operator::Addition, m()),
                component(2, Operator::Positivwert, Operand::Step(id(1))),
                component(2, Operator::Addition, m()),
            ]
        ),
        Err(FormulaError::MixedStep { .. })
    ));
    // G-8: Positivwert over a MeLo.
    assert_eq!(
        Formula::from_components(id(1), [component(1, Operator::Positivwert, m())]),
        Err(FormulaError::PositivwertOverMelo { step: id(1) })
    );
    // The B1-M2 message, component by component, is the named constructor.
    let wire = Formula::from_components(
        id(3),
        [
            component(
                1,
                Operator::Faktor,
                MeloOperand::new(melo(1), FlowDirection::Erzeugung)
                    .with_factor(FactorKind::Aufteilungsfaktor, dec!(0.1))
                    .unwrap()
                    .into(),
            ),
            component(2, Operator::Subtraktion, Operand::Step(id(1))),
            component(
                2,
                Operator::Addition,
                MeloOperand::new(melo(2), FlowDirection::Verbrauch).into(),
            ),
            component(3, Operator::Positivwert, Operand::Step(id(2))),
        ],
    )
    .unwrap();
    assert_eq!(wire, b1_m2());
}

#[test]
fn g9_a_factor_qualifier_occurs_once() {
    let once = a(1)
        .with_factor(FactorKind::VerlustfaktorTrafo, dec!(1.04))
        .unwrap();
    assert_eq!(
        once.with_factor(FactorKind::VerlustfaktorTrafo, dec!(1.02)),
        Err(FormulaError::DuplicateFactor {
            melo: melo(1),
            kind: FactorKind::VerlustfaktorTrafo
        })
    );
}

#[test]
fn the_operator_codes_are_the_ahb_codes() {
    let codes: Vec<&str> = Operator::ALL.iter().map(|o| o.as_str()).collect();
    assert_eq!(codes, ["Z69", "Z70", "Z80", "Z81", "Z82", "Z83"]);
    assert_eq!(FlowDirection::CODES, ["Z71", "Z72"]);
    assert_eq!(FactorKind::CODES, ["Z16", "ZB2", "ZG6"]);
    assert_eq!(Sign::Minus.operator(), Operator::Subtraktion);
    assert_eq!("z83".parse::<Operator>().unwrap(), Operator::Positivwert);
}

// ── evaluation errors (E-1, E-2) and the remaining refusals ───────────────────

#[test]
fn e1_a_missing_register_is_named() {
    let r = single(&[
        (1, Direction::Export, dec!(10)),
        (2, Direction::Import, dec!(1)),
    ]);
    let f = Formula::proportional_share(melo(2), melo(1), &[melo(2), melo(3)]).unwrap();
    assert_eq!(
        r.eval(&f),
        Err(FormulaError::MissingSeries {
            melo: melo(3),
            direction: FlowDirection::Verbrauch
        })
    );
}

#[test]
fn e2_a_series_on_another_grid_is_named_with_the_instant() {
    let mut r = single(&[(1, Direction::Export, dec!(10))]);
    let hourly = Series::new(
        Resolution::Hour,
        DayBoundary::Strom,
        vec![MeterInterval::hour(BASE, dec!(4), QualityFlag::Measured).unwrap()],
    )
    .unwrap();
    r.0.insert((melo(2), Direction::Import), hourly);
    let f =
        Formula::constant_share(melo(2), melo(1), SplitFactor::new(dec!(0.5)).unwrap()).unwrap();
    let Err(FormulaError::GridMismatch { at, .. }) = r.eval(&f) else {
        panic!("a 15-minute and a 60-minute series are not one grid");
    };
    assert_eq!(at, Some(BASE));
}

/// A gap in one register is a different grid, not a silently shorter result.
#[test]
fn a_gap_in_one_register_is_refused() {
    let mut r = Registers::new([(melo(1), Direction::Export, vec![dec!(1), dec!(1)])]);
    r.0.insert((melo(2), Direction::Import), qh(&[dec!(1)]));
    let f = Formula::residual(a(2), [MeloOperand::new(melo(1), FlowDirection::Erzeugung)]).unwrap();
    assert!(matches!(r.eval(&f), Err(FormulaError::GridMismatch { .. })));
}

#[test]
fn a_negative_register_value_is_refused_with_its_instant() {
    let r = single(&[(1, Direction::Import, dec!(-1))]);
    let f = Formula::melo(a(1)).unwrap();
    assert_eq!(
        r.eval(&f),
        Err(FormulaError::NegativeValue {
            melo: melo(1),
            direction: FlowDirection::Verbrauch,
            at: BASE
        })
    );
}

#[test]
fn a_proportional_tenant_must_be_in_the_denominator_once() {
    assert_eq!(
        Formula::proportional_share(melo(9), melo(1), &[melo(2), melo(3)]),
        Err(FormulaError::Participants { melo: melo(9) })
    );
    assert_eq!(
        Formula::proportional_share(melo(2), melo(1), &[melo(2), melo(2)]),
        Err(FormulaError::Participants { melo: melo(2) })
    );
}

// ── the fixed shapes, as formulas ──────────────────────────────────────

#[test]
fn sum_residual_and_net_exchange_are_formulas() {
    let r = Registers::new([
        (melo(1), Direction::Import, vec![dec!(3), dec!(3)]),
        (melo(2), Direction::Import, vec![dec!(2), dec!(2)]),
    ]);
    assert_eq!(
        r.eval(&Formula::sum([a(1), a(2)]).unwrap()).unwrap(),
        vec![dec!(5), dec!(5)]
    );

    let r = Registers::new([
        (melo(1), Direction::Import, vec![dec!(10), dec!(8), dec!(1)]),
        (melo(2), Direction::Export, vec![dec!(3), dec!(2), dec!(5)]),
    ]);
    let net =
        Formula::residual(a(1), [MeloOperand::new(melo(2), FlowDirection::Erzeugung)]).unwrap();
    assert_eq!(
        r.eval(&net).unwrap(),
        vec![dec!(7), dec!(6), dec!(-4)],
        "signed: −4 is feed-in"
    );
}

/// The result carries the worst input quality and no OBIS channel.
#[test]
fn the_result_carries_the_worst_quality() {
    let mut r = single(&[(1, Direction::Import, dec!(1))]);
    let estimated = Series::new(
        Resolution::QUARTER_HOUR,
        DayBoundary::Strom,
        vec![MeterInterval::quarter_hour(BASE, dec!(2), QualityFlag::Estimated).unwrap()],
    )
    .unwrap();
    r.0.insert((melo(2), Direction::Import), estimated);
    let out = Formula::sum([a(1), a(2)])
        .unwrap()
        .eval(|m, d| r.0.get(&(*m, d)))
        .unwrap();
    assert_eq!(out.series.as_slice()[0].quality(), QualityFlag::Estimated);
    assert_eq!(out.series.channel(), None);
}
