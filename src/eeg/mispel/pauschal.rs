//! MiSpeL Anlage 2 — the Pauschaloption, one calendar year (or Rumpfjahr)
//! at a time.
//!
//! For PV up to 30 kWp behind one meter Z1. Two flat-rate limits replace the
//! second meter: the **Pauschalgrenze der Förderfähigkeit** (P1), 500 kWh per
//! kWp, and the **Pauschalgrenze der Saldierungsfähigkeit** (P4), which adds
//! the Indifferenzbereich (P3). [`Pauschalgrenzen`] computes them;
//! [`Pauschaloption::jahr`] applies them to a year of quarter-hours. No
//! monthly level: *"stets im Rückblick für das abgelaufene Abrechnungsjahr"*
//! (p. 9).
//!
//! **Rumpfjahr** (Kap. 9, pp. 52–56): each part of a split year is evaluated
//! alone with (P1)R, (P3)R and (P4)R. The caller passes each part's first and
//! last day: *"der Kalendertag, an dem die bestimmungsrelevante Änderung
//! eintritt, zählt zu dem Rumpfjahr vor der Änderung"* (p. 54).
//!
//! Anlage 2 has no privilegierungsfähige Speicherverluste and no
//! Fremdtankstrom.

use rust_decimal::{Decimal, dec};
use time::{Date, Month, OffsetDateTime};

use super::{Einspeisung, Gate, Pauschalfall};
use crate::eeg::{EegError, add, align, mul, share, sub, sum};
use crate::series::Series;
use crate::series::interval::QualityFlag;
use crate::time::calendar::DayBoundary;

/// The 500 kWh/kW of (P1) = Pinst • 500 kWh/kW (p. 28).
pub const FOERDERGRENZE_KWH_JE_KWP: Decimal = dec!(500);

/// The 0,1 kWh/kW of (P2)P1 = 0,1 kWh/kW • Pinst / SKinst (p. 28).
pub const INDIFFERENZ_SPEICHER_KWH_JE_KW: Decimal = dec!(0.1);

/// (P2)P2 = 0,2 (p. 29).
pub const INDIFFERENZ_LADEPUNKT: Decimal = dec!(0.2);

/// (P17) = ANZAHL \[ TS \] = 183 — the days of the Sommerperiode, April to
/// September, in every calendar year (p. 54).
pub const SOMMERTAGE: u16 = 183;

/// What sits behind the meter — the Basisfall, selecting (P2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pauschalbasis {
    /// P1 — storage: (P2)P1 = 0,1 kWh/kW • Pinst / SKinst.
    Stromspeicher {
        /// SKinst — installed capacity of **all** storage systems, kWh.
        kapazitaet_kwh: Decimal,
    },
    /// P2 — charge point: (P2)P2 = 0,2.
    Ladepunkt,
    /// P3 — storage and charge point: (P2)P3 = MIN \[ (P2)P1 ; (P2)P2 \].
    StromspeicherUndLadepunkt {
        /// SKinst — installed capacity of all storage systems (charge points
        /// excluded), kWh.
        kapazitaet_kwh: Decimal,
    },
}

impl Pauschalbasis {
    /// The case code.
    #[must_use]
    pub const fn fall(&self) -> Pauschalfall {
        match self {
            Self::Stromspeicher { .. } => Pauschalfall::P1,
            Self::Ladepunkt => Pauschalfall::P2,
            Self::StromspeicherUndLadepunkt { .. } => Pauschalfall::P3,
        }
    }
}

/// The two flat-rate limits of a calendar year (§ 4.2.2, pp. 28–30).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct Pauschalgrenzen {
    /// Pinst, kWp.
    pub pinst: Decimal,
    /// (P1) = Pinst • 500 kWh/kW — Pauschalgrenze der Förderfähigkeit, kWh/a.
    pub p1: Decimal,
    /// (P2) — the Rechengröße of the case; (P2)P1 is not bounded above
    /// (30 kWp with 4 kWh gives 0,75).
    pub p2: Decimal,
    /// (P3) = (P2) • (P1) — the Indifferenzbereich, kWh/a, product first.
    pub p3: Decimal,
    /// (P4) = (P1) + (P3) — Pauschalgrenze der Saldierungsfähigkeit, kWh/a.
    pub p4: Decimal,
    /// (P3) as numerator ÷ denominator, so (P3)R divides once.
    p3_num: Decimal,
    p3_den: Decimal,
}

impl Pauschalgrenzen {
    /// (P1)–(P4) for `pinst_kwp` — all PV behind the Einspeisestelle,
    /// including PV without subsidy and Steckersolargeräte.
    ///
    /// ```rust
    /// use metering::eeg::mispel::pauschal::{Pauschalbasis, Pauschalgrenzen};
    /// use rust_decimal::dec;
    ///
    /// // Anl. 2 p. 13: 8 kWp, 10 kWh.
    /// let g = Pauschalgrenzen::new(Pauschalbasis::Stromspeicher { kapazitaet_kwh: dec!(10) }, dec!(8))?;
    /// assert_eq!((g.p1, g.p3, g.p4), (dec!(4000), dec!(320), dec!(4320)));
    /// # Ok::<(), metering::eeg::EegError>(())
    /// ```
    ///
    /// # Errors
    ///
    /// [`EegError::ZeroWeight`] for Pinst or SKinst ≤ 0; overflow.
    pub fn new(basis: Pauschalbasis, pinst_kwp: Decimal) -> Result<Self, EegError> {
        if pinst_kwp <= Decimal::ZERO {
            return Err(EegError::ZeroWeight {
                formula: "(P1)",
                what: "Pinst",
            });
        }
        let p1 = mul(pinst_kwp, FOERDERGRENZE_KWH_JE_KWP)?;
        let storage = |sk: Decimal| -> Result<(Decimal, Decimal), EegError> {
            if sk <= Decimal::ZERO {
                return Err(EegError::ZeroWeight {
                    formula: "(P2)P1",
                    what: "SKinst",
                });
            }
            Ok((mul(INDIFFERENZ_SPEICHER_KWH_JE_KW, pinst_kwp)?, sk))
        };
        let (p2_num, p2_den) = match basis {
            Pauschalbasis::Stromspeicher { kapazitaet_kwh } => storage(kapazitaet_kwh)?,
            Pauschalbasis::Ladepunkt => (INDIFFERENZ_LADEPUNKT, Decimal::ONE),
            Pauschalbasis::StromspeicherUndLadepunkt { kapazitaet_kwh } => {
                let (n, d) = storage(kapazitaet_kwh)?;
                // MIN [ n/d ; 0,2 ], compared without dividing.
                if n <= mul(INDIFFERENZ_LADEPUNKT, d)? {
                    (n, d)
                } else {
                    (INDIFFERENZ_LADEPUNKT, Decimal::ONE)
                }
            }
        };
        let p2 = share(p2_num, Decimal::ONE, p2_den)?.ok_or(EegError::Overflow)?;
        let p3_num = mul(p2_num, p1)?;
        let p3 = share(p3_num, Decimal::ONE, p2_den)?.ok_or(EegError::Overflow)?;
        Ok(Self {
            pinst: pinst_kwp,
            p1,
            p2,
            p3,
            p4: add(p1, p3)?,
            p3_num,
            p3_den: p2_den,
        })
    }

    /// (P17)–(P22): the limits of the Rumpfjahr from `erster_tag` to
    /// `letzter_tag`, both inclusive (Kap. 9.2, pp. 54–55).
    ///
    /// (P1)R = TRS • (P1) / 183 follows the summer days, as PV yield does;
    /// (P3)R = TR • (P3) / TK follows all days. Unrounded.
    ///
    /// # Errors
    ///
    /// [`EegError::Calendar`] when the range is reversed, spans two years or
    /// leaves the supported calendar; overflow.
    pub fn rumpfjahr(&self, erster_tag: Date, letzter_tag: Date) -> Result<Rumpfjahr, EegError> {
        let year = erster_tag.year();
        if letzter_tag < erster_tag || letzter_tag.year() != year {
            return Err(EegError::Calendar);
        }
        let days = |a: Date, b: Date| u16::try_from((b - a).whole_days() + 1).ok();
        let tr = days(erster_tag, letzter_tag).ok_or(EegError::Calendar)?;
        let april =
            Date::from_calendar_date(year, Month::April, 1).map_err(|_| EegError::Calendar)?;
        let september =
            Date::from_calendar_date(year, Month::September, 30).map_err(|_| EegError::Calendar)?;
        let (lo, hi) = (erster_tag.max(april), letzter_tag.min(september));
        let trs = if lo <= hi {
            days(lo, hi).ok_or(EegError::Calendar)?
        } else {
            0
        };
        let tk = time::util::days_in_year(year);
        let p1 = share(Decimal::from(trs), self.p1, Decimal::from(SOMMERTAGE))?
            .ok_or(EegError::Overflow)?;
        let p3 = share(
            Decimal::from(tr),
            self.p3_num,
            mul(self.p3_den, Decimal::from(tk))?,
        )?
        .ok_or(EegError::Overflow)?;
        Ok(Rumpfjahr {
            erster_tag,
            letzter_tag,
            tr,
            trs,
            tk,
            p1,
            p3,
            p4: add(p1, p3)?,
        })
    }
}

/// The limits of one Rumpfjahr (Kap. 9).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct Rumpfjahr {
    /// First day (inclusive).
    pub erster_tag: Date,
    /// Last day (inclusive).
    pub letzter_tag: Date,
    /// (P22)R = ANZAHL \[ TR \] — the days of the Rumpfjahr.
    pub tr: u16,
    /// (P19)R = ANZAHL \[ TRS \] — of them, the days April to September.
    pub trs: u16,
    /// (P20) = ANZAHL \[ TK \] — the days of the calendar year.
    pub tk: u16,
    /// (P1)R = (P19)R • (P18), with (P18) = (P1) / (P17).
    pub p1: Decimal,
    /// (P3)R = (P22)R • (P21), with (P21) = (P3) / (P20).
    pub p3: Decimal,
    /// (P4)R = (P1)R + (P3)R.
    pub p4: Decimal,
}

/// The period of one evaluation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Zeitraum {
    /// A whole Berlin calendar year.
    Kalenderjahr(i32),
    /// A Rumpfjahr: first and last day, inclusive, in one calendar year.
    Rumpfjahr {
        /// First day.
        erster_tag: Date,
        /// Last day.
        letzter_tag: Date,
    },
}

/// One solar plant: its installed power and its AW¼ > 0 gate.
#[derive(Debug, Clone, Copy)]
pub struct SolarAnlage<'a> {
    /// Installed power, kWp, > 0.
    pub kwp: Decimal,
    /// (P12)¼ = WENN \[ AW¼ > 0 ; 1 ; 0 \]; [`Gate::Always`] for a plant
    /// without subsidy (Kap. 8).
    pub aw: Gate<'a>,
}

/// The inputs of one year or Rumpfjahr.
#[derive(Debug, Clone)]
pub struct Pauschaloption<'a> {
    /// Z1NB¼.
    pub z1nb: &'a Series,
    /// Z1NE¼, or ZWNE¼ for P5.
    pub einspeisung: Einspeisung<'a>,
    /// (P5)¼ = WENN \[ SP¼ ≥ 0 ; 1 ; 0 \].
    pub sp: Gate<'a>,
    /// What sits behind the meter.
    pub basis: Pauschalbasis,
    /// Every PV plant; Pinst is their sum. Several plants are case P4.
    pub anlagen: Vec<SolarAnlage<'a>>,
    /// The calendar year or Rumpfjahr.
    pub zeitraum: Zeitraum,
}

/// One quarter-hour of [`Pauschaljahr`].
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct PauschalViertelstunde {
    /// Start (UTC).
    pub from: OffsetDateTime,
    /// End (UTC, exclusive).
    pub to: OffsetDateTime,
    /// Z1NB¼.
    pub z1nb: Decimal,
    /// Z1NE¼ (ZWNE¼ in P5).
    pub einspeisung: Decimal,
    /// (P5)¼ as a flag; (P6)¼ = this • Z1NE¼.
    pub sp: bool,
    /// (P12k)¼ per plant; (P13k)¼ = this • Z1NE¼.
    pub aw: Vec<bool>,
    /// The worse of the two input qualities.
    pub quality: QualityFlag,
}

/// One plant's förderfähige Netzeinspeisung (§ 4.2.5; P4: pp. 37–38).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct SolarAnlageJahr {
    /// (P14k) = ∑J (P13k)¼ — feed-in in the plant's AW>0 quarter-hours.
    pub p14: Decimal,
    /// (P15k) = MIN \[ (P14k) ; (P1) \] ((P1)R in a Rumpfjahr).
    pub p15: Decimal,
    /// (P16k) = (ZFk) • (P15k), ZFk = Pkinst / Pinst, product first.
    pub p16: Decimal,
}

/// One calendar year or Rumpfjahr under the Pauschaloption.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Pauschaljahr {
    /// The Formelsätze applied: P1/P2/P3, then P4 and P5 when present.
    pub faelle: Vec<Pauschalfall>,
    /// (P1)–(P4) of the calendar year.
    pub grenzen: Pauschalgrenzen,
    /// (P1)R–(P4)R, in a Rumpfjahr — then they replace (P1) and (P4) below.
    pub rumpfjahr: Option<Rumpfjahr>,
    /// (P7) = ∑J (P6)¼ — feed-in in SP≥0 quarter-hours.
    pub p7: Decimal,
    /// (P8) = MAX \[ (P7) – (P4) ; 0 \].
    pub p8: Decimal,
    /// (P9) = ∑J Z1NB¼ — Netzbezug.
    pub p9: Decimal,
    /// (P10) = MIN \[ (P8) ; (P9) \] — saldierungsfähige Netzeinspeisung.
    pub p10: Decimal,
    /// (P11) = (P9) – (P10) — umlagebelasteter Netzbezug.
    pub p11: Decimal,
    /// Per plant, in input order.
    pub anlagen: Vec<SolarAnlageJahr>,
    /// Every quarter-hour, ascending.
    pub viertelstunden: Vec<PauschalViertelstunde>,
}

impl Pauschaljahr {
    /// Σ (P16k) — the förderfähige Netzeinspeisung of all plants.
    ///
    /// # Errors
    ///
    /// [`EegError::Overflow`].
    pub fn foerderfaehig(&self) -> Result<Decimal, EegError> {
        sum(self.anlagen.iter().map(|a| a.p16))
    }
}

impl Pauschaloption<'_> {
    /// Evaluate the year or Rumpfjahr.
    ///
    /// # Errors
    ///
    /// [`EegError`] when a series is not quarter-hourly, the grids differ, a
    /// value is negative or non-billable, a quarter-hour lies outside the
    /// period, a power or capacity is not positive, no plant is given, or
    /// on overflow.
    pub fn jahr(&self) -> Result<Pauschaljahr, EegError> {
        if self.anlagen.is_empty() {
            return Err(EegError::Inconsistent {
                what: "the Pauschaloption needs at least one solar plant",
            });
        }
        if self.anlagen.iter().any(|a| a.kwp <= Decimal::ZERO) {
            return Err(EegError::ZeroWeight {
                formula: "(ZF)",
                what: "installed power",
            });
        }
        let pinst = sum(self.anlagen.iter().map(|a| a.kwp))?;
        let grenzen = Pauschalgrenzen::new(self.basis, pinst)?;
        let (start, end, rumpfjahr) = match self.zeitraum {
            Zeitraum::Kalenderjahr(y) => {
                let p = DayBoundary::Strom.year(y).ok_or(EegError::Calendar)?;
                (p.start(), p.end(), None)
            }
            Zeitraum::Rumpfjahr {
                erster_tag,
                letzter_tag,
            } => {
                let r = grenzen.rumpfjahr(erster_tag, letzter_tag)?;
                let a = DayBoundary::Strom
                    .day(erster_tag)
                    .ok_or(EegError::Calendar)?;
                let b = DayBoundary::Strom
                    .day(letzter_tag)
                    .ok_or(EegError::Calendar)?;
                (a.start(), b.end(), Some(r))
            }
        };
        let (p1, p4) = rumpfjahr.map_or((grenzen.p1, grenzen.p4), |r| (r.p1, r.p4));

        let rows = align(&[
            ("Z1NB", self.z1nb),
            (self.einspeisung.name(), self.einspeisung.series()),
        ])?;
        if let Some(out) = rows.iter().find(|r| r.from < start || r.to > end) {
            return Err(EegError::OutsidePeriod { at: out.from });
        }
        let viertelstunden: Vec<PauschalViertelstunde> = rows
            .iter()
            .map(|r| PauschalViertelstunde {
                from: r.from,
                to: r.to,
                z1nb: r.values[0],
                einspeisung: r.values[1],
                sp: self.sp.passes(r.from),
                aw: self.anlagen.iter().map(|a| a.aw.passes(r.from)).collect(),
                quality: r.quality,
            })
            .collect();

        let p7 = sum(viertelstunden
            .iter()
            .filter(|q| q.sp)
            .map(|q| q.einspeisung))?;
        let p8 = sub(p7, p4)?.max(Decimal::ZERO);
        let p9 = sum(viertelstunden.iter().map(|q| q.z1nb))?;
        let p10 = p8.min(p9);
        let p11 = sub(p9, p10)?;
        let anlagen = self
            .anlagen
            .iter()
            .enumerate()
            .map(|(k, a)| {
                let p14 = sum(viertelstunden
                    .iter()
                    .filter(|q| q.aw[k])
                    .map(|q| q.einspeisung))?;
                let p15 = p14.min(p1);
                let p16 = share(a.kwp, p15, pinst)?.ok_or(EegError::Overflow)?;
                Ok(SolarAnlageJahr { p14, p15, p16 })
            })
            .collect::<Result<Vec<_>, EegError>>()?;

        let mut faelle = vec![self.basis.fall()];
        if self.anlagen.len() > 1 {
            faelle.push(Pauschalfall::P4);
        }
        if matches!(self.einspeisung, Einspeisung::Zw(_)) {
            faelle.push(Pauschalfall::P5);
        }
        Ok(Pauschaljahr {
            faelle,
            grenzen,
            rumpfjahr,
            p7,
            p8,
            p9,
            p10,
            p11,
            anlagen,
            viertelstunden,
        })
    }
}
