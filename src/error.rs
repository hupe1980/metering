//! [`ParseError`] — the single error type every [`FromStr`] in this crate returns.
//!
//! One type rather than one per parsed type, so a caller decoding a row can
//! write
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
//! without three `map_err` calls or a bespoke enum. The rejected input and the
//! type that rejected it are both carried, so the message stays specific.
//!
//! [`FromStr`]: std::str::FromStr

use std::fmt;

/// What a parser would have accepted.
///
/// Private so the variants can grow without a breaking change; render it
/// through [`ParseError`]'s [`Display`](fmt::Display) or read the machine-usable
/// parts via [`ParseError::expected_values`].
#[derive(Debug, Clone, PartialEq, Eq)]
enum Expected {
    /// A closed set of codes, e.g. `["STROM", "GAS", …]`.
    OneOf(&'static [&'static str]),
    /// A free-form shape, e.g. `"A-B:C.D.E*F"`.
    Format(&'static str),
}

/// A string that could not be parsed into one of this crate's types.
///
/// Returned by every [`FromStr`](std::str::FromStr) implementation here.
/// Construct one only through the crate's parsers — the fields are private so
/// that added context is never a breaking change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    type_name: &'static str,
    input: String,
    expected: Expected,
}

impl ParseError {
    /// A value rejected against a closed set of codes.
    pub(crate) fn one_of(
        type_name: &'static str,
        input: &str,
        codes: &'static [&'static str],
    ) -> Self {
        Self {
            type_name,
            input: input.to_owned(),
            expected: Expected::OneOf(codes),
        }
    }

    /// A value rejected against a format description.
    pub(crate) fn format(type_name: &'static str, input: &str, shape: &'static str) -> Self {
        Self {
            type_name,
            input: input.to_owned(),
            expected: Expected::Format(shape),
        }
    }

    /// The type that rejected the input, e.g. `"Sparte"`.
    ///
    /// Useful when one `?` chain parses several types and the handler needs to
    /// know which column was bad.
    #[must_use]
    pub const fn type_name(&self) -> &'static str {
        self.type_name
    }

    /// The input that was rejected, verbatim.
    #[must_use]
    pub fn input(&self) -> &str {
        &self.input
    }

    /// The accepted codes, when the type has a closed set of them.
    ///
    /// `None` for types parsed by shape rather than by enumeration
    /// ([`ObisCode`](crate::ObisCode), [`IntervalResolution`](crate::IntervalResolution)) —
    /// their expectation is a format, which [`Display`](fmt::Display) renders.
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
        write!(f, "invalid {} {:?}: expected ", self.type_name, self.input)?;
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
    use crate::{IntervalResolution, ObisCode, QualityFlag, Sparte};

    #[test]
    fn message_names_the_type_and_the_input() {
        let err = "KOHLE".parse::<Sparte>().unwrap_err();
        assert_eq!(err.type_name(), "Sparte");
        assert_eq!(err.input(), "KOHLE");
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

        let iso = "nope".parse::<IntervalResolution>().unwrap_err();
        assert_eq!(iso.expected_values(), None);
        assert!(iso.to_string().contains("PT15M"), "{iso}");
    }

    /// One error type across every parser, so a decoder can use a single `?`
    /// chain — this is the reason the type is shared rather than per-type.
    #[test]
    fn every_parser_returns_the_same_error_type() {
        fn decode(s: &str, q: &str, o: &str, r: &str) -> Result<(), ParseError> {
            let _: Sparte = s.parse()?;
            let _: QualityFlag = q.parse()?;
            let _: ObisCode = o.parse()?;
            let _: IntervalResolution = r.parse()?;
            Ok(())
        }
        assert!(decode("GAS", "MEASURED", "7-0:3.0.0", "PT1H").is_ok());
        let err = decode("GAS", "MEASURED", "bad", "PT1H").unwrap_err();
        assert_eq!(err.type_name(), "ObisCode");
    }
}
