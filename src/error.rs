//! [`ParseError`] — the single error type every [`FromStr`] in this crate returns,
//! so one `?` chain decodes a row:
//!
//! ```rust
//! # use metering::{ObisCode, ParseError, QualityFlag, Sparte};
//! fn decode(sparte: &str, quality: &str, obis: &str)
//!     -> Result<(Sparte, QualityFlag, ObisCode), ParseError>
//! {
//!     Ok((sparte.parse()?, quality.parse()?, obis.parse()?))
//! }
//! # assert!(decode("STROM", "MEASURED", "1-0:1.8.0").is_ok());
//! # assert!(decode("KOHLE", "MEASURED", "1-0:1.8.0").is_err());
//! ```
//!
//! It carries the rejected input, the rejecting type and a [`ParseErrorKind`].
//!
//! [`FromStr`]: std::str::FromStr

use std::fmt;

/// Why a string was rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ParseErrorKind {
    /// The input has the wrong number of characters (or fields).
    Length,
    /// The input contains a character the format does not allow.
    Charset,
    /// The input is well-formed but its check digit or character is wrong.
    CheckDigit,
    /// The input is not one of a closed set of codes.
    Unknown,
    /// The input is well-formed but names a value outside the accepted range.
    Range,
}

/// What a parser would have accepted.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Expected {
    /// A closed set of codes, e.g. `["STROM", "GAS", …]`.
    OneOf(&'static [&'static str]),
    /// A free-form shape, e.g. `"A-B:C.D.E*F"`.
    Format(&'static str),
}

/// A string that could not be parsed into one of this crate's types.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    kind: ParseErrorKind,
    type_name: &'static str,
    input: String,
    expected: Expected,
}

impl ParseError {
    /// A value rejected against a closed set of codes ([`ParseErrorKind::Unknown`]).
    pub(crate) fn one_of(
        type_name: &'static str,
        input: &str,
        codes: &'static [&'static str],
    ) -> Self {
        Self {
            kind: ParseErrorKind::Unknown,
            type_name,
            input: input.to_owned(),
            expected: Expected::OneOf(codes),
        }
    }

    /// A value rejected against a format description.
    pub(crate) fn format(
        kind: ParseErrorKind,
        type_name: &'static str,
        input: &str,
        shape: &'static str,
    ) -> Self {
        Self {
            kind,
            type_name,
            input: input.to_owned(),
            expected: Expected::Format(shape),
        }
    }

    /// Why the input was rejected.
    #[must_use]
    pub const fn kind(&self) -> ParseErrorKind {
        self.kind
    }

    /// The type that rejected the input, e.g. `"Sparte"`.
    #[must_use]
    pub const fn type_name(&self) -> &'static str {
        self.type_name
    }

    /// The input that was rejected, verbatim.
    #[must_use]
    pub fn input(&self) -> &str {
        &self.input
    }

    /// The accepted codes, when the type has a closed set of them; `None` for
    /// types parsed by shape ([`ObisCode`](crate::ObisCode),
    /// [`Resolution`](crate::Resolution)), whose format
    /// [`Display`](fmt::Display) renders.
    #[must_use]
    pub const fn expected_values(&self) -> Option<&'static [&'static str]> {
        match self.expected {
            Expected::OneOf(codes) => Some(codes),
            Expected::Format(_) => None,
        }
    }
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let why = match self.kind {
            ParseErrorKind::Length => "wrong length",
            ParseErrorKind::Charset => "invalid character",
            ParseErrorKind::CheckDigit => "wrong check digit",
            ParseErrorKind::Unknown => "unknown code",
            ParseErrorKind::Range => "out of range",
        };
        write!(
            f,
            "invalid {} {:?} ({why}): expected ",
            self.type_name, self.input
        )?;
        match self.expected {
            Expected::OneOf(codes) => write!(f, "one of {}", codes.join(", ")),
            Expected::Format(shape) => f.write_str(shape),
        }
    }
}

impl std::error::Error for ParseError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ObisCode, QualityFlag, Resolution, Sparte};

    #[test]
    fn message_names_the_type_and_the_input() {
        let err = "KOHLE".parse::<Sparte>().unwrap_err();
        assert_eq!(err.type_name(), "Sparte");
        assert_eq!(err.input(), "KOHLE");
        assert_eq!(err.kind(), ParseErrorKind::Unknown);
        let msg = err.to_string();
        assert!(msg.contains("Sparte"), "{msg}");
        assert!(msg.contains("KOHLE"), "{msg}");
        assert!(msg.contains("STROM"), "{msg}");
    }

    #[test]
    fn closed_sets_expose_their_codes_and_formats_do_not() {
        let closed = "nope".parse::<QualityFlag>().unwrap_err();
        assert_eq!(closed.expected_values(), Some(QualityFlag::CODES));

        let shaped = "nope".parse::<ObisCode>().unwrap_err();
        assert_eq!(shaped.expected_values(), None);
        assert!(shaped.to_string().contains("A-B:C.D.E*F"), "{shaped}");

        let iso = "nope".parse::<Resolution>().unwrap_err();
        assert_eq!(iso.expected_values(), None);
        assert!(iso.to_string().contains("PT15M"), "{iso}");
    }

    #[test]
    fn every_parser_returns_the_same_error_type() {
        fn decode(s: &str, q: &str, o: &str, r: &str) -> Result<(), ParseError> {
            let _: Sparte = s.parse()?;
            let _: QualityFlag = q.parse()?;
            let _: ObisCode = o.parse()?;
            let _: Resolution = r.parse()?;
            Ok(())
        }
        assert!(decode("GAS", "MEASURED", "7-0:3.0.0", "PT1H").is_ok());
        let err = decode("GAS", "MEASURED", "bad", "PT1H").unwrap_err();
        assert_eq!(err.type_name(), "ObisCode");
    }
}
