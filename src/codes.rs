//! The one-code-per-value contract, derived rather than written out.
//!
//! [`string_codes!`] takes a type with
//!
//! - `pub const ALL: [Self; N]` — every variant, in declaration order, and
//! - `pub const fn as_str(self) -> &'static str` — the code,
//!
//! and derives `CODES`, [`Display`](std::fmt::Display) and
//! [`FromStr`](std::str::FromStr). `CODES` is *computed* from `ALL` in a
//! `const` block, so the two cannot disagree: adding a variant extends both or
//! fails to compile. A database `CHECK` constraint generated from `CODES`
//! therefore cannot drift from what the crate writes.
//!
//! Enums carrying data — [`MeasurementSource`](crate::MeasurementSource),
//! [`AggregationRule`](crate::AggregationRule),
//! [`Capability`](crate::Capability) — are not codes; a variant plus its
//! payload has no single string. Each carries a coded discriminator instead.

/// Derive `CODES`, [`Display`](std::fmt::Display) and
/// [`FromStr`](std::str::FromStr) for a coded enum.
///
/// See the [module docs](self) for the contract. Each entry is a type, and
/// optionally a list of lenient input aliases that are accepted on the way in
/// and never written on the way out:
///
/// ```text
/// string_codes! {
///     Messtyp;
///     Sparte, aliases = [("WÄRME", Self::Waerme)];
/// }
/// ```
macro_rules! string_codes {
    ($(
        $ty:ty
        $(, aliases = [$( ($alias:literal, $to:expr) ),+ $(,)?])?
    );+ $(;)?) => {$(
        impl $ty {
            /// Every accepted code, in the same order as `ALL`.
            ///
            /// Computed from `ALL`, so it cannot fall out of step with it —
            /// which is what a database `CHECK` constraint generated from this
            /// list depends on. Lenient input aliases are deliberately absent:
            /// this is the set of strings the type *writes*.
            pub const CODES: &'static [&'static str] = &Self::CODE_ARRAY;

            /// Backing storage for [`CODES`](Self::CODES). The array length is
            /// `ALL`'s own, so a new variant that is not given a code fails to
            /// compile rather than falling off the end of the list.
            const CODE_ARRAY: [&'static str; <$ty>::ALL.len()] = {
                let all = Self::ALL;
                let mut out = [""; <$ty>::ALL.len()];
                let mut i = 0;
                while i < out.len() {
                    out[i] = all[i].as_str();
                    i += 1;
                }
                out
            };
        }

        impl ::std::fmt::Display for $ty {
            /// Writes [`as_str`](Self::as_str) — the same string the `serde`
            /// tag uses and [`FromStr`](std::str::FromStr) reads.
            fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                f.pad(self.as_str())
            }
        }

        impl ::std::str::FromStr for $ty {
            type Err = $crate::error::ParseError;

            /// Parses the [`CODES`](Self::CODES), case-insensitively and
            /// ignoring surrounding whitespace.
            ///
            /// An unrecognised code is an error, never a silent default: a
            /// value this crate cannot name is a statement about the message,
            /// not about the thing the message describes.
            fn from_str(s: &str) -> ::std::result::Result<Self, Self::Err> {
                let trimmed = s.trim();
                $($(
                    if trimmed.eq_ignore_ascii_case($alias)
                        || trimmed.to_uppercase() == $alias
                    {
                        return Ok($to);
                    }
                )+)?
                let upper = trimmed.to_uppercase();
                Self::ALL
                    .into_iter()
                    .find(|v| v.as_str() == upper)
                    .ok_or_else(|| {
                        $crate::error::ParseError::one_of(stringify!($ty), s, Self::CODES)
                    })
            }
        }
    )+};
}

pub(crate) use string_codes;
