//! Typed market identifiers — [`MaloId`], [`MeloId`], [`BdewCode`] and [`Eic`].
//!
//! Parse at the boundary (`"41373559241".parse::<MaloId>()?`): every type
//! validates its Bildungsvorschrift, including the check digit where one
//! exists, so a constructed value is always well-formed and writes one
//! canonical string.
//!
//! ## MaLo-ID (Marktlokations-Identifikationsnummer)
//!
//! BNetzA BK6-16-200 / BK7-16-142. The BDEW Anwendungshilfe *"Die neue
//! Marktlokations-Identifikationsnummer"* (v1.0, 28.04.2017) defines the
//! Bildungsvorschrift:
//!
//! | Position | Content |
//! |---|---|
//! | 1 | Vergabestelle — `1`–`3` DVGW, `4`–`9` BDEW |
//! | 2–10 | digits `0`–`9` |
//! | 11 | check digit |
//!
//! One MaLo-ID identifies one Marktlokation or one Tranche, permanently; it
//! says nothing about the Sparte.
//!
//! The check digit follows the *"Lok- und Waggon-Kennzeichnungsverfahren"*
//! (Anwendungshilfe §3.2): the odd-position digit sum plus **twice the
//! even-position sum** (not Luhn), subtracted from the next multiple of 10
//! (`0` when that difference is 10):
//!
//! ```text
//! 4 1 3 7 3 5 5 9 2 4 →  a) 4+3+3+5+2 = 17
//!                         b) (1+7+5+9+4) × 2 = 52
//!                         c) 17 + 52 = 69
//!                         d) 70 − 69 = 1  →  41373559241
//! ```
//!
//! The scheme misses a ±5 change in an even position and adjacent
//! transpositions of digits that differ by 5.
//!
//! ## MeLo-ID and Zählpunktbezeichnung
//!
//! [`MeloId`] is the 33-character Zählpunktbezeichnung of VDE-AR-N 4400
//! (Strom) / DVGW G 2000 (Gas) — one identifier, one type. It has no check
//! digit, so only its structure is validated.
//!
//! ## EIC (Energy Identification Code)
//!
//! [`Eic`] is the sixteen-character ENTSO-E identifier of Bilanzkreise,
//! Bilanzierungsgebiete, Regelzonen and Metering Grid Areas; its check
//! character is enforced at the parse.

use std::fmt;
use std::str::FromStr;

use crate::error::{ParseError, ParseErrorKind};

// ── issuer ────────────────────────────────────────────────────────────────────

/// Which Codevergabestelle issued a [`MaloId`] (see [`MaloId::issuer`]) or a
/// [`BdewCode`] (see [`BdewCode::issuer`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Issuer {
    /// Energie Codes und Services GmbH (BDEW), Sparte Strom.
    Bdew,
    /// DVGW Service & Consult GmbH, Sparte Gas.
    Dvgw,
    /// GS1 Germany — a Global Location Number used as a Marktpartner-ID.
    Gs1,
}

impl Issuer {
    /// Every issuer, in declaration order.
    pub const ALL: [Self; 3] = [Self::Bdew, Self::Dvgw, Self::Gs1];

    /// Stable DB/wire label. Matches the `serde` tag and `FromStr` input.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Bdew => "BDEW",
            Self::Dvgw => "DVGW",
            Self::Gs1 => "GS1",
        }
    }
}

crate::ids::codes::string_codes! {
    Issuer;
}

// ── MaloId ────────────────────────────────────────────────────────────────────

/// A validated 11-digit Marktlokations-Identifikationsnummer.
///
/// [`FromStr`]/[`TryFrom`] enforce the Bildungsvorschrift including the
/// check digit (see the [module docs](crate::ids)); [`Display`](fmt::Display)
/// writes the eleven digits.
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

    /// The check digit for ten leading digits (Anwendungshilfe §3.2), to
    /// complete an ID; `None` unless `digits` is exactly ten ASCII digits.
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

    /// Which Codevergabestelle issued this ID: first digit `1`–`3` DVGW,
    /// `4`–`9` BDEW.
    #[must_use]
    pub const fn issuer(&self) -> Issuer {
        match self.digits[0] {
            b'1'..=b'3' => Issuer::Dvgw,
            _ => Issuer::Bdew,
        }
    }

    /// The eleven digits as a `&str`.
    #[must_use]
    pub fn as_str(&self) -> &str {
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

    /// Parses eleven digits, ignoring surrounding whitespace; rejects a wrong
    /// length, a non-digit, a leading `0` and a check-digit mismatch.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let err = |kind| ParseError::format(kind, "MaloId", s, MALO_FORMAT);
        let t = s.trim();
        let bytes = t.as_bytes();
        if bytes.len() != Self::LEN {
            return Err(err(ParseErrorKind::Length));
        }
        if !bytes.iter().all(u8::is_ascii_digit) || bytes[0] == b'0' {
            return Err(err(ParseErrorKind::Charset));
        }
        let expected =
            Self::compute_check_digit(&t[..10]).ok_or_else(|| err(ParseErrorKind::Charset))?;
        if bytes[10] - b'0' != expected {
            return Err(err(ParseErrorKind::CheckDigit));
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

/// Accepts an owned `String` for generic `TryInto<MaloId>` bounds.
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
/// Shared by the MaLo-ID (§3.3), the BDEW-/DVGW-Codenummer (§2.3) and the
/// NeLo-ID (§4.3). `None` unless every byte is an ASCII digit.
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

/// A 13-digit **Marktpartner-Identifikationsnummer** — the BDEW- or
/// DVGW-Codenummer every market participant is addressed by.
///
/// MSCONS carries it in `NAD+MS`/`NAD+MR`.
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
/// # Check digit
///
/// §2.3 uses the *Lok- und Waggon-Kennzeichnungsverfahren* of the
/// [`MaloId`], except: *"Bei einer von GS1 vergebenen GLN
/// (= Globale Lokationsnummer) gilt das von GS1 verwendete
/// Prüfzifferverfahren."* A code is accepted when its thirteenth digit
/// satisfies either procedure and rejected with
/// [`ParseErrorKind::CheckDigit`] otherwise; [`issuer`](Self::issuer)
/// reports which one held.
///
/// ```rust
/// use metering::ids::{BdewCode, Issuer};
///
/// // Twelve digits plus the computed BDEW thirteenth.
/// let check = BdewCode::compute_check_digit("990098765432").unwrap();
/// let nb: BdewCode = format!("990098765432{check}").parse()?;
/// assert_eq!(nb.issuer(), Issuer::Bdew);
///
/// // A GS1 GLN (check digit by GS1's own procedure).
/// let gln: BdewCode = "4012345000009".parse()?;
/// assert_eq!(gln.issuer(), Issuer::Gs1);
///
/// // A mistyped digit fails both procedures.
/// assert!("9900987654321".parse::<BdewCode>().is_err());
/// # Ok::<(), metering::ParseError>(())
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BdewCode {
    /// The thirteen ASCII digits.
    digits: [u8; 13],
}

/// The shape [`BdewCode`] accepts, as rendered in a [`ParseError`].
const BDEW_CODE_FORMAT: &str =
    "a 13-digit BDEW/DVGW Marktpartner-ID, e.g. 9900987654329 (99 = BDEW/Strom, 98 = DVGW/Gas)";

impl BdewCode {
    /// The identifier's length: always thirteen digits.
    pub const LEN: usize = 13;

    /// The BDEW check digit for the twelve leading digits (*Identifikatoren in
    /// der Marktkommunikation* §6.1); `None` unless `digits` is exactly twelve
    /// ASCII digits. Not the GS1 procedure — see the [type docs](Self).
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
    #[must_use]
    pub fn has_bdew_check_digit(&self) -> bool {
        Self::compute_check_digit(&self.as_str()[..12]) == Some(self.check_digit())
    }

    /// Which Codevergabestelle issued this code.
    ///
    /// [`Bdew`](Issuer::Bdew) for a `99` prefix and [`Dvgw`](Issuer::Dvgw) for
    /// `98` when the BDEW check digit holds; [`Gs1`](Issuer::Gs1) otherwise —
    /// the parse guarantees the GS1 check digit then holds.
    #[must_use]
    pub fn issuer(&self) -> Issuer {
        match (self.digits[0], self.digits[1], self.has_bdew_check_digit()) {
            (b'9', b'9', true) => Issuer::Bdew,
            (b'9', b'8', true) => Issuer::Dvgw,
            _ => Issuer::Gs1,
        }
    }

    /// The thirteen digits as a `&str`.
    #[must_use]
    pub fn as_str(&self) -> &str {
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

    /// Parses thirteen digits, ignoring surrounding whitespace; the check
    /// digit must satisfy the BDEW or the GS1 procedure.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let err = |kind| ParseError::format(kind, "BdewCode", s, BDEW_CODE_FORMAT);
        let t = s.trim();
        let bytes = t.as_bytes();
        if bytes.len() != Self::LEN {
            return Err(err(ParseErrorKind::Length));
        }
        if !bytes.iter().all(u8::is_ascii_digit) {
            return Err(err(ParseErrorKind::Charset));
        }
        let check = bytes[12] - b'0';
        let bdew = Self::compute_check_digit(&t[..12]) == Some(check);
        if !bdew && gs1_check_digit(&bytes[..12]) != check {
            return Err(err(ParseErrorKind::CheckDigit));
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

/// Accepts an owned `String` for generic `TryInto<BdewCode>` bounds.
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

/// The GS1 modulo-10 check digit of a GLN body: weights 1 and 3 alternating
/// from the left over twelve digits (GS1 General Specifications § 7.9).
fn gs1_check_digit(body: &[u8]) -> u8 {
    let total: u32 = body
        .iter()
        .enumerate()
        .map(|(i, b)| u32::from(b - b'0') * if i % 2 == 0 { 1 } else { 3 })
        .sum();
    ((10 - (total % 10)) % 10) as u8
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
/// is canonicalised to uppercase.
///
/// ```rust
/// use metering::MeloId;
///
/// let melo: MeloId = "DE00056266802AO6G56M11SN51G21M24S".parse()?;
/// assert_eq!(melo.country(), "DE");
/// assert_eq!(melo.netzbetreiber_nr(), "000562");
/// assert!("not a zaehlpunkt".parse::<MeloId>().is_err());
/// # Ok::<(), metering::ParseError>(())
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MeloId {
    /// The thirty-three ASCII characters, uppercase.
    chars: [u8; 33],
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
        std::str::from_utf8(&self.chars).unwrap_or_default()
    }

    /// The two-letter country code (positions 1–2), `"DE"` in the German
    /// market.
    #[must_use]
    pub fn country(&self) -> &str {
        self.as_str().get(..2).unwrap_or_default()
    }

    /// The six-digit Netzbetreiber number (positions 3–8).
    #[must_use]
    pub fn netzbetreiber_nr(&self) -> &str {
        self.as_str().get(2..8).unwrap_or_default()
    }
}

impl fmt::Display for MeloId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.pad(self.as_str())
    }
}

impl FromStr for MeloId {
    type Err = ParseError;

    /// Parses a 33-character Zählpunktbezeichnung, ignoring surrounding
    /// whitespace and canonicalising to uppercase.
    ///
    /// Enforced: the length, letters in positions 1–2, digits in positions 3–8,
    /// and ASCII alphanumerics throughout. The Postleitzahl group may carry
    /// letters, as issued Zählpunktbezeichnungen do.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let err = |kind| ParseError::format(kind, "MeloId", s, MELO_FORMAT);
        let t = s.trim();
        if t.len() != Self::LEN {
            return Err(err(ParseErrorKind::Length));
        }
        if !t.bytes().all(|b| b.is_ascii_alphanumeric()) {
            return Err(err(ParseErrorKind::Charset));
        }
        let mut chars = [0u8; Self::LEN];
        chars.copy_from_slice(t.as_bytes());
        chars.make_ascii_uppercase();
        if !chars[..2].iter().all(u8::is_ascii_uppercase)
            || !chars[2..8].iter().all(u8::is_ascii_digit)
        {
            return Err(err(ParseErrorKind::Charset));
        }
        Ok(Self { chars })
    }
}

impl TryFrom<&str> for MeloId {
    type Error = ParseError;
    fn try_from(s: &str) -> Result<Self, Self::Error> {
        s.parse()
    }
}

/// Accepts an owned `String` for generic `TryInto<MeloId>` bounds.
impl TryFrom<String> for MeloId {
    type Error = ParseError;
    fn try_from(s: String) -> Result<Self, Self::Error> {
        s.parse()
    }
}

impl From<MeloId> for String {
    fn from(id: MeloId) -> String {
        id.as_str().to_owned()
    }
}

// ── Eic ───────────────────────────────────────────────────────────────────────

/// What an [`Eic`] identifies, read off its third character.
///
/// ENTSO-E *EIC Reference Manual* §4.2. The code is the letter itself;
/// [`name`](Self::name) carries the description.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum EicType {
    /// `X` — a **party**: a market participant, TSO, supplier, trader,
    /// Bilanzkreisverantwortlicher; German Bilanzkreise also carry `X` codes.
    Party,
    /// `Y` — an **area or domain**: a bidding zone, a control area, a
    /// Bilanzierungsgebiet, a Metering Grid Area.
    Area,
    /// `Z` — a **measurement point**.
    MeasurementPoint,
    /// `W` — a **resource object**: a generation, consumption or storage unit.
    /// Passive grid elements are type `T`.
    ResourceObject,
    /// `T` — a **tie line** or other connecting object: interconnectors,
    /// lines, busbar couplers, transformers.
    TieLine,
    /// `V` — a **location**, physical or logical, or an IT system.
    Location,
    /// `A` — a **substation** or topological node.
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

    /// The manual's description of the type (not a code).
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

crate::ids::codes::string_codes! {
    EicType;
}

/// One of the four German **Regelzonen**, read off position 4 of a
/// Bilanzierungsgebiet's EIC.
///
/// BDEW *Anwendungshilfe Energy Identification Codes* v1.0 (18.12.2017)
/// §2.2.2: `N` TenneT, `R` Amprion, `V` 50Hertz, `W` TransnetBW.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum Regelzone {
    /// `N` — TenneT TSO GmbH.
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

    /// The operator's own spelling (a description, not a code).
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::TenneT => "TenneT TSO",
            Self::Amprion => "Amprion",
            Self::FiftyHertz => "50Hertz Transmission",
            Self::TransnetBw => "TransnetBW",
        }
    }

    /// This Regelzone's letter at position 4 of a Bilanzierungsgebiet EIC.
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
    /// A `Y` code of the Central Issuing Office (LIO `10`), distinct from the
    /// Bilanzierungsgebiet codes under LIO `11`. The bodies carry the former
    /// company names (`EON`, `RWENET`, `VE`, `ENBW`).
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

crate::ids::codes::string_codes! {
    Regelzone;
}

/// The shape [`Eic`] accepts, as rendered in a [`ParseError`].
const EIC_FORMAT: &str = "a 16-character EIC: 2 characters of issuing office, an object-type \
     letter, 12 characters of `0-9`, `A-Z` or `-`, and a check character \
     (ENTSO-E EIC Reference Manual)";

/// A check-character-validated **Energy Identification Code**.
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
/// numbers (0 to 9) or the capital letters (A to Z)"* — never the minus. For
/// the German LIO see [`GERMAN_LIO`](Self::GERMAN_LIO).
///
/// # Check character
///
/// Enforced at the parse. Each of the first fifteen characters takes a value
/// (`0`–`9` → 0–9, `A`–`Z` → 10–35, `-` → 36) and is weighted 16 down to 2;
/// the check character has the value `36 − ((Σ − 1) mod 37)`. A result of 36
/// (the minus) is never issued.
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
        std::str::from_utf8(&self.chars).unwrap_or("")
    }

    /// The Local Issuing Office (positions 1–2). The CIO's list is open, so
    /// this is a `&str` and is not required to be digits.
    #[must_use]
    pub fn issuing_office(&self) -> &str {
        self.as_str().get(..2).unwrap_or("")
    }

    /// What this code identifies, or `None` for a type letter the manual's
    /// list does not contain.
    ///
    /// The list is ENTSO-E's to extend, so an unlisted letter is not a parse
    /// failure; the parse only requires an uppercase letter at position 3.
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
    /// BDEW (*Anwendungshilfe Energy Identification Codes* v1.0 §2.2.1): *"die
    /// Zahl „11" steht für das deutsche LIO im Strommarkt, den BDEW
    /// Bundesverband der Energie- und Wasserwirtschaft e.V."*
    pub const GERMAN_LIO: &'static str = "11";

    /// `true` when this code was issued by the German LIO.
    #[must_use]
    pub fn is_german(&self) -> bool {
        self.issuing_office() == Self::GERMAN_LIO
    }

    /// The **Regelzone** this code names: a Regelzone's own
    /// [`control_area_eic`](Regelzone::control_area_eic), or a German (`11`)
    /// `Y` code with `N`, `R`, `V` or `W` at position 4 — letters
    /// *Anwendungshilfe* v1.0 §2.2.2 reserves for Bilanzierungsgebiete.
    /// `None` otherwise.
    ///
    /// ```rust
    /// use metering::ids::{Eic, Regelzone};
    ///
    /// // A Bilanzierungsgebiet in the Amprion Regelzone (illustrative body).
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
    /// // ...and maps back to its Regelzone.
    /// assert_eq!(
    ///     Regelzone::Amprion.control_area_eic().regelzone(),
    ///     Some(Regelzone::Amprion),
    /// );
    /// # Ok::<(), metering::ParseError>(())
    /// ```
    #[must_use]
    pub fn regelzone(&self) -> Option<Regelzone> {
        if let Some(zone) = Regelzone::ALL
            .into_iter()
            .find(|z| z.control_area_eic() == *self)
        {
            return Some(zone);
        }
        if self.chars[0] != b'1' || self.chars[1] != b'1' || self.chars[2] != b'Y' {
            return None;
        }
        Regelzone::from_eic_letter(self.chars[3])
    }

    /// The check character for the first fifteen characters of a code.
    ///
    /// `None` when `body` is not exactly fifteen permitted characters, or when
    /// the algorithm yields 36 (the minus sign, forbidden as a check character).
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

/// The inverse of [`eic_value`] for check characters; `None` for 36 (the
/// minus, forbidden by §5.2).
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
    /// in position 3, and the check character.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let err = |kind| ParseError::format(kind, "Eic", s, EIC_FORMAT);
        let trimmed = s.trim();
        if trimmed.len() != Self::LEN {
            return Err(err(ParseErrorKind::Length));
        }
        let upper = trimmed.to_ascii_uppercase();
        let bytes = upper.as_bytes();
        if !bytes.iter().all(|b| eic_value(*b).is_some()) || !bytes[2].is_ascii_uppercase() {
            return Err(err(ParseErrorKind::Charset));
        }
        if Self::compute_check_character(&upper[..Self::LEN - 1]) != Some(bytes[15] as char) {
            return Err(err(ParseErrorKind::CheckDigit));
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

/// Accepts an owned `String` for generic `TryInto<Eic>` bounds.
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

// ── serde ─────────────────────────────────────────────────────────────────────

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
            f.write_str("a 13-digit Marktpartner-ID such as \"9900987654329\"")
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

    /// The worked example from the BDEW Anwendungshilfe §3.2.
    #[test]
    fn the_anwendungshilfe_example_is_reproduced() {
        assert_eq!(MaloId::compute_check_digit("4137355924"), Some(1));
        let malo: MaloId = "41373559241".parse().unwrap();
        assert_eq!(malo.check_digit(), 1);
        assert_eq!(malo.issuer(), Issuer::Bdew);
    }

    /// 5123869678 takes check digit 1 here; Luhn would give another.
    #[test]
    fn the_scheme_is_not_luhn() {
        assert_eq!(MaloId::compute_check_digit("5123869678"), Some(1));
        assert!("51238696781".parse::<MaloId>().is_ok());
        assert!("51238696780".parse::<MaloId>().is_err());
    }

    /// A total that is already a multiple of 10 takes check digit 0, not 10.
    #[test]
    fn a_round_total_gives_check_digit_zero() {
        assert_eq!(MaloId::compute_check_digit("6200000000"), Some(0));
        assert!("62000000000".parse::<MaloId>().is_ok());

        assert_eq!(MaloId::compute_check_digit("1000000000"), Some(9));
        assert_eq!(MaloId::compute_check_digit("8642097531"), Some(2));
    }

    #[test]
    fn issuer_follows_the_first_digit() {
        let dvgw: MaloId = {
            let check = MaloId::compute_check_digit("1234567890").unwrap();
            format!("1234567890{check}").parse().unwrap()
        };
        assert_eq!(dvgw.issuer(), Issuer::Dvgw);

        let bdew: MaloId = "41373559241".parse().unwrap();
        assert_eq!(bdew.issuer(), Issuer::Bdew);
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

    /// The EIC Reference Manual §5.1 examples and published German codes.
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

    /// A digit at position 3 is refused even with a correct check character.
    #[test]
    fn the_type_position_must_be_a_letter() {
        let body = "1010DE---------";
        let check = Eic::compute_check_character(body).unwrap();
        assert!(format!("{body}{check}").parse::<Eic>().is_err());
    }

    #[test]
    fn lowercase_is_canonicalised() {
        let eic: Eic = "  10yde-ve-------2  ".parse().unwrap();
        assert_eq!(eic.to_string(), "10YDE-VE-------2");
    }

    /// §5.2 forbids the minus as a check character.
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
        let body = "10Q---FUTURE---";
        let check = Eic::compute_check_character(body).unwrap();
        let eic: Eic = format!("{body}{check}").parse().unwrap();
        assert_eq!(eic.object_type(), None);
    }

    // ── Regelzone ─────────────────────────────────────────────────────────

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

    #[test]
    fn only_a_german_area_code_has_a_regelzone() {
        let party: Eic = "11XSAP-AMPRION-B".parse().unwrap();
        assert_eq!(party.object_type(), Some(EicType::Party));
        assert_eq!(party.regelzone(), None, "an X code carries no Regelzone");

        assert_eq!(
            Regelzone::Amprion.control_area_eic().regelzone(),
            Some(Regelzone::Amprion)
        );
    }

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
            accepts::<BdewCode, _>("9900987654329".to_owned()).expect("a valid code"),
            "9900987654329".parse::<BdewCode>().expect("the same code"),
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

        let err = accepts::<MaloId, _>("nope".to_owned()).expect_err("not an ID");
        assert!(err.to_string().contains("MaloId"), "{err}");
    }

    #[test]
    fn parse_errors_carry_their_kind() {
        let kind = |r: Result<MaloId, ParseError>| r.unwrap_err().kind();
        assert_eq!(kind("4137355924".parse()), ParseErrorKind::Length);
        assert_eq!(kind("4137355924x".parse()), ParseErrorKind::Charset);
        assert_eq!(kind("41373559242".parse()), ParseErrorKind::CheckDigit);
        assert_eq!(
            "10YED-VE-------2".parse::<Eic>().unwrap_err().kind(),
            ParseErrorKind::CheckDigit
        );
        assert_eq!(
            "DE0005626680€AO6G56M11SN51G21M24"
                .parse::<MeloId>()
                .unwrap_err()
                .kind(),
            ParseErrorKind::Length
        );
    }

    #[test]
    fn a_bdew_code_needs_one_valid_check_digit() {
        let bdew: BdewCode = "9900987654329".parse().unwrap();
        assert_eq!(bdew.issuer(), Issuer::Bdew);
        let gln: BdewCode = "4012345000009".parse().unwrap();
        assert_eq!(gln.issuer(), Issuer::Gs1);
        assert!(!gln.has_bdew_check_digit());
        assert_eq!(
            "9900987654321".parse::<BdewCode>().unwrap_err().kind(),
            ParseErrorKind::CheckDigit
        );
    }

    #[test]
    fn control_area_codes_map_back_to_their_regelzone() {
        for zone in Regelzone::ALL {
            assert_eq!(zone.control_area_eic().regelzone(), Some(zone));
        }
    }

    #[test]
    fn a_melo_id_is_copy() {
        let a: MeloId = "de00056266802ao6g56m11sn51g21m24s".parse().unwrap();
        let b = a;
        assert_eq!(a, b);
        assert_eq!(a.as_str(), "DE00056266802AO6G56M11SN51G21M24S");
    }
}
