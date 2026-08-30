//! Typed market identifiers — [`MaloId`], [`MeloId`], [`BdewCode`] and [`Eic`].
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
//! The MeLo-ID **is** the Zählpunktbezeichnung. There is no separate type for
//! one: a Messlokation is identified by its Zählpunkt, the two names describe
//! the same thirty-three characters, and giving them two types would produce
//! two columns holding one identifier.
//!
//! ## EIC (Energy Identification Code)
//!
//! [`Eic`] is the sixteen-character ENTSO-E identifier — the one a Bilanzkreis,
//! a Bilanzierungsgebiet, a Regelzone and a Metering Grid Area are addressed
//! by. It carries a check character, and unlike the BDEW-Codenummer the scheme
//! has no exception to it, so [`Eic`] enforces it at the parse.
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

impl MaloIssuer {
    /// Every Vergabestelle, in declaration order.
    pub const ALL: [Self; 2] = [Self::Dvgw, Self::Bdew];

    /// Stable DB/wire label. Matches the `serde` tag and `FromStr` input.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Dvgw => "DVGW",
            Self::Bdew => "BDEW",
        }
    }
}

crate::codes::string_codes! {
    MaloIssuer;
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
        (digits.len() == 10)
            .then(|| lok_waggon_check_digit(digits))
            .flatten()
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

/// Accepts an owned `String`, which a generic `TryInto<MaloId>` bound does not
/// get from `TryFrom<&str>` by deref coercion.
impl TryFrom<String> for MaloId {
    type Error = ParseError;
    fn try_from(s: String) -> Result<Self, Self::Error> {
        s.parse()
    }
}

impl From<MaloId> for String {
    fn from(id: MaloId) -> String {
        id.as_str().to_owned()
    }
}

// ── the Lok- und Waggon-Kennzeichnungsverfahren ───────────────────────────────

/// The check digit for a run of leading digits, by the *Lok- und
/// Waggon-Kennzeichnungsverfahren*.
///
/// BDEW *Identifikatoren in der Marktkommunikation*, §6.1, verbatim:
///
/// > a) Quersumme aller Ziffern in ungerader Position
/// > b) Quersumme aller Ziffern auf gerader Position multipliziert mit 2
/// > c) Summe von a) und b)
/// > d) Differenz von c) zum nächsten Vielfachen von 10 (ergibt sich hier 10,
/// >    wird die Prüfziffer 0 genommen)
///
/// One implementation, because the same document applies it to **three**
/// identifiers: the MaLo-ID (§3.3), the BDEW-/DVGW-Codenummer (§2.3) and the
/// NeLo-ID (§4.3). Note it is *not* Luhn: the even-position **sum** is doubled,
/// not each digit.
///
/// `None` unless every byte is an ASCII digit.
fn lok_waggon_check_digit(digits: &str) -> Option<u8> {
    let bytes = digits.as_bytes();
    if bytes.is_empty() || !bytes.iter().all(u8::is_ascii_digit) {
        return None;
    }
    let digit = |i: usize| u32::from(bytes[i] - b'0');
    let odd: u32 = (0..bytes.len()).step_by(2).map(digit).sum();
    let even: u32 = (1..bytes.len()).step_by(2).map(digit).sum();
    let total = odd + even * 2;
    Some(((10 - (total % 10)) % 10) as u8)
}

// ── BdewCode ──────────────────────────────────────────────────────────────────

/// Which Codevergabestelle issued a [`BdewCode`], read off its first two
/// digits.
///
/// BDEW *Identifikatoren in der Marktkommunikation* v1.2, §2.2, prints the
/// Bildungsvorschrift as a table: positions 1+2 are the
/// *Vergabestelle/Sparte*, `99` = BDEW/Strom and `98` = DVGW/Gas.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "SCREAMING_SNAKE_CASE"))]
pub enum CodeVergabestelle {
    /// `99` — BDEW / Energie Codes und Services GmbH, Sparte Strom.
    BdewStrom,
    /// `98` — DVGW Service & Consult GmbH, Sparte Gas.
    DvgwGas,
    /// Anything else, which in practice is a GS1 **GLN** used as a
    /// Marktpartner-ID.
    ///
    /// A documented case rather than an unknown one: the same Anwendungshilfe
    /// §2.3 says *"Bei einer von GS1 vergebenen GLN (= Globale
    /// Lokationsnummer) gilt das von GS1 verwendete Prüfzifferverfahren"*, so
    /// such a code is well-formed and simply not issued by BDEW or DVGW.
    Gs1OrOther,
}

impl CodeVergabestelle {
    /// Every variant, in declaration order.
    pub const ALL: [Self; 3] = [Self::BdewStrom, Self::DvgwGas, Self::Gs1OrOther];

    /// Stable DB/wire label. Matches the `serde` tag and
    /// [`FromStr`] input.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::BdewStrom => "BDEW_STROM",
            Self::DvgwGas => "DVGW_GAS",
            Self::Gs1OrOther => "GS1_OR_OTHER",
        }
    }
}

crate::codes::string_codes! {
    CodeVergabestelle;
}

/// A 13-digit **Marktpartner-Identifikationsnummer** — the BDEW- or
/// DVGW-Codenummer every market participant is addressed by.
///
/// The identifier MSCONS carries in `NAD+MS`/`NAD+MR`, UTILMD in the
/// Marktpartner segments, and every Netzbetreiber publishes on its price
/// sheet. It is what makes a Zählzeitdefinition attributable: an
/// NB-assigned `HT/NT-1` means nothing until you know *whose*.
///
/// # Bildungsvorschrift
///
/// BDEW *Identifikatoren in der Marktkommunikation* v1.2, §2.2:
///
/// | Position | Content |
/// |---|---|
/// | 1–2 | Vergabestelle/Sparte — `99` BDEW/Strom, `98` DVGW/Gas |
/// | 3 | `0`–`8` for BDEW, `0`–`9` for DVGW |
/// | 4–12 | digits `0`–`9` |
/// | 13 | Prüfziffer |
///
/// # The check digit is verified, but not enforced
///
/// §2.3 says the Prüfziffer uses the same *Lok- und
/// Waggon-Kennzeichnungsverfahren* as the [`MaloId`] — and then carves out an
/// exception in the next sentence: *"Bei einer von GS1 vergebenen GLN
/// (= Globale Lokationsnummer) gilt das von GS1 verwendete
/// Prüfzifferverfahren."*
///
/// So a well-formed Marktpartner-ID may legitimately fail the BDEW procedure,
/// and this type **parses it anyway**. [`has_bdew_check_digit`] reports the
/// outcome instead, so a caller can warn without a library refusing data the
/// market issued. This is the same restraint the crate applies wherever a rule
/// cannot be verified end to end — [`MaloId`], whose Bildungsvorschrift has no
/// such carve-out and whose worked example is printed in the same document, is
/// checked at the parse and rejects a mismatch.
///
/// [`has_bdew_check_digit`]: Self::has_bdew_check_digit
///
/// ```rust
/// use metering::{BdewCode, CodeVergabestelle};
///
/// let nb: BdewCode = "9900987654321".parse()?;
/// assert_eq!(nb.vergabestelle(), CodeVergabestelle::BdewStrom);
/// assert_eq!(nb.to_string(), "9900987654321");
///
/// // Twelve digits plus the computed thirteenth is always self-consistent.
/// let check = BdewCode::compute_check_digit("990098765432").unwrap();
/// let consistent: BdewCode = format!("990098765432{check}").parse()?;
/// assert!(consistent.has_bdew_check_digit());
/// # Ok::<(), metering::ParseError>(())
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BdewCode {
    /// The thirteen ASCII digits.
    digits: [u8; 13],
}

/// The shape [`BdewCode`] accepts, as rendered in a [`ParseError`].
const BDEW_CODE_FORMAT: &str =
    "a 13-digit BDEW/DVGW Marktpartner-ID, e.g. 9900987654321 (99 = BDEW/Strom, 98 = DVGW/Gas)";

impl BdewCode {
    /// The identifier's length: always thirteen digits.
    pub const LEN: usize = 13;

    /// The check digit for the twelve leading digits, by the *Lok- und
    /// Waggon-Kennzeichnungsverfahren* of the Anwendungshilfe §6.1.
    ///
    /// `None` unless `digits` is exactly twelve ASCII digits. Does not apply to
    /// a GS1-issued GLN — see the [type docs](Self).
    #[must_use]
    pub fn compute_check_digit(digits: &str) -> Option<u8> {
        (digits.len() == 12)
            .then(|| lok_waggon_check_digit(digits))
            .flatten()
    }

    /// The check digit this code carries (its thirteenth digit).
    #[must_use]
    pub const fn check_digit(&self) -> u8 {
        self.digits[12] - b'0'
    }

    /// `true` when the thirteenth digit matches the BDEW procedure.
    ///
    /// **Advisory.** `false` does not mean the code is invalid: a GS1-issued
    /// GLN uses a different procedure and is a legitimate Marktpartner-ID. Use
    /// it to raise a warning, never to reject.
    #[must_use]
    pub fn has_bdew_check_digit(&self) -> bool {
        let Some(expected) = Self::compute_check_digit(&self.as_str()[..12]) else {
            return false;
        };
        expected == self.check_digit()
    }

    /// Which Codevergabestelle issued this code, read off the first two digits.
    #[must_use]
    pub const fn vergabestelle(&self) -> CodeVergabestelle {
        match (self.digits[0], self.digits[1]) {
            (b'9', b'9') => CodeVergabestelle::BdewStrom,
            (b'9', b'8') => CodeVergabestelle::DvgwGas,
            _ => CodeVergabestelle::Gs1OrOther,
        }
    }

    /// The thirteen digits as a `&str`.
    #[must_use]
    pub fn as_str(&self) -> &str {
        // Always valid UTF-8: only ASCII digits are ever stored.
        std::str::from_utf8(&self.digits).unwrap_or_default()
    }
}

impl fmt::Display for BdewCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.pad(self.as_str())
    }
}

impl FromStr for BdewCode {
    type Err = ParseError;

    /// Parses thirteen digits, ignoring surrounding whitespace.
    ///
    /// The **check digit is not enforced** — see the [type docs](Self).
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let t = s.trim();
        let bytes = t.as_bytes();
        if bytes.len() != Self::LEN || !bytes.iter().all(u8::is_ascii_digit) {
            return Err(ParseError::format("BdewCode", s, BDEW_CODE_FORMAT));
        }
        let mut digits = [0u8; Self::LEN];
        digits.copy_from_slice(bytes);
        Ok(Self { digits })
    }
}

impl TryFrom<&str> for BdewCode {
    type Error = ParseError;
    fn try_from(s: &str) -> Result<Self, Self::Error> {
        s.parse()
    }
}

/// Accepts an owned `String`, which a generic `TryInto<BdewCode>` bound does not
/// get from `TryFrom<&str>` by deref coercion.
impl TryFrom<String> for BdewCode {
    type Error = ParseError;
    fn try_from(s: String) -> Result<Self, Self::Error> {
        s.parse()
    }
}

impl From<BdewCode> for String {
    fn from(id: BdewCode) -> String {
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

/// Accepts an owned `String`, which a generic `TryInto<MeloId>` bound does not
/// get from `TryFrom<&str>` by deref coercion.
impl TryFrom<String> for MeloId {
    type Error = ParseError;
    fn try_from(s: String) -> Result<Self, Self::Error> {
        s.parse()
    }
}

impl From<MeloId> for String {
    fn from(id: MeloId) -> String {
        id.value
    }
}

// ── Eic ───────────────────────────────────────────────────────────────────────

/// What an [`Eic`] identifies, read off its third character.
///
/// The closed list of ENTSO-E *EIC Reference Manual* §4.2. The code **is** the
/// letter, because that is the letter the market writes and reads: an EIC is
/// quoted as `10YDE-VE-------2`, and calling the `Y` `AREA` in one place and
/// `Y` in another would be the second spelling this crate refuses everywhere
/// else. [`name`](Self::name) carries the description.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum EicType {
    /// `X` — a **party**: a market participant, TSO, supplier, trader,
    /// Bilanzkreisverantwortlicher. German Bilanzkreise carry a type `X` code,
    /// which the manual calls out as a national usage that remains valid.
    #[cfg_attr(feature = "serde", serde(rename = "X"))]
    Party,
    /// `Y` — an **area or domain**: a bidding zone, a control area, a
    /// Bilanzierungsgebiet, a Metering Grid Area.
    #[cfg_attr(feature = "serde", serde(rename = "Y"))]
    Area,
    /// `Z` — a **measurement point**.
    #[cfg_attr(feature = "serde", serde(rename = "Z"))]
    MeasurementPoint,
    /// `W` — a **resource object**: a generation, consumption or storage unit.
    /// Passive grid elements are type `T`.
    #[cfg_attr(feature = "serde", serde(rename = "W"))]
    ResourceObject,
    /// `T` — a **tie line** or other connecting object: interconnectors,
    /// lines, busbar couplers, transformers.
    #[cfg_attr(feature = "serde", serde(rename = "T"))]
    TieLine,
    /// `V` — a **location**, physical or logical, or an IT system.
    #[cfg_attr(feature = "serde", serde(rename = "V"))]
    Location,
    /// `A` — a **substation** or topological node.
    #[cfg_attr(feature = "serde", serde(rename = "A"))]
    Substation,
}

impl EicType {
    /// Every variant, in declaration order.
    pub const ALL: [Self; 7] = [
        Self::Party,
        Self::Area,
        Self::MeasurementPoint,
        Self::ResourceObject,
        Self::TieLine,
        Self::Location,
        Self::Substation,
    ];

    /// The EIC object-type letter. Matches the `serde` tag and
    /// [`FromStr`] input.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Party => "X",
            Self::Area => "Y",
            Self::MeasurementPoint => "Z",
            Self::ResourceObject => "W",
            Self::TieLine => "T",
            Self::Location => "V",
            Self::Substation => "A",
        }
    }

    /// The manual's own description of the type.
    ///
    /// A description, not a code — see [`as_str`](Self::as_str).
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Party => "Party",
            Self::Area => "Area or Domain",
            Self::MeasurementPoint => "Measurement point",
            Self::ResourceObject => "Resource object",
            Self::TieLine => "Tie-line",
            Self::Location => "Location",
            Self::Substation => "Substation",
        }
    }

    /// The type a letter names, or `None` for a letter the manual does not
    /// list.
    #[must_use]
    pub const fn from_letter(letter: u8) -> Option<Self> {
        match letter {
            b'X' => Some(Self::Party),
            b'Y' => Some(Self::Area),
            b'Z' => Some(Self::MeasurementPoint),
            b'W' => Some(Self::ResourceObject),
            b'T' => Some(Self::TieLine),
            b'V' => Some(Self::Location),
            b'A' => Some(Self::Substation),
            _ => None,
        }
    }
}

crate::codes::string_codes! {
    EicType;
}

/// One of the four German **Regelzonen**, read off position 4 of a
/// Bilanzierungsgebiet's EIC.
///
/// BDEW *Anwendungshilfe Energy Identification Codes* v1.0 (18.12.2017) §2.2.2
/// prints the Bildungsvorschrift for a Bilanzierungsgebiet as its own table:
/// position 4 identifies the Regelzone the area lies in, with `N` TenneT,
/// `R` Amprion, `V` 50Hertz and `W` TransnetBW.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "SCREAMING_SNAKE_CASE"))]
pub enum Regelzone {
    /// `N` — TenneT TSO GmbH.
    ///
    /// Renamed explicitly: `SCREAMING_SNAKE_CASE` would split the operator's
    /// own inner capital into `TENNE_T`.
    #[cfg_attr(feature = "serde", serde(rename = "TENNET"))]
    TenneT,
    /// `R` — Amprion GmbH.
    Amprion,
    /// `V` — 50Hertz Transmission GmbH.
    FiftyHertz,
    /// `W` — TransnetBW GmbH.
    TransnetBw,
}

impl Regelzone {
    /// Every variant, in declaration order.
    pub const ALL: [Self; 4] = [
        Self::TenneT,
        Self::Amprion,
        Self::FiftyHertz,
        Self::TransnetBw,
    ];

    /// Stable DB/wire label. Matches the `serde` tag and [`FromStr`] input.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::TenneT => "TENNET",
            Self::Amprion => "AMPRION",
            Self::FiftyHertz => "FIFTY_HERTZ",
            Self::TransnetBw => "TRANSNET_BW",
        }
    }

    /// The operator's own spelling.
    ///
    /// A description, not a code — `50Hertz` starts with a digit and
    /// `TransnetBW` mixes case, neither of which belongs in a database column.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::TenneT => "TenneT TSO",
            Self::Amprion => "Amprion",
            Self::FiftyHertz => "50Hertz Transmission",
            Self::TransnetBw => "TransnetBW",
        }
    }

    /// The letter this Regelzone occupies position 4 of a Bilanzierungsgebiet
    /// EIC with.
    #[must_use]
    pub const fn eic_letter(self) -> char {
        match self {
            Self::TenneT => 'N',
            Self::Amprion => 'R',
            Self::FiftyHertz => 'V',
            Self::TransnetBw => 'W',
        }
    }

    /// The Regelzone a position-4 letter names, or `None` for any other letter.
    #[must_use]
    pub const fn from_eic_letter(letter: u8) -> Option<Self> {
        match letter {
            b'N' => Some(Self::TenneT),
            b'R' => Some(Self::Amprion),
            b'V' => Some(Self::FiftyHertz),
            b'W' => Some(Self::TransnetBw),
            _ => None,
        }
    }

    /// The ENTSO-E **control-area** code for this Regelzone.
    ///
    /// A `Y` code issued by the Central Issuing Office (LIO `10`), and a
    /// different thing from the Bilanzierungsgebiet codes the BDEW issues
    /// under LIO `11` — this one identifies the whole Regelzone in ENTSO-E
    /// schedules and publications. The bodies still carry the pre-2010 company
    /// names (`EON`, `RWENET`, `VE`, `ENBW`), which is why they are worth
    /// having as constants rather than being guessed from the operator's
    /// current name.
    #[must_use]
    pub const fn control_area_eic(self) -> Eic {
        Eic {
            chars: match self {
                Self::TenneT => *b"10YDE-EON------1",
                Self::Amprion => *b"10YDE-RWENET---I",
                Self::FiftyHertz => *b"10YDE-VE-------2",
                Self::TransnetBw => *b"10YDE-ENBW-----N",
            },
        }
    }
}

crate::codes::string_codes! {
    Regelzone;
}

/// The shape [`Eic`] accepts, as rendered in a [`ParseError`].
const EIC_FORMAT: &str = "a 16-character EIC: 2 characters of issuing office, an object-type \
     letter, 12 characters of `0-9`, `A-Z` or `-`, and a check character \
     (ENTSO-E EIC Reference Manual)";

/// A check-character-validated **Energy Identification Code**.
///
/// The identifier every ENTSO-E-connected object is addressed by, and the one
/// the German market uses for Bilanzkreise, Bilanzierungsgebiete, Regelzonen
/// and Metering Grid Areas. It is not a BDEW identifier and shares nothing
/// with one: a [`BdewCode`] is thirteen digits and addresses a *Marktpartner*,
/// an EIC is sixteen alphanumerics and addresses whatever its type letter
/// says.
///
/// # Bildungsvorschrift
///
/// ENTSO-E *The Energy Identification Coding Scheme (EIC) Reference Manual*,
/// §5.2–5.3:
///
/// | Position | Content |
/// |---|---|
/// | 1–2 | the Local Issuing Office, assigned by the Central Issuing Office |
/// | 3 | the object type — see [`EicType`] |
/// | 4–15 | twelve characters assigned by the LIO |
/// | 16 | the check character |
///
/// Permitted characters are *"numbers (0 to 9), capital letters (A to Z,
/// English alphabet) and the sign minus (-)"*, and the check character is
/// restricted further: *"To avoid confusion, the check character shall use
/// numbers (0 to 9) or the capital letters (A to Z)"* — never the minus. The
/// BDEW *Anwendungshilfe Energy Identification Codes* v1.0 (18.12.2017) §2.2.1
/// prints the same two rows for the German market, and adds that positions 1–2
/// are `11` for the BDEW as the German LIO — see [`GERMAN_LIO`](Self::GERMAN_LIO)
/// and [`regelzone`](Self::regelzone).
///
/// # The check character is enforced
///
/// Unlike [`BdewCode`], whose Bildungsvorschrift carves out GS1-issued GLNs
/// and whose check digit is therefore only reported, the EIC scheme has no
/// exception: every code the CIO or a LIO issues satisfies the algorithm, and
/// the manual prints two worked examples that this crate pins as tests. So a
/// mistyped EIC is rejected at the parse, where the message that carried it is
/// still available to report — the same treatment [`MaloId`] gets.
///
/// ## The algorithm
///
/// Each of the first fifteen characters takes a value (`0`–`9` → 0–9, `A`–`Z`
/// → 10–35, `-` → 36), is multiplied by a weight running 16 down to 2, and the
/// products are summed. The check character is the character whose value is
/// `36 − ((Σ − 1) mod 37)`. A result of 36 would be a minus sign, which the
/// manual forbids as a check character, so such a code is never issued.
///
/// ```rust
/// use metering::ids::{Eic, EicType};
///
/// // The 50Hertz control area — a type `Y` code.
/// let regelzone: Eic = "10YDE-VE-------2".parse()?;
/// assert_eq!(regelzone.object_type(), Some(EicType::Area));
/// assert_eq!(regelzone.issuing_office(), "10");
/// assert_eq!(regelzone.check_character(), '2');
///
/// // A transposed pair is a different, plausible-looking code — and is caught.
/// assert!("10YED-VE-------2".parse::<Eic>().is_err());
/// # Ok::<(), metering::ParseError>(())
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Eic {
    /// The sixteen ASCII characters.
    chars: [u8; 16],
}

impl Eic {
    /// The identifier's length: always 16 characters.
    pub const LEN: usize = 16;

    /// The code as a `&str`, uppercase.
    #[must_use]
    pub fn as_str(&self) -> &str {
        // Every byte was checked to be an ASCII digit, `A`-`Z` or `-` at
        // parse, so this is infallible; the fallback keeps the crate free of
        // `unsafe` and of a panic path, which `#![deny(unsafe_code)]` and the
        // no-panic norm both want.
        std::str::from_utf8(&self.chars).unwrap_or("")
    }

    /// The Local Issuing Office (positions 1–2).
    ///
    /// Returned as a `&str` rather than a typed enum: the CIO assigns these,
    /// publishes the list on the EIC website, and adds to it — pinning a
    /// closed set here would reject codes from an office registered after this
    /// crate was released. The manual says only *"the 2-characters identifying
    /// the LIO"*, so this type does not require them to be digits either,
    /// though every office assigned so far uses two.
    #[must_use]
    pub fn issuing_office(&self) -> &str {
        self.as_str().get(..2).unwrap_or("")
    }

    /// What this code identifies, or `None` for a type letter the manual's
    /// list does not contain.
    ///
    /// `Option` rather than a parse failure, for the reason
    /// [`BdewCode::has_bdew_check_digit`] is advisory: the type list is
    /// ENTSO-E's to extend, and a library that hard-fails on a letter added
    /// after its release rejects data the market has already issued. The
    /// *shape* is still enforced — position 3 must be an uppercase letter.
    ///
    /// A caller whose own downstream rejects unlisted types gates on
    /// `object_type().is_none()` and reports it there, where the message that
    /// carried the code is still in hand. This crate does not make that
    /// decision on their behalf, in either direction.
    #[must_use]
    pub const fn object_type(&self) -> Option<EicType> {
        EicType::from_letter(self.chars[2])
    }

    /// The check character (position 16).
    #[must_use]
    pub const fn check_character(&self) -> char {
        self.chars[15] as char
    }

    /// The Local Issuing Office of the German electricity market: `11`, the
    /// BDEW.
    ///
    /// BDEW *Anwendungshilfe Energy Identification Codes* v1.0 §2.2.1: *"die
    /// Zahl „11" steht für das deutsche LIO im Strommarkt, den BDEW
    /// Bundesverband der Energie- und Wasserwirtschaft e.V."*
    pub const GERMAN_LIO: &'static str = "11";

    /// `true` when this code was issued by the German LIO.
    #[must_use]
    pub fn is_german(&self) -> bool {
        self.issuing_office() == Self::GERMAN_LIO
    }

    /// The **Regelzone** this code names, when it is a German
    /// Bilanzierungsgebiet.
    ///
    /// A Bilanzierungsgebiet is a `Y` code in the EIC function *Metering Grid
    /// Area*, and the BDEW Bildungsvorschrift (*Anwendungshilfe* v1.0 §2.2.2)
    /// gives position 4 a meaning no other EIC has: it identifies the
    /// Regelzone the area lies in.
    ///
    /// The EIC *function* is registry metadata and is not encoded in the code
    /// itself, so a `Y` code cannot in general be told apart from a
    /// Bilanzkreis's — but this one can, because the same section adds a
    /// Praxishinweis that the Energie Codes und Services GmbH **excludes**
    /// `N`, `R`, `V` and `W` at position 4 for every other `Y` function. So a
    /// German `Y` code carrying one of those four letters there is a
    /// Bilanzierungsgebiet, and `Some` here is that inference.
    ///
    /// `None` for a code from another issuing office, a non-`Y` type, or any
    /// other position-4 letter.
    ///
    /// ```rust
    /// use metering::ids::{Eic, Regelzone};
    ///
    /// // A Bilanzierungsgebiet in the Amprion Regelzone. (The body here is
    /// // illustrative; the real ones are published per Regelzone.)
    /// let bg: Eic = "11YR-AMPRION-BG9".parse()?;
    /// assert_eq!(bg.regelzone(), Some(Regelzone::Amprion));
    /// assert!(bg.is_german());
    ///
    /// // The Regelzone's own ENTSO-E control-area code is a different code.
    /// assert_eq!(
    ///     Regelzone::Amprion.control_area_eic().to_string(),
    ///     "10YDE-RWENET---I",
    /// );
    ///
    /// // A control-area code comes from the ENTSO-E CIO, not the BDEW, so it
    /// // carries no Bilanzierungsgebiet Regelzone letter.
    /// assert_eq!(Regelzone::Amprion.control_area_eic().regelzone(), None);
    /// # Ok::<(), metering::ParseError>(())
    /// ```
    #[must_use]
    pub const fn regelzone(&self) -> Option<Regelzone> {
        if self.chars[0] != b'1' || self.chars[1] != b'1' || self.chars[2] != b'Y' {
            return None;
        }
        Regelzone::from_eic_letter(self.chars[3])
    }

    /// The check character for the first fifteen characters of a code.
    ///
    /// `None` when `body` is not exactly fifteen permitted characters, and
    /// when the algorithm yields 36 — the minus sign, which the manual forbids
    /// as a check character, so no such code is issued.
    ///
    /// ```rust
    /// use metering::ids::Eic;
    ///
    /// // The two worked examples printed in the EIC Reference Manual §5.1.
    /// assert_eq!(Eic::compute_check_character("10X168Y4E6H0041"), Some('Z'));
    /// assert_eq!(Eic::compute_check_character("10X---ENTSOE---"), Some('L'));
    /// ```
    #[must_use]
    pub fn compute_check_character(body: &str) -> Option<char> {
        let bytes = body.as_bytes();
        if bytes.len() != Self::LEN - 1 {
            return None;
        }
        let mut sum: u32 = 0;
        for (i, byte) in bytes.iter().enumerate() {
            let value = eic_value(*byte)?;
            // Weights run 16 down to 2 across the fifteen characters.
            sum += value * (16 - i as u32);
        }
        let check = 36 - ((sum + 36) % 37);
        eic_char(check)
    }
}

/// The numeric value of one EIC character: `0`-`9` → 0–9, `A`-`Z` → 10–35,
/// `-` → 36. `None` for anything else.
const fn eic_value(byte: u8) -> Option<u32> {
    match byte {
        b'0'..=b'9' => Some((byte - b'0') as u32),
        b'A'..=b'Z' => Some((byte - b'A') as u32 + 10),
        b'-' => Some(36),
        _ => None,
    }
}

/// The inverse of [`eic_value`], restricted to what a check character may be.
///
/// `None` for 36, whose character is the minus sign — forbidden as a check
/// character by §5.2, so a body that computes to it is never issued a code.
const fn eic_char(value: u32) -> Option<char> {
    match value {
        0..=9 => Some((b'0' + value as u8) as char),
        10..=35 => Some((b'A' + (value - 10) as u8) as char),
        _ => None,
    }
}

impl fmt::Display for Eic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.pad(self.as_str())
    }
}

impl FromStr for Eic {
    type Err = ParseError;

    /// Parses a 16-character EIC, ignoring surrounding whitespace and
    /// canonicalising to uppercase.
    ///
    /// Enforced: the length, the permitted character set, an uppercase letter
    /// in position 3, and the check character. The issuing office is not
    /// required to be numeric — see [`Eic::issuing_office`].
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let err = || ParseError::format("Eic", s, EIC_FORMAT);
        let trimmed = s.trim();
        if trimmed.len() != Self::LEN {
            return Err(err());
        }
        let upper = trimmed.to_ascii_uppercase();
        let bytes = upper.as_bytes();
        if !bytes.iter().all(|b| eic_value(*b).is_some()) || !bytes[2].is_ascii_uppercase() {
            return Err(err());
        }
        if Self::compute_check_character(&upper[..Self::LEN - 1]) != Some(bytes[15] as char) {
            return Err(err());
        }
        let mut chars = [0u8; 16];
        chars.copy_from_slice(bytes);
        Ok(Self { chars })
    }
}

impl TryFrom<&str> for Eic {
    type Error = ParseError;
    fn try_from(s: &str) -> Result<Self, Self::Error> {
        s.parse()
    }
}

/// Accepts an owned `String`, which a generic `TryInto<Eic>` bound does not
/// get from `TryFrom<&str>` by deref coercion.
impl TryFrom<String> for Eic {
    type Error = ParseError;
    fn try_from(s: String) -> Result<Self, Self::Error> {
        s.parse()
    }
}

impl From<Eic> for String {
    fn from(id: Eic) -> String {
        id.as_str().to_owned()
    }
}

// ── Serde: one string on the wire ─────────────────────────────────────────────

#[cfg(feature = "serde")]
mod serde_impl {
    use super::{BdewCode, Eic, MaloId, MeloId};
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

    impl Serialize for BdewCode {
        /// Writes the canonical thirteen-digit string.
        fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
            serializer.collect_str(self)
        }
    }

    struct BdewCodeVisitor;

    impl Visitor<'_> for BdewCodeVisitor {
        type Value = BdewCode;

        fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("a 13-digit Marktpartner-ID such as \"9900987654321\"")
        }

        fn visit_str<E: de::Error>(self, v: &str) -> Result<BdewCode, E> {
            v.parse().map_err(de::Error::custom)
        }
    }

    impl<'de> Deserialize<'de> for BdewCode {
        fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
            deserializer.deserialize_str(BdewCodeVisitor)
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

    impl Serialize for Eic {
        /// Writes the canonical uppercase 16-character string.
        fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
            serializer.collect_str(self)
        }
    }

    struct EicVisitor;

    impl Visitor<'_> for EicVisitor {
        type Value = Eic;

        fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("a 16-character EIC such as \"10YDE-VE-------2\"")
        }

        fn visit_str<E: de::Error>(self, v: &str) -> Result<Eic, E> {
            v.parse().map_err(de::Error::custom)
        }
    }

    impl<'de> Deserialize<'de> for Eic {
        fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
            deserializer.deserialize_str(EicVisitor)
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
        // ...while the same digits with a wrong check digit do not — which is
        // the point of the type.
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

    // ── Eic ───────────────────────────────────────────────────────────────

    /// The two examples the EIC Reference Manual §5.1 prints, and three
    /// German codes in daily use. A check-digit implementation that is wrong
    /// is worse than none, so it is pinned against codes nobody here invented.
    #[test]
    fn the_published_eic_examples_are_reproduced() {
        for code in [
            "10X168Y4E6H0041Z", // manual §5.1, "random non-significant"
            "10X---ENTSOE---L", // manual §5.1, "non-random significant"
            "10YDE-VE-------2", // 50Hertz Regelzone
            "10YDE-ENBW-----N", // TransnetBW Regelzone
            "10YDE-RWENET---I", // Amprion Regelzone
            "10YDE-EON------1", // TenneT TSO Regelzone
            "10Y1001A1001A83F", // the German bidding zone
        ] {
            let eic: Eic = code.parse().unwrap_or_else(|e| panic!("{code}: {e}"));
            assert_eq!(eic.as_str(), code);
            assert_eq!(
                Eic::compute_check_character(&code[..15]),
                Some(code.as_bytes()[15] as char),
            );
        }
    }

    #[test]
    fn the_object_type_is_the_third_character() {
        let area: Eic = "10YDE-VE-------2".parse().unwrap();
        assert_eq!(area.object_type(), Some(EicType::Area));
        assert_eq!(area.issuing_office(), "10");
        assert_eq!(area.check_character(), '2');

        let party: Eic = "10X---ENTSOE---L".parse().unwrap();
        assert_eq!(party.object_type(), Some(EicType::Party));
    }

    /// A single wrong character, and a transposed pair, both fail — which is
    /// the whole reason this is a type and not a `String`.
    #[test]
    fn a_corrupted_code_is_refused() {
        assert!(
            "10YDE-VE-------3".parse::<Eic>().is_err(),
            "wrong check char"
        );
        assert!("10YED-VE-------2".parse::<Eic>().is_err(), "transposition");
        assert!("10YDE-VE-------".parse::<Eic>().is_err(), "too short");
        assert!("10YDE-VE-------22".parse::<Eic>().is_err(), "too long");
        assert!("10YDE_VE-------2".parse::<Eic>().is_err(), "underscore");
    }

    /// Position 3 names the object, so a digit or a minus there is malformed
    /// however the check character comes out.
    #[test]
    fn the_type_position_must_be_a_letter() {
        // Build a body whose third character is a digit, then give it its own
        // correct check character: the shape rule must still reject it.
        let body = "1010DE---------";
        let check = Eic::compute_check_character(body).unwrap();
        assert!(format!("{body}{check}").parse::<Eic>().is_err());
    }

    /// Lowercase is accepted on the way in and never written back out — two
    /// casings of one Bilanzkreis must not become two keys.
    #[test]
    fn lowercase_is_canonicalised() {
        let eic: Eic = "  10yde-ve-------2  ".parse().unwrap();
        assert_eq!(eic.to_string(), "10YDE-VE-------2");
    }

    /// The minus sign is a permitted body character but §5.2 forbids it as a
    /// **check** character, so a body whose algorithm yields 36 is one the CIO
    /// never issues a code for. The function says `None` rather than writing a
    /// `-` that no reader would accept.
    #[test]
    fn a_body_computing_to_the_minus_sign_has_no_check_character() {
        let body = "10X000000000002"; // weighted sum ≡ 1 (mod 37)
        assert_eq!(Eic::compute_check_character(body), None);
        assert!(format!("{body}-").parse::<Eic>().is_err());
    }

    #[test]
    fn a_short_or_impure_body_has_no_check_character() {
        assert_eq!(Eic::compute_check_character("10X"), None);
        assert_eq!(Eic::compute_check_character("10x---entsoe---"), None);
    }

    #[test]
    fn an_unknown_type_letter_is_not_a_parse_failure() {
        // `Q` is not in the manual's list; the code is still well-formed.
        let body = "10Q---FUTURE---";
        let check = Eic::compute_check_character(body).unwrap();
        let eic: Eic = format!("{body}{check}").parse().unwrap();
        assert_eq!(eic.object_type(), None);
    }

    // ── Regelzone ─────────────────────────────────────────────────────────

    /// The four published ENTSO-E control-area codes, parsed back through the
    /// same check-character algorithm they were written with.
    #[test]
    fn every_control_area_code_is_a_valid_eic() {
        for zone in Regelzone::ALL {
            let eic = zone.control_area_eic();
            let text = eic.to_string();
            assert_eq!(
                text.parse::<Eic>(),
                Ok(eic),
                "{}: {text} does not round-trip",
                zone.as_str(),
            );
            assert_eq!(eic.object_type(), Some(EicType::Area));
            assert_eq!(eic.issuing_office(), "10", "the CIO, not the BDEW");
            assert!(!eic.is_german(), "a control area is a CIO code");
        }
        assert_eq!(
            Regelzone::FiftyHertz.control_area_eic().to_string(),
            "10YDE-VE-------2",
        );
    }

    /// Position 4 of a German `Y` code names the Regelzone — and only there.
    #[test]
    fn the_regelzone_is_read_off_a_german_y_code() {
        let bg: Eic = "11YR-AMPRION-BG9".parse().unwrap();
        assert_eq!(bg.regelzone(), Some(Regelzone::Amprion));
        assert_eq!(bg.object_type(), Some(EicType::Area));
        assert!(bg.is_german());

        for (code, zone) in [
            ("11YN-TENNET--BGQ", Regelzone::TenneT),
            ("11YV-50HERTZ-BGX", Regelzone::FiftyHertz),
            ("11YW-TNGBW---BGW", Regelzone::TransnetBw),
        ] {
            let eic: Eic = code.parse().unwrap_or_else(|e| panic!("{code}: {e}"));
            assert_eq!(eic.regelzone(), Some(zone));
            assert_eq!(zone.eic_letter(), code.as_bytes()[3] as char);
        }
    }

    /// A `X` code is a party, not an area, so position 4 means nothing there —
    /// and a code from another issuing office is not the BDEW's scheme at all.
    #[test]
    fn only_a_german_area_code_has_a_regelzone() {
        let party: Eic = "11XSAP-AMPRION-B".parse().unwrap();
        assert_eq!(party.object_type(), Some(EicType::Party));
        assert_eq!(party.regelzone(), None, "an X code carries no Regelzone");

        // A CIO-issued control area: right type, wrong issuing office.
        assert_eq!(Regelzone::Amprion.control_area_eic().regelzone(), None);
    }

    /// Every identifier converts from an owned `String` as well as a `&str`.
    ///
    /// A generic `impl TryInto<T>` bound sees the caller's own type, not a
    /// deref of it, so a missing `TryFrom<String>` is a `.as_str()` at one call
    /// site and a `.parse()?` at the next for the same value.
    #[test]
    fn an_owned_string_converts_like_a_borrowed_one() {
        fn accepts<T: TryFrom<S>, S>(value: S) -> Result<T, T::Error> {
            value.try_into()
        }

        assert_eq!(
            accepts::<MaloId, _>("51238696781".to_owned()).expect("a valid MaLo-ID"),
            "51238696781".parse::<MaloId>().expect("the same ID"),
        );
        assert_eq!(
            accepts::<BdewCode, _>("9900987654321".to_owned()).expect("a valid code"),
            "9900987654321".parse::<BdewCode>().expect("the same code"),
        );
        assert_eq!(
            accepts::<MeloId, _>("DE0001234567800000000000000012345".to_owned())
                .expect("a valid MeLo-ID"),
            "DE0001234567800000000000000012345"
                .parse::<MeloId>()
                .expect("the same ID"),
        );
        assert_eq!(
            accepts::<Eic, _>("10X168Y4E6H0041Z".to_owned()).expect("a valid EIC"),
            "10X168Y4E6H0041Z".parse::<Eic>().expect("the same EIC"),
        );

        // The error type is the crate's own, on both spellings.
        let err = accepts::<MaloId, _>("nope".to_owned()).expect_err("not an ID");
        assert!(err.to_string().contains("MaloId"), "{err}");
    }
}
