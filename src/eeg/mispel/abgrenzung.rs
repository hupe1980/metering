//! MiSpeL Anlage 1 — the Abgrenzungsoption, one calendar month at a time.
//!
//! [`Abgrenzungsoption::monat`] runs the two-meter Formelsatz (Basisfälle
//! A1–A4, with the Sonderfälle A5, A6 and A7 on top) over one calendar month
//! or Rumpfmonat; [`Einzelzaehler::monat`] runs the single-meter Sonderfälle
//! A8, A10 and A11; [`jahr`] and [`jahr_einzelzaehler`] add months to the
//! year's (22) and (33).
//!
//! Three levels, each finished before the next (Anl. 1 pp. 14, 33–39): per
//! quarter-hour (1)¼, (2)¼, (23)¼ and the gates; per month the sums, then
//! every `MIN`/`MAX` clamp; per year the sums of monthly results.
//!
//! *"Der jeweilige Rumpfmonat tritt bei der entsprechenden Anwendung an die
//! Stelle des Kalendermonats"* (Kap. 11, p. 102): pass the part-month's
//! quarter-hours.
//!
//! Every numbered intermediate is in the result as `fN` for formula (N).

use rust_decimal::{Decimal, dec};
use time::OffsetDateTime;

use super::{Abgrenzungsfall, Einspeisung, Gate};
use crate::eeg::{EegError, Row, add, align, coincident, mul, share, sub, sum};
use crate::series::Series;
use crate::series::interval::QualityFlag;
use crate::time::calendar::DayBoundary;
use crate::time::resolution::Resolution;

/// (14)A2,A3,A4 = 0,85 — the Wirkungsgrad wherever a charge point sits
/// behind Z2 (p. 35).
pub const WIRKUNGSGRAD_LADEPUNKT: Decimal = dec!(0.85);

/// What sits behind Z2 — the Basisfall; selects (14), (17) and (19).
#[derive(Debug, Clone, Copy)]
pub enum Basisfall<'a> {
    /// A1 — storage: (14)A1 = (6) / (5); (17)A1 = MAX \[ (5) – (6) ; 0 \].
    Stromspeicher,
    /// A2 — charge point: (14) = 0,85; (19)A2,A3 = 0.
    Ladepunkt,
    /// A3 — storage and charge point behind one Z2: as A2.
    StromspeicherUndLadepunkt,
    /// A4 — as A3 plus a storage-only meter Z3:
    /// (17)A4 = MAX \[ (7)A4 – (8)A4 ; 0 \].
    MitSpeicherzaehler {
        /// Z3V¼ — consumption of the storage.
        z3v: &'a Series,
        /// Z3E¼ — generation of the storage.
        z3e: &'a Series,
    },
}

impl Basisfall<'_> {
    /// The case code.
    #[must_use]
    pub const fn fall(&self) -> Abgrenzungsfall {
        match self {
            Self::Stromspeicher => Abgrenzungsfall::A1,
            Self::Ladepunkt => Abgrenzungsfall::A2,
            Self::StromspeicherUndLadepunkt => Abgrenzungsfall::A3,
            Self::MitSpeicherzaehler { .. } => Abgrenzungsfall::A4,
        }
    }
}

/// A plant of A5, weighted by its installed power.
#[derive(Debug, Clone, Copy)]
pub struct GewichteteAnlage<'a> {
    /// Painst — installed power (for onshore wind the Referenz- or
    /// Standortertrag), > 0.
    pub leistung: Decimal,
    /// AW¼ > 0 of this plant — (24a)¼, (24b)¼.
    pub aw: Gate<'a>,
}

/// A metered plant with priority in A6.
#[derive(Debug, Clone, Copy)]
pub struct VorrangAnlage<'a> {
    /// Z4E¼ — the plant's own generation.
    pub z4e: &'a Series,
    /// AW¼ > 0 of this plant.
    pub aw: Gate<'a>,
}

/// The generation side — how (23)¼ and (28) are split between plants.
#[derive(Debug, Clone)]
pub enum Anlagen<'a> {
    /// One plant: the Basisfälle (and Kap. 10.1 with [`Gate::Always`]).
    Eine(Gate<'a>),
    /// A5 — plants of the same kind. ZF = Pkinst / Σ Pinst (p. 46),
    /// (23k)¼ A5 = ZF • (23)¼, (28k)A5 = ZF • (28). The A5-Variante,
    /// (32k) = ZF • (32) (p. 53), is this with one gate for every plant.
    Gleichartig(Vec<GewichteteAnlage<'a>>),
    /// A6 — plants in priority order, each metered by Z4, then the plant
    /// that takes the residual (pp. 58–63):
    /// (34a)¼ = MIN \[ Z2V¼ – (1)¼ ; Z4E¼ \], (34b)¼ the rest;
    /// (23a)¼ A6 = MIN \[ (23)¼ ; Z4E¼ – (34a)¼ \], (23b)¼ A6 the rest;
    /// (36k) = (35k) / (10), (28k)A6 = (36k) • (28). The two-plant text
    /// *"lässt sich … entsprechend erweitern"*: each further metered plant
    /// takes its share of what the ones before it left.
    Vorrang {
        /// The metered plants with priority, highest first.
        vorrang: Vec<VorrangAnlage<'a>>,
        /// AW¼ > 0 of the plant taking the residual.
        rest: Gate<'a>,
    },
}

/// The inputs of one month under the two-meter Formelsatz.
#[derive(Debug, Clone)]
pub struct Abgrenzungsoption<'a> {
    /// Z1NB¼ — grid withdrawal at the Entnahmestelle.
    pub z1nb: &'a Series,
    /// Z1NE¼, or ZWNE¼ for A7.
    pub einspeisung: Einspeisung<'a>,
    /// Z2V¼ — consumption of the storage and/or charge point.
    pub z2v: &'a Series,
    /// Z2E¼ — generation of the storage and/or charge point.
    pub z2e: &'a Series,
    /// What sits behind Z2.
    pub basisfall: Basisfall<'a>,
    /// The generation side.
    pub anlagen: Anlagen<'a>,
}

/// One plant's share of one quarter-hour.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct AnlageViertelstunde {
    /// AW¼ > 0 — (24)¼ / (24k)¼ as a flag.
    pub aw: bool,
    /// (23k)¼ — the plant's share of the direct feed-in, before the gate;
    /// (25k)¼ = `aw` • this.
    pub f23: Decimal,
    /// (34k)¼ — A6 only: the plant's share of Z2V¼ – (1)¼.
    pub f34: Option<Decimal>,
}

/// One quarter-hour of [`Monat`].
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Viertelstunde {
    /// Start (UTC).
    pub from: OffsetDateTime,
    /// End (UTC, exclusive).
    pub to: OffsetDateTime,
    /// Z1NB¼.
    pub z1nb: Decimal,
    /// Z1NE¼ (ZWNE¼ in A7).
    pub einspeisung: Decimal,
    /// Z2V¼.
    pub z2v: Decimal,
    /// Z2E¼.
    pub z2e: Decimal,
    /// (1)¼ = MIN \[ Z1NB¼ ; Z2V¼ \] (p. 33).
    pub f1: Decimal,
    /// (2)¼ = MIN \[ Z1NE¼ ; Z2E¼ \] (p. 34).
    pub f2: Decimal,
    /// (23)¼ = Z1NE¼ – (2)¼ (p. 38).
    pub f23: Decimal,
    /// Per plant, in the order of [`Monat::anlagen`].
    pub anlagen: Vec<AnlageViertelstunde>,
    /// The worst quality among the inputs; reported, never hidden.
    pub quality: QualityFlag,
}

/// One plant's monthly förderfähige Netzeinspeisung (§ 4.2.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct AnlageMonat {
    /// (26) = ∑M (25)¼ — direct feed-in in AW>0 quarter-hours (p. 38).
    pub f26: Decimal,
    /// (28) (or (28k)A5, (28k)A6) — the plant's grid-sourced storage feed-in
    /// (pp. 38, 48, 61).
    pub f28: Decimal,
    /// (29) = ∑M (27)¼ — storage feed-in (2)¼ in AW>0 quarter-hours (p. 39).
    pub f29: Decimal,
    /// (30) = (29) / (11); `None` when (11) = 0, where (31) is 0.
    pub f30: Option<Decimal>,
    /// (31) = (30) • (28), formed as (29) • (28) / (11).
    pub f31: Decimal,
    /// (32) = (26) + (31) — förderfähige Netzeinspeisung (p. 39).
    pub f32: Decimal,
    /// (35k) = ∑M (34k)¼ — A6 only (p. 61).
    pub f35: Option<Decimal>,
    /// (36k) = (35k) / (10) — A6 only; `None` also when (10) = 0.
    pub f36: Option<Decimal>,
}

/// One calendar month (or Rumpfmonat) under the two-meter Formelsatz.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Monat {
    /// The Formelsätze applied: the Basisfall, then A5/A6/A7 when present.
    pub faelle: Vec<Abgrenzungsfall>,
    /// First quarter-hour start (UTC).
    pub from: OffsetDateTime,
    /// End of the last quarter-hour (UTC, exclusive).
    pub to: OffsetDateTime,
    /// (3) = ∑M Z1NB¼ — Netzbezug.
    pub f3: Decimal,
    /// (4) = ∑M Z1NE¼ — Netzeinspeisung.
    pub f4: Decimal,
    /// (5) = ∑M Z2V¼.
    pub f5: Decimal,
    /// (6) = ∑M Z2E¼.
    pub f6: Decimal,
    /// (7)A4 = ∑M Z3V¼ — A4 only.
    pub f7: Option<Decimal>,
    /// (8)A4 = ∑M Z3E¼ — A4 only.
    pub f8: Option<Decimal>,
    /// (9) = ∑M (1)¼ — storage consumption from the grid.
    pub f9: Decimal,
    /// (10) = (5) – (9) — storage consumption from the plant.
    pub f10: Decimal,
    /// (11) = ∑M (2)¼ — grid feed-in from the storage.
    pub f11: Decimal,
    /// (12) = MAX \[ (6) – (5) ; 0 \] — Fremdtankstrom.
    pub f12: Decimal,
    /// (13) = MAX \[ (11) – (12) ; 0 \].
    pub f13: Decimal,
    /// (14) — Wirkungsgrad: (6) / (5) in A1 (`None` when (5) = 0, where
    /// (15) is 0); 0,85 otherwise. Not clamped: (14)A1 may exceed 1.
    pub f14: Option<Decimal>,
    /// (15) = (14) • (10) — EE-Speichererzeugung.
    pub f15: Decimal,
    /// (16) = MAX \[ (13) – (15) ; 0 \] — saldierungsfähige Netzeinspeisung.
    pub f16: Decimal,
    /// (17) — storage losses: A1 from Z2, A4 from Z3; `None` in A2/A3.
    pub f17: Option<Decimal>,
    /// (18) = (16) / (6); `None` when (6) = 0.
    pub f18: Option<Decimal>,
    /// (19) — privilegierungsfähige Stromspeicherverluste: (18) • (17) in
    /// A1/A4, formed as (16) • (17) / (6); 0 in A2/A3 and when (6) = 0 (no
    /// arbitrage feed-in, no arbitrage loss — Begründung).
    pub f19: Decimal,
    /// (20) = MIN \[ (16) + (19) ; (3) \] — umlagereduzierende Strommenge.
    pub f20: Decimal,
    /// (21) = (3) – (20) — umlagebelasteter Netzbezug.
    pub f21: Decimal,
    /// (28) = MIN \[ (13) ; (15) \] — before any plant split.
    pub f28: Decimal,
    /// Per plant; in A6 the priority plants in order, then the residual one.
    pub anlagen: Vec<AnlageMonat>,
    /// Every quarter-hour, ascending.
    pub viertelstunden: Vec<Viertelstunde>,
}

impl Monat {
    /// Σ (32) over the plants — the month's förderfähige Netzeinspeisung.
    ///
    /// # Errors
    ///
    /// [`EegError::Overflow`].
    pub fn foerderfaehig(&self) -> Result<Decimal, EegError> {
        sum(self.anlagen.iter().map(|a| a.f32))
    }
}

/// The intervals of one quarter-hour series must lie in one calendar month.
fn one_month(rows: &[Row]) -> Result<(OffsetDateTime, OffsetDateTime), EegError> {
    let (Some(first), Some(last)) = (rows.first(), rows.last()) else {
        return Err(EegError::Empty);
    };
    let month = DayBoundary::Strom
        .bucket(first.from, Resolution::Month)
        .ok_or(EegError::Calendar)?;
    if let Some(out) = rows.iter().find(|r| r.to > month.end()) {
        return Err(EegError::OutsidePeriod { at: out.from });
    }
    Ok((first.from, last.to))
}

fn clamp0(x: Decimal) -> Decimal {
    x.max(Decimal::ZERO)
}

/// A plant's (28k) as numerator ÷ denominator, so (31k) divides once.
struct Split {
    num: Decimal,
    den: Decimal,
}

impl Abgrenzungsoption<'_> {
    /// Evaluate one calendar month or Rumpfmonat.
    ///
    /// # Errors
    ///
    /// [`EegError`] when a series is not quarter-hourly, the grids differ, a
    /// value is negative or non-billable, the quarter-hours reach into a
    /// second calendar month, an A5 weight is not positive, a plant list is
    /// empty, or on overflow.
    pub fn monat(&self) -> Result<Monat, EegError> {
        let mut inputs: Vec<(&'static str, &Series)> = vec![
            ("Z1NB", self.z1nb),
            (self.einspeisung.name(), self.einspeisung.series()),
            ("Z2V", self.z2v),
            ("Z2E", self.z2e),
        ];
        if let Basisfall::MitSpeicherzaehler { z3v, z3e } = self.basisfall {
            inputs.push(("Z3V", z3v));
            inputs.push(("Z3E", z3e));
        }
        let z4_at = inputs.len();
        if let Anlagen::Vorrang { vorrang, .. } = &self.anlagen {
            if vorrang.is_empty() {
                return Err(EegError::Inconsistent {
                    what: "A6 needs at least one plant with priority",
                });
            }
            inputs.extend(vorrang.iter().map(|p| ("Z4E", p.z4e)));
        }
        let rows = align(&inputs)?;
        let (from, to) = one_month(&rows)?;

        let mut faelle = vec![self.basisfall.fall()];
        match &self.anlagen {
            Anlagen::Eine(_) => {}
            Anlagen::Gleichartig(list) => {
                if list.is_empty() {
                    return Err(EegError::Inconsistent {
                        what: "A5 needs at least one plant",
                    });
                }
                if list.iter().any(|p| p.leistung <= Decimal::ZERO) {
                    return Err(EegError::ZeroWeight {
                        formula: "(ZF)",
                        what: "installed power",
                    });
                }
                faelle.push(Abgrenzungsfall::A5);
            }
            Anlagen::Vorrang { .. } => faelle.push(Abgrenzungsfall::A6),
        }
        if matches!(self.einspeisung, Einspeisung::Zw(_)) {
            faelle.push(Abgrenzungsfall::A7);
        }

        let gates: Vec<Gate<'_>> = match &self.anlagen {
            Anlagen::Eine(g) => vec![*g],
            Anlagen::Gleichartig(list) => list.iter().map(|p| p.aw).collect(),
            Anlagen::Vorrang { vorrang, rest } => {
                vorrang.iter().map(|p| p.aw).chain([*rest]).collect()
            }
        };
        let weights: Option<(Vec<Decimal>, Decimal)> = match &self.anlagen {
            Anlagen::Gleichartig(list) => {
                let w: Vec<Decimal> = list.iter().map(|p| p.leistung).collect();
                let total = sum(w.iter().copied())?;
                Some((w, total))
            }
            _ => None,
        };

        // ── per quarter-hour ────────────────────────────────────────────
        let mut quarters = Vec::with_capacity(rows.len());
        for r in &rows {
            let (z1nb, ne, z2v, z2e) = (r.values[0], r.values[1], r.values[2], r.values[3]);
            let f1 = coincident(z1nb, z2v);
            let f2 = coincident(ne, z2e);
            let f23 = sub(ne, f2)?;
            let anlagen = match (&self.anlagen, &weights) {
                (Anlagen::Gleichartig(_), Some((w, total))) => gates
                    .iter()
                    .zip(w)
                    .map(|(g, wk)| {
                        Ok(AnlageViertelstunde {
                            aw: g.passes(r.from),
                            f23: share(*wk, f23, *total)?.ok_or(EegError::Overflow)?,
                            f34: None,
                        })
                    })
                    .collect::<Result<Vec<_>, EegError>>()?,
                (Anlagen::Vorrang { .. }, _) => {
                    let mut left_v = sub(z2v, f1)?;
                    let mut left_23 = f23;
                    let mut out = Vec::with_capacity(gates.len());
                    for (k, g) in gates.iter().enumerate() {
                        let (f34, f23k) = match r.values.get(z4_at + k) {
                            Some(&z4e) => {
                                let f34 = left_v.min(z4e);
                                let f23k = left_23.min(sub(z4e, f34)?);
                                (f34, f23k)
                            }
                            None => (left_v, left_23),
                        };
                        left_v = sub(left_v, f34)?;
                        left_23 = sub(left_23, f23k)?;
                        out.push(AnlageViertelstunde {
                            aw: g.passes(r.from),
                            f23: f23k,
                            f34: Some(f34),
                        });
                    }
                    out
                }
                _ => vec![AnlageViertelstunde {
                    aw: gates[0].passes(r.from),
                    f23,
                    f34: None,
                }],
            };
            quarters.push(Viertelstunde {
                from: r.from,
                to: r.to,
                z1nb,
                einspeisung: ne,
                z2v,
                z2e,
                f1,
                f2,
                f23,
                anlagen,
                quality: r.quality,
            });
        }

        // ── per month: sums ─────────────────────────────────────────────
        let f3 = sum(quarters.iter().map(|q| q.z1nb))?;
        let f4 = sum(quarters.iter().map(|q| q.einspeisung))?;
        let f5 = sum(quarters.iter().map(|q| q.z2v))?;
        let f6 = sum(quarters.iter().map(|q| q.z2e))?;
        let (f7, f8) = if matches!(self.basisfall, Basisfall::MitSpeicherzaehler { .. }) {
            (
                Some(sum(rows.iter().map(|r| r.values[4]))?),
                Some(sum(rows.iter().map(|r| r.values[5]))?),
            )
        } else {
            (None, None)
        };
        let f9 = sum(quarters.iter().map(|q| q.f1))?;
        let f10 = sub(f5, f9)?;
        let f11 = sum(quarters.iter().map(|q| q.f2))?;

        // ── per month: clamps and shares ────────────────────────────────
        let f12 = clamp0(sub(f6, f5)?);
        let f13 = clamp0(sub(f11, f12)?);
        let (f14, f15) = match self.basisfall {
            Basisfall::Stromspeicher => (
                share(f6, Decimal::ONE, f5)?,
                share(f6, f10, f5)?.unwrap_or(Decimal::ZERO),
            ),
            _ => (
                Some(WIRKUNGSGRAD_LADEPUNKT),
                mul(WIRKUNGSGRAD_LADEPUNKT, f10)?,
            ),
        };
        let f16 = clamp0(sub(f13, f15)?);
        let f17 = match (self.basisfall, f7, f8) {
            (Basisfall::Stromspeicher, _, _) => Some(clamp0(sub(f5, f6)?)),
            (Basisfall::MitSpeicherzaehler { .. }, Some(v), Some(e)) => Some(clamp0(sub(v, e)?)),
            _ => None,
        };
        let f18 = share(f16, Decimal::ONE, f6)?;
        let f19 = match f17 {
            Some(l) => share(f16, l, f6)?.unwrap_or(Decimal::ZERO),
            None => Decimal::ZERO,
        };
        let f20 = add(f16, f19)?.min(f3);
        let f21 = sub(f3, f20)?;
        let f28 = f13.min(f15);

        // ── per plant ───────────────────────────────────────────────────
        let f35: Vec<Option<Decimal>> = match &self.anlagen {
            Anlagen::Vorrang { .. } => (0..gates.len())
                .map(|k| {
                    sum(quarters
                        .iter()
                        .map(|q| q.anlagen[k].f34.unwrap_or(Decimal::ZERO)))
                    .map(Some)
                })
                .collect::<Result<_, _>>()?,
            _ => vec![None; gates.len()],
        };
        let mut anlagen = Vec::with_capacity(gates.len());
        for k in 0..gates.len() {
            let gated = |value: fn(&Viertelstunde, usize) -> Decimal| {
                sum(quarters
                    .iter()
                    .filter(|q| q.anlagen[k].aw)
                    .map(|q| value(q, k)))
            };
            let split = match (&self.anlagen, &weights) {
                (Anlagen::Gleichartig(_), Some((w, total))) => Split {
                    num: mul(w[k], f28)?,
                    den: *total,
                },
                (Anlagen::Vorrang { .. }, _) => Split {
                    num: mul(f35[k].unwrap_or(Decimal::ZERO), f28)?,
                    den: f10,
                },
                _ => Split {
                    num: f28,
                    den: Decimal::ONE,
                },
            };
            // (10) = 0 forces (15) = 0 and so (28) = 0: the share is 0.
            let f28k = share(split.num, Decimal::ONE, split.den)?.unwrap_or(Decimal::ZERO);
            let f26 = match (&self.anlagen, &weights) {
                // Product first: ZF • Σ gated (23)¼.
                (Anlagen::Gleichartig(_), Some((w, total))) => {
                    share(w[k], gated(|q, _| q.f23)?, *total)?.unwrap_or(Decimal::ZERO)
                }
                _ => gated(|q, k| q.anlagen[k].f23)?,
            };
            let f29 = gated(|q, _| q.f2)?;
            let f30 = share(f29, Decimal::ONE, f11)?;
            // (11) = 0 forces (13) = 0 and so (28) = 0: (31) is 0.
            let f31 = share(f29, split.num, mul(split.den, f11)?)?.unwrap_or(Decimal::ZERO);
            anlagen.push(AnlageMonat {
                f26,
                f28: f28k,
                f29,
                f30,
                f31,
                f32: add(f26, f31)?,
                f35: f35[k],
                f36: match f35[k] {
                    Some(v) => share(v, Decimal::ONE, f10)?,
                    None => None,
                },
            });
        }

        Ok(Monat {
            faelle,
            from,
            to,
            f3,
            f4,
            f5,
            f6,
            f7,
            f8,
            f9,
            f10,
            f11,
            f12,
            f13,
            f14,
            f15,
            f16,
            f17,
            f18,
            f19,
            f20,
            f21,
            f28,
            anlagen,
            viertelstunden: quarters,
        })
    }
}

/// The single-meter Sonderfälle: only Z1 is metered.
#[derive(Debug, Clone, Copy)]
pub enum Einzelzaehlerfall<'a> {
    /// A8 — storage in co-location, no other consumption, no charge point
    /// (pp. 72–73): (20)A8 = (3), (37)¼ = (24)¼ • Z1NE¼,
    /// (32)A8 = (39) • (40).
    Kolokation {
        /// AW¼ > 0 of the plant.
        aw: Gate<'a>,
    },
    /// A10 — grid-only storage (p. 96): (20)A10 = (3), (21)A10 = 0.
    Netzspeicher,
    /// A11 — storage and/or charge point without other generation
    /// (pp. 100–101): (16)A11 = (4), (20)A11 = MIN \[ (16)A11 ; (3) \].
    OhneErzeugung,
}

/// The inputs of one month under a single-meter Sonderfall.
#[derive(Debug, Clone, Copy)]
pub struct Einzelzaehler<'a> {
    /// Z1NB¼.
    pub z1nb: &'a Series,
    /// Z1NE¼.
    pub z1ne: &'a Series,
    /// The Sonderfall.
    pub fall: Einzelzaehlerfall<'a>,
}

/// One quarter-hour of [`EinzelMonat`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct EinzelViertelstunde {
    /// Start (UTC).
    pub from: OffsetDateTime,
    /// End (UTC, exclusive).
    pub to: OffsetDateTime,
    /// Z1NB¼.
    pub z1nb: Decimal,
    /// Z1NE¼.
    pub z1ne: Decimal,
    /// A8: AW¼ > 0; (37)¼ = this • Z1NE¼. `None` in A10 and A11.
    pub aw: Option<bool>,
    /// The worse of the two input qualities.
    pub quality: QualityFlag,
}

/// One calendar month under a single-meter Sonderfall.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct EinzelMonat {
    /// A8, A10 or A11.
    pub fall: Abgrenzungsfall,
    /// First quarter-hour start (UTC).
    pub from: OffsetDateTime,
    /// End of the last quarter-hour (UTC, exclusive).
    pub to: OffsetDateTime,
    /// (3) = ∑M Z1NB¼.
    pub f3: Decimal,
    /// (4) = ∑M Z1NE¼.
    pub f4: Decimal,
    /// (16)A11 = (4) — A11 only.
    pub f16: Option<Decimal>,
    /// (20)A8, (20)A10 or (20)A11.
    pub f20: Decimal,
    /// (21) = (3) – (20).
    pub f21: Decimal,
    /// (38) = ∑M (37)¼ — A8 only.
    pub f38: Option<Decimal>,
    /// (39) = (38) / (4) — A8 only; `None` also when (4) = 0.
    pub f39: Option<Decimal>,
    /// (40) = MAX \[ (4) – (20)A8 ; 0 \] — A8 only.
    pub f40: Option<Decimal>,
    /// (32)A8 = (39) • (40), formed as (38) • (40) / (4) — A8 only.
    pub f32: Option<Decimal>,
    /// Every quarter-hour, ascending.
    pub viertelstunden: Vec<EinzelViertelstunde>,
}

impl Einzelzaehler<'_> {
    /// Evaluate one calendar month or Rumpfmonat.
    ///
    /// # Errors
    ///
    /// As [`Abgrenzungsoption::monat`].
    pub fn monat(&self) -> Result<EinzelMonat, EegError> {
        let rows = align(&[("Z1NB", self.z1nb), ("Z1NE", self.z1ne)])?;
        let (from, to) = one_month(&rows)?;
        let aw = match self.fall {
            Einzelzaehlerfall::Kolokation { aw } => Some(aw),
            _ => None,
        };
        let viertelstunden: Vec<EinzelViertelstunde> = rows
            .iter()
            .map(|r| EinzelViertelstunde {
                from: r.from,
                to: r.to,
                z1nb: r.values[0],
                z1ne: r.values[1],
                aw: aw.map(|g| g.passes(r.from)),
                quality: r.quality,
            })
            .collect();
        let f3 = sum(viertelstunden.iter().map(|q| q.z1nb))?;
        let f4 = sum(viertelstunden.iter().map(|q| q.z1ne))?;
        let mut m = EinzelMonat {
            fall: Abgrenzungsfall::A10,
            from,
            to,
            f3,
            f4,
            f16: None,
            f20: f3,
            f21: Decimal::ZERO,
            f38: None,
            f39: None,
            f40: None,
            f32: None,
            viertelstunden,
        };
        match self.fall {
            Einzelzaehlerfall::Netzspeicher => {}
            Einzelzaehlerfall::OhneErzeugung => {
                m.fall = Abgrenzungsfall::A11;
                m.f16 = Some(f4);
                m.f20 = f4.min(f3);
                m.f21 = sub(f3, m.f20)?;
            }
            Einzelzaehlerfall::Kolokation { .. } => {
                m.fall = Abgrenzungsfall::A8;
                let f38 = sum(m
                    .viertelstunden
                    .iter()
                    .filter(|q| q.aw == Some(true))
                    .map(|q| q.z1ne))?;
                let f40 = clamp0(sub(f4, f3)?);
                m.f38 = Some(f38);
                m.f39 = share(f38, Decimal::ONE, f4)?;
                m.f40 = Some(f40);
                // (4) = 0 forces (40) = 0.
                m.f32 = Some(share(f38, f40, f4)?.unwrap_or(Decimal::ZERO));
            }
        }
        Ok(m)
    }
}

/// The year's sums of monthly results.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Jahr {
    /// (22) = ∑J (21) — umlagebelasteter Netzbezug of the year (p. 37).
    pub f22: Decimal,
    /// (33) = ∑J (32), per plant (p. 39); A8: (33)A8; empty for A10/A11.
    pub f33: Vec<Decimal>,
}

/// Check that the months lie in one calendar year and do not overlap.
fn one_year(mut spans: Vec<(OffsetDateTime, OffsetDateTime)>) -> Result<(), EegError> {
    spans.sort_unstable();
    let year = |t| DayBoundary::Strom.day_of(t).map(|d| d.year());
    let first = spans.first().ok_or(EegError::Empty)?;
    let y = year(first.0);
    for w in spans.windows(2) {
        if w[1].0 < w[0].1 {
            return Err(EegError::Inconsistent {
                what: "two months overlap",
            });
        }
    }
    if spans.iter().any(|s| year(s.0) != y) {
        return Err(EegError::Inconsistent {
            what: "the months span two calendar years",
        });
    }
    Ok(())
}

/// (22) = ∑J (21) and (33) = ∑J (32): the sums of the months' results —
/// *"Summe über alle Kalendermonate eines Kalenderjahres"* (p. 33).
///
/// # Errors
///
/// [`EegError::Inconsistent`] when the months overlap, span two years, or
/// hold different numbers of plants; [`EegError::Empty`]; overflow.
pub fn jahr(monate: &[Monat]) -> Result<Jahr, EegError> {
    one_year(monate.iter().map(|m| (m.from, m.to)).collect())?;
    let plants = monate.first().map_or(0, |m| m.anlagen.len());
    if monate.iter().any(|m| m.anlagen.len() != plants) {
        return Err(EegError::Inconsistent {
            what: "the months hold different numbers of plants",
        });
    }
    Ok(Jahr {
        f22: sum(monate.iter().map(|m| m.f21))?,
        f33: (0..plants)
            .map(|k| sum(monate.iter().map(|m| m.anlagen[k].f32)))
            .collect::<Result<_, _>>()?,
    })
}

/// [`jahr`] for the single-meter Sonderfälle: (22)A8/A10/A11 and (33)A8.
///
/// # Errors
///
/// As [`jahr`]; also when the months mix Sonderfälle.
pub fn jahr_einzelzaehler(monate: &[EinzelMonat]) -> Result<Jahr, EegError> {
    one_year(monate.iter().map(|m| (m.from, m.to)).collect())?;
    let fall = monate.first().map(|m| m.fall);
    if monate.iter().any(|m| Some(m.fall) != fall) {
        return Err(EegError::Inconsistent {
            what: "the months mix Sonderfälle",
        });
    }
    let f33 = if fall == Some(Abgrenzungsfall::A8) {
        vec![sum(monate.iter().map(|m| m.f32.unwrap_or(Decimal::ZERO)))?]
    } else {
        Vec::new()
    };
    Ok(Jahr {
        f22: sum(monate.iter().map(|m| m.f21))?,
        f33,
    })
}
