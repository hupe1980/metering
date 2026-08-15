//! Typed market identifiers — [`MaloId`] and [`MeloId`].
//!
//! ## Why these are types and not strings
//!
//! The same reasoning as [`crate::ObisCode`]: an identifier that is a `String`
//! is validated nowhere, so a transposed digit in a Marktlokations-ID becomes a
//! *different, plausible-looking* Marktlokations-ID — a wrong database key, a
//! reading filed against the wrong delivery point, and no error anywhere. The
//! MaLo-ID carries a **check digit** precisely so that this class of mistake is
//! detectable, and a type that does not check it throws that protection away.
//!
//! Parse at the boundary — `"41373559241".parse::<MaloId>()?` — and a mistyped
//! identifier is rejected there, where the message is still available to
//! report.
//!
//! ## MaLo-ID (Marktlokations-Identifikationsnummer)
//!
//! Introduced market-wide on 1 February 2018 (BNetzA BK6-16-200 / BK7-16-142,
//! Interimsmodell). The BDEW Anwendungshilfe *"Die neue
//! Marktlokations-Identifikationsnummer"* (v1.0, 28.04.2017) defines the
//! Bildungsvorschrift:
//!
//! | Position | Content |
//! |---|---|
//! | 1 | Vergabestelle — `1`–`3` DVGW, `4`–`9` BDEW |
//! | 2–10 | digits `0`–`9` |
//! | 11 | check digit |
//!
//! One MaLo-ID identifies one Marktlokation **or one Tranche**, permanently —
//! it survives a Netzbetreiberwechsel, and says nothing about the Sparte.
//!
//! ### The check digit
//!
//! Computed by the *"Lok- und Waggon-Kennzeichnungsverfahren"*, quoted from the
//! Anwendungshilfe §3.2 (with its worked example, which
//! `tests` here pin):
//!
//! 1. sum of the digits in **odd** positions (1, 3, 5, 7, 9);
//! 2. sum of the digits in **even** positions (2, 4, 6, 8, 10), **multiplied
//!    by 2** — the sum is doubled, not each digit (this is *not* the Luhn
//!    algorithm);
//! 3. add the two;
//! 4. the check digit is the difference to the next multiple of 10, and `0`
//!    when that difference is 10.
//!
//! ```text
//! 4 1 3 7 3 5 5 9 2 4 →  a) 4+3+3+5+2 = 17
//!                         b) (1+7+5+9+4) × 2 = 52
//!                         c) 17 + 52 = 69
//!                         d) 70 − 69 = 1  →  41373559241
//! ```
//!
//! One consequence of doubling the *sum* rather than each digit (as Luhn
//! does): the scheme detects every single-digit error **except a ±5 change in
//! an even position**, which shifts the total by exactly 10. It also misses
//! adjacent transpositions whose digits differ by 5. That is the scheme the
//! market defined; this type checks what it can and cannot promise more.
//!
//! ## MeLo-ID (Messlokations-Identifikationsnummer / Zählpunktbezeichnung)
//!
//! A Messlokation is identified by the 33-character Zählpunktbezeichnung of
//! VDE-AR-N 4400 (Strom) / DVGW Arbeitsblatt G 2000 (Gas): a two-letter
//! country code, a six-digit Netzbetreiber number, five characters of
//! Postleitzahl, and twenty alphanumeric characters assigned by the
//! Netzbetreiber. There is **no check digit**, so [`MeloId`] validates the
//! structure — length, charset and the numeric prefix groups — and nothing
//! more.
//!
//! ## What deliberately stays a plain string
//!
//! The keys of [`crate::virtual_meter::SourceMap`] are arbitrary series labels
//! a caller chooses — `"PLANT"`, `"T2"` — and are not asserted to be MaLo-IDs,
//! so they remain `String`.

use std::fmt;
use std::str::FromStr;

use crate::error::ParseError;

// ── issuer ────────────────────────────────────────────────────────────────────

/// Which Codevergabestelle issued a [`MaloId`], read off its first digit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "SCREAMING_SNAKE_CASE"))]
pub enum MaloIssuer {
    /// First digit `1`–`3` — DVGW Service & Consult GmbH.
    Dvgw,
    /// First digit `4`–`9` — Energie Codes und Services GmbH (BDEW).
    Bdew,
}

// ── MaloId ────────────────────────────────────────────────────────────────────

/// A validated 11-digit Marktlokations-Identifikationsnummer.
///
/// Construction goes through [`FromStr`]/[`TryFrom`], which enforce the
/// Bildungsvorschrift **including the check digit** — see the
/// [module docs](self). A constructed value is therefore always well-formed,
/// and [`Display`](fmt::Display) writes the one canonical spelling: the eleven
/// digits, nothing else.
///
/// The identifier also names **Tranchen**, which share the format.
///
/// ```rust
/// use metering::MaloId;
///
/// // The worked example from the BDEW Anwendungshilfe.
/// let malo: MaloId = "41373559241".parse()?;
/// assert_eq!(malo.to_string(), "41373559241");
/// assert_eq!(malo.check_digit(), 1);
///
/// // A transposed digit no longer matches its check digit.
/// assert!("41373559214".parse::<MaloId>().is_err());
/// # Ok::<(), metering::ParseError>(())
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MaloId {
    /// The eleven ASCII digits.
    digits: [u8; 11],
}

/// The shape [`MaloId`] accepts, as rendered in a [`ParseError`].
const MALO_FORMAT: &str =
    "an 11-digit MaLo-ID with a valid check digit, first digit 1-9, e.g. 41373559241";

impl MaloId {
    /// The identifier's length: always eleven digits.
    pub const LEN: usize = 11;

    /// The check digit for ten leading digits, per the Anwendungshilfe §3.2.
    ///
    /// Exposed so an issuing or test system can *complete* an ID; validation of
    /// a full ID is what [`FromStr`] does.
    ///
    /// `None` unless `digits` is exactly ten ASCII digits.
    #[must_use]
    pub fn compute_check_digit(digits: &str) -> Option<u8> {
        let bytes = digits.as_bytes();
        if bytes.len() != 10 || !bytes.iter().all(u8::is_ascii_digit) {
            return None;
        }
        let digit = |i: usize| u32::from(bytes[i] - b'0');
        // a) odd positions (1-based 1,3,5,7,9), b) even positions summed, ×2.
        let odd: u32 = (0..10).step_by(2).map(digit).sum();
        let even: u32 = (1..10).step_by(2).map(digit).sum();
        let total = odd + even * 2;
        // d) difference to the next multiple of 10; 10 becomes 0.
        Some(((10 - (total % 10)) % 10) as u8)
    }

    /// The check digit this ID carries (its eleventh digit).
    #[must_use]
    pub const fn check_digit(&self) -> u8 {
        self.digits[10] - b'0'
    }

    /// Which Codevergabestelle issued this ID, read off the first digit.
    #[must_use]
    pub const fn issuer(&self) -> MaloIssuer {
        match self.digits[0] {
            b'1'..=b'3' => MaloIssuer::Dvgw,
            _ => MaloIssuer::Bdew,
        }
    }

    /// The eleven digits as a `&str`.
    #[must_use]
    pub fn as_str(&self) -> &str {
        // Always valid UTF-8: only ASCII digits are ever stored.
        std::str::from_utf8(&self.digits).unwrap_or_default()
    }
}

impl fmt::Display for MaloId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.pad(self.as_str())
    }
}

impl FromStr for MaloId {
    type Err = ParseError;

    /// Parses eleven digits, ignoring surrounding whitespace.
    ///
    /// Rejects a wrong length, a non-digit, a leading `0` (no Vergabestelle
    /// issues one) and — the case the format exists to catch — a **check-digit
    /// mismatch**.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let err = || ParseError::format("MaloId", s, MALO_FORMAT);
        let t = s.trim();
        let bytes = t.as_bytes();
        if bytes.len() != Self::LEN || !bytes.iter().all(u8::is_ascii_digit) || bytes[0] == b'0' {
            return Err(err());
        }
        let expected = Self::compute_check_digit(&t[..10]).ok_or_else(err)?;
        if bytes[10] - b'0' != expected {
            return Err(err());
        }
        let mut digits = [0u8; Self::LEN];
        digits.copy_from_slice(bytes);
        Ok(Self { digits })
    }
}

impl TryFrom<&str> for MaloId {
    type Error = ParseError;
    fn try_from(s: &str) -> Result<Self, Self::Error> {
        s.parse()
    }
}

impl From<MaloId> for String {
    fn from(id: MaloId) -> String {
        id.as_str().to_owned()
    }
}

// ── MeloId ────────────────────────────────────────────────────────────────────

/// A validated 33-character Messlokations-Identifikationsnummer
/// (Zählpunktbezeichnung).
///
/// Structure per VDE-AR-N 4400 / DVGW G 2000:
///
/// | Position | Content |
/// |---|---|
/// | 1–2 | country code, uppercase letters (`DE`) |
/// | 3–8 | Netzbetreiber number, six digits |
/// | 9–13 | Postleitzahl, five characters |
/// | 14–33 | Zählpunktnummer, twenty alphanumeric characters |
///
/// There is no check digit, so this validates structure only. Lowercase input
/// is accepted and canonicalised to uppercase — the market writes these
/// uppercase, and two casings of one Messlokation must not become two keys.
///
/// ```rust
/// use metering::MeloId;
///
/// let melo: MeloId = "DE00056266802AO6G56M11SN51G21M24S".parse()?;
/// assert_eq!(melo.country(), "DE");
/// assert_eq!(melo.netzbetreiber_nr(), "000562");
///
/// // No check digit exists, so only the structure can be enforced.
/// assert!("not a zaehlpunkt".parse::<MeloId>().is_err());
/// # Ok::<(), metering::ParseError>(())
/// ```
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MeloId {
    value: String,
}

/// The shape [`MeloId`] accepts, as rendered in a [`ParseError`].
const MELO_FORMAT: &str = "a 33-character Zählpunktbezeichnung: 2 uppercase letters, \
     6-digit Netzbetreiber number, then 25 alphanumeric characters (VDE-AR-N 4400)";

impl MeloId {
    /// The identifier's length: always 33 characters.
    pub const LEN: usize = 33;

    /// The identifier as a `&str`, uppercase.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.value
    }

    /// The two-letter country code (positions 1–2), `"DE"` in the German
    /// market.
    #[must_use]
    pub fn country(&self) -> &str {
        &self.value[..2]
    }

    /// The six-digit Netzbetreiber number (positions 3–8).
    #[must_use]
    pub fn netzbetreiber_nr(&self) -> &str {
        &self.value[2..8]
    }
}

impl fmt::Display for MeloId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.pad(&self.value)
    }
}

impl FromStr for MeloId {
    type Err = ParseError;

    /// Parses a 33-character Zählpunktbezeichnung, ignoring surrounding
    /// whitespace and canonicalising to uppercase.
    ///
    /// Enforced: the length, letters in positions 1–2, digits in positions 3–8,
    /// and ASCII alphanumerics throughout. The Postleitzahl group is **not**
    /// required to be numeric — real Zählpunktbezeichnungen exist whose
    /// location group carries letters, and rejecting them would refuse
    /// identifiers the market has already accepted.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let err = || ParseError::format("MeloId", s, MELO_FORMAT);
        let t = s.trim();
        if t.len() != Self::LEN || !t.bytes().all(|b| b.is_ascii_alphanumeric()) {
            return Err(err());
        }
        let value = t.to_ascii_uppercase();
        let bytes = value.as_bytes();
        if !bytes[..2].iter().all(u8::is_ascii_uppercase)
            || !bytes[2..8].iter().all(u8::is_ascii_digit)
        {
            return Err(err());
        }
        Ok(Self { value })
    }
}

impl TryFrom<&str> for MeloId {
    type Error = ParseError;
    fn try_from(s: &str) -> Result<Self, Self::Error> {
        s.parse()
    }
}

impl From<MeloId> for String {
    fn from(id: MeloId) -> String {
        id.value
    }
}

// ── Serde: one string on the wire ─────────────────────────────────────────────

#[cfg(feature = "serde")]
mod serde_impl {
    use super::{MaloId, MeloId};
    use serde::de::{self, Visitor};
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use std::fmt;

    impl Serialize for MaloId {
        /// Writes the canonical eleven-digit string.
        fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
            serializer.collect_str(self)
        }
    }

    struct MaloIdVisitor;

    impl Visitor<'_> for MaloIdVisitor {
        type Value = MaloId;

        fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("an 11-digit MaLo-ID string such as \"41373559241\"")
        }

        fn visit_str<E: de::Error>(self, v: &str) -> Result<MaloId, E> {
            v.parse().map_err(de::Error::custom)
        }
    }

    impl<'de> Deserialize<'de> for MaloId {
        fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
            deserializer.deserialize_str(MaloIdVisitor)
        }
    }

    impl Serialize for MeloId {
        /// Writes the canonical uppercase 33-character string.
        fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
            serializer.collect_str(self)
        }
    }

    struct MeloIdVisitor;

    impl Visitor<'_> for MeloIdVisitor {
        type Value = MeloId;

        fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("a 33-character Zählpunktbezeichnung")
        }

        fn visit_str<E: de::Error>(self, v: &str) -> Result<MeloId, E> {
            v.parse().map_err(de::Error::custom)
        }
    }

    impl<'de> Deserialize<'de> for MeloId {
        fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
            deserializer.deserialize_str(MeloIdVisitor)
        }
    }
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// The worked example from the BDEW Anwendungshilfe §3.2, digit for digit:
    /// 4137355924 → a) 17, b) 52, c) 69, d) 70 − 69 = 1.
    #[test]
    fn the_anwendungshilfe_example_is_reproduced() {
        assert_eq!(MaloId::compute_check_digit("4137355924"), Some(1));
        let malo: MaloId = "41373559241".parse().unwrap();
        assert_eq!(malo.check_digit(), 1);
        assert_eq!(malo.issuer(), MaloIssuer::Bdew);
    }

    /// The algorithm doubles the even-position **sum**, not each digit — it is
    /// not Luhn. An ID that is valid under one and not the other pins the
    /// difference: for 5123869678, this scheme gives 80 − 79 = 1, while Luhn
    /// (digit-summing 2,6,12,12,16 → 2+6+3+3+7) would give a different total.
    #[test]
    fn the_scheme_is_not_luhn() {
        assert_eq!(MaloId::compute_check_digit("5123869678"), Some(1));
        assert!("51238696781".parse::<MaloId>().is_ok());
        // The old, check-digit-invalid spelling this crate itself used to
        // carry in its examples fails — which is the point of the type.
        assert!("51238696780".parse::<MaloId>().is_err());
    }

    /// A total that is already a multiple of 10 takes check digit 0, not 10 —
    /// the "ergibt sich hier 10, wird die Prüfziffer 0 genommen" clause.
    #[test]
    fn a_round_total_gives_check_digit_zero() {
        // 6200000000: odd sum 6, even sum 2 × 2 = 4, total 10 → check digit 0.
        assert_eq!(MaloId::compute_check_digit("6200000000"), Some(0));
        assert!("62000000000".parse::<MaloId>().is_ok());

        // Two ordinary cases, computed by hand against the §3.2 schema.
        // 1000000000: odd 1, even 0 → total 1 → 10 − 1 = 9.
        assert_eq!(MaloId::compute_check_digit("1000000000"), Some(9));
        // 8642097531: odd 8+4+0+7+3 = 22, even (6+2+9+5+1) × 2 = 46 → 68 → 2.
        assert_eq!(MaloId::compute_check_digit("8642097531"), Some(2));
    }

    #[test]
    fn issuer_follows_the_first_digit() {
        // 1–3 DVGW, 4–9 BDEW, per the Bildungsvorschrift.
        let dvgw: MaloId = {
            let check = MaloId::compute_check_digit("1234567890").unwrap();
            format!("1234567890{check}").parse().unwrap()
        };
        assert_eq!(dvgw.issuer(), MaloIssuer::Dvgw);

        let bdew: MaloId = "41373559241".parse().unwrap();
        assert_eq!(bdew.issuer(), MaloIssuer::Bdew);
    }

    #[test]
    fn malformed_malo_ids_are_rejected() {
        for s in [
            "",
            "4137355924",    // ten digits — no check digit
            "413735592411",  // twelve
            "4137355924X",   // non-digit
            "41373559240",   // wrong check digit
            "01373559241",   // leading zero — no Vergabestelle issues one
            "41 37355924 1", // interior whitespace
        ] {
            assert!(s.parse::<MaloId>().is_err(), "{s:?} must not parse");
        }
        // Surrounding whitespace is tolerated, as everywhere in this crate.
        assert!("  41373559241  ".parse::<MaloId>().is_ok());
    }

    #[test]
    fn malo_round_trips_and_orders() {
        let a: MaloId = "41373559241".parse().unwrap();
        assert_eq!(a.to_string().parse::<MaloId>().unwrap(), a);
        assert_eq!(String::from(a), "41373559241");
        assert_eq!(format!("{a:>13}"), "  41373559241", "padding is honoured");

        let err = "nope".parse::<MaloId>().unwrap_err();
        assert_eq!(err.type_name(), "MaloId");
        assert!(err.to_string().contains("check digit"), "{err}");
    }

    // ── MeloId ───────────────────────────────────────────────────────────────

    const MELO: &str = "DE00056266802AO6G56M11SN51G21M24S";

    #[test]
    fn a_real_shaped_melo_id_parses() {
        let melo: MeloId = MELO.parse().unwrap();
        assert_eq!(melo.as_str(), MELO);
        assert_eq!(melo.country(), "DE");
        assert_eq!(melo.netzbetreiber_nr(), "000562");
        assert_eq!(melo.to_string().parse::<MeloId>().unwrap(), melo);
    }

    /// Lowercase input canonicalises to uppercase — one Messlokation, one key.
    #[test]
    fn lowercase_normalises_to_uppercase() {
        let lower = MELO.to_lowercase();
        let melo: MeloId = lower.parse().unwrap();
        assert_eq!(melo.as_str(), MELO);
    }

    #[test]
    fn malformed_melo_ids_are_rejected() {
        for s in [
            "",
            "DE0005626680",                       // too short
            "DE00056266802AO6G56M11SN51G21M24S7", // 34 chars
            "D30005626680AO6G56M11SN51G21M24S5",  // digit in the country code
            "DEX0056266802AO6G56M11SN51G21M24S",  // letter in the NB number
            "DE00056266802AO6G56M11SN51G21M2-S",  // non-alphanumeric
        ] {
            assert!(s.parse::<MeloId>().is_err(), "{s:?} must not parse");
        }
    }
}
