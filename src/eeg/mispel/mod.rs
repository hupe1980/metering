//! MiSpeL — *Marktintegration von Speichern und Ladepunkten*, the BNetzA
//! Festlegung of 01.10.2026 (Az. 618-25-02).
//!
//! A storage system or bidirectional charge point that mixes grid and plant
//! electricity keeps its Umlageprivilegien (§ 21 EnFG) and Marktprämie
//! (§ 19 EEG) for the share these formulas attribute to it:
//!
//! - [`abgrenzung`] — **Anlage 1, Abgrenzungsoption.** Quarter-hour
//!   allocation over two meters (Z1 at the connection, Z2 in front of the
//!   storage or charge point), saldiert per **calendar month**: formulas
//!   (1)–(40), cases A1–A11. For the year *"werden die Monatswerte zu
//!   Jahreswerten aufsummiert"* (Anl. 1 p. 14) — [`abgrenzung::jahr`].
//! - [`pauschal`] — **Anlage 2, Pauschaloption.** Flat-rate limits for PV up
//!   to 30 kWp on one meter, per **calendar year** or Rumpfjahr: formulas
//!   (P1)–(P22), cases P1–P5.
//!
//! The caller names the Formelsatz (an operator decision binding for a
//! calendar year, Anl. 1 p. 24, Anl. 2 p. 22) and passes AW¼ > 0 ((24)¼,
//! (P12)¼) and SP¼ ≥ 0 ((P5)¼) as a [`Gate`].
//!
//! **Pauschaloption applicability.** Tenorziffer 9 b): Tenorziffern 2
//! and 4 *"sind frühestens für Netzeinspeisung ab dem ersten Kalendertag des
//! Kalendermonats anwendbar, der auf die beihilferechtliche Genehmigung der
//! Europäischen Kommission zu § 19 Abs. 3c EEG"* follows. Whether that
//! approval exists is the caller's fact; the formulas run regardless.
//!
//! **Out of scope:** the money formulas (41)–(44) of A9, the § 22 EnFG
//! heat-pump quantity of the A7 and P5 footnotes, the unnumbered A10 split.
//!
//! **Rounding.** None: quantities are exact `Decimal`, shares are formed
//! product first and divided last. The published tables round for display
//! only (*"Zur vereinfachten Darstellung sind die Werte gerundet"*, Anl. 2
//! p. 12).

use std::collections::BTreeSet;

use time::OffsetDateTime;

use crate::series::Series;

pub mod abgrenzung;
pub mod pauschal;

/// Which quarter-hours pass a sign test the Festlegung defines on a money
/// value: AW¼ > 0 for (24)¼, (24a)¼, (24b)¼ and (P12)¼; SP¼ ≥ 0 for (P5)¼.
///
/// The boundaries differ (a spot price of zero passes (P5)¼), so each set is
/// passed separately.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gate<'a> {
    /// Every quarter-hour passes. A plant without subsidy has its gate
    /// *"mit dem Wert 1 anzusetzen"* (Anl. 1 Kap. 10.1 p. 92; Anl. 2
    /// Kap. 8 p. 50).
    Always,
    /// Exactly the quarter-hours starting at these instants pass; instants
    /// outside the evaluated period are ignored.
    Flagged(&'a BTreeSet<OffsetDateTime>),
}

impl Gate<'_> {
    /// `true` when the quarter-hour starting at `at` passes.
    #[must_use]
    pub fn passes(&self, at: OffsetDateTime) -> bool {
        match self {
            Self::Always => true,
            Self::Flagged(set) => set.contains(&at),
        }
    }
}

/// Where the feed-in is metered.
#[derive(Debug, Clone, Copy)]
pub enum Einspeisung<'a> {
    /// Z1NE¼ at the connection's Einspeisestelle — the ordinary case.
    Z1(&'a Series),
    /// ZWNE¼ at meter ZW upstream of a separately supplied consumer (a heat
    /// pump or a third party): case A7 (Anl. 1 pp. 69–70) and P5 (Anl. 2
    /// p. 46): *"allein der viertelstündliche Messwert Z1NE¼ durch den
    /// viertelstündlichen Messwert ZWNE¼"* is replaced.
    Zw(&'a Series),
}

impl<'a> Einspeisung<'a> {
    pub(crate) const fn series(self) -> &'a Series {
        match self {
            Self::Z1(s) | Self::Zw(s) => s,
        }
    }

    pub(crate) const fn name(self) -> &'static str {
        match self {
            Self::Z1(_) => "Z1NE",
            Self::Zw(_) => "ZWNE",
        }
    }
}

/// The Formelsätze of Anlage 1 (Abgrenzungsoption), by their published code.
///
/// A1–A4 are the Basisfälle; A5, A6, A7 and A9 are Sonderfälle drawn on top
/// of a Basisfall (*Baukastenprinzip*, Anl. 1 p. 24); A8, A10 and A11 are the
/// single-meter Sonderfälle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Abgrenzungsfall {
    /// A1 — Stromspeicher (pp. 29, 33–41).
    A1,
    /// A2 — Ladepunkt (pp. 29–30).
    A2,
    /// A3 — Stromspeicher und Ladepunkt behind one Z2 (pp. 30–31).
    A3,
    /// A4 — as A3 with a separate storage meter Z3 (pp. 31–32).
    A4,
    /// A5 — several plants of the same kind (pp. 42–54).
    A5,
    /// A6 — a further plant of a different kind, with priority (pp. 54–65).
    A6,
    /// A7 — a separately supplied heat pump or third party, feed-in at ZW
    /// (pp. 66–70).
    A7,
    /// A8 — storage in co-location, single meter (pp. 70–75).
    A8,
    /// A9 — Netzbezug at differing Umlage rates (pp. 75–87). Its own formulas
    /// are money; never reported by an evaluation here.
    A9,
    /// A10 — grid-only storage, single meter (pp. 94–97).
    A10,
    /// A11 — storage and/or charge point without other generation, single
    /// meter (pp. 98–102).
    A11,
}

impl Abgrenzungsfall {
    /// Every case, in the Festlegung's order.
    pub const ALL: [Self; 11] = [
        Self::A1,
        Self::A2,
        Self::A3,
        Self::A4,
        Self::A5,
        Self::A6,
        Self::A7,
        Self::A8,
        Self::A9,
        Self::A10,
        Self::A11,
    ];

    /// The published code.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::A1 => "A1",
            Self::A2 => "A2",
            Self::A3 => "A3",
            Self::A4 => "A4",
            Self::A5 => "A5",
            Self::A6 => "A6",
            Self::A7 => "A7",
            Self::A8 => "A8",
            Self::A9 => "A9",
            Self::A10 => "A10",
            Self::A11 => "A11",
        }
    }
}

/// The Formelsätze of Anlage 2 (Pauschaloption), by their published code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Pauschalfall {
    /// P1 — Stromspeicher (pp. 25, 28–33).
    P1,
    /// P2 — Ladepunkt (pp. 26, 29).
    P2,
    /// P3 — Stromspeicher und Ladepunkt (pp. 26, 29).
    P3,
    /// P4 — several solar plants (pp. 33–42).
    P4,
    /// P5 — a separately supplied heat pump, feed-in at ZW (pp. 42–46).
    P5,
}

impl Pauschalfall {
    /// Every case, in the Festlegung's order.
    pub const ALL: [Self; 5] = [Self::P1, Self::P2, Self::P3, Self::P4, Self::P5];

    /// The published code.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::P1 => "P1",
            Self::P2 => "P2",
            Self::P3 => "P3",
            Self::P4 => "P4",
            Self::P5 => "P5",
        }
    }
}

crate::ids::codes::string_codes! {
    Abgrenzungsfall;
    Pauschalfall;
}
