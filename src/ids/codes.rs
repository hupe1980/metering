//! The one-code-per-value contract for fieldless enums.
//!
//! [`string_codes!`] takes a type with `pub const ALL: [Self; N]` (every
//! variant, in declaration order) and `pub const fn as_str(self) -> &'static
//! str`, and derives `CODES`, [`Display`](std::fmt::Display),
//! [`FromStr`](std::str::FromStr) and, with the `serde` feature, `Serialize`
//! (writes `as_str`) and `Deserialize` (reads through `FromStr`). `CODES` is
//! computed from `ALL`, so the two cannot disagree.

/// Derive `CODES`, [`Display`](std::fmt::Display) and
/// [`FromStr`](std::str::FromStr) for a coded enum.
///
/// Each entry is a type, optionally with input aliases that are accepted but
/// never written:
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
            /// Every code the type writes, in the order of `ALL`; input
            /// aliases are absent.
            pub const CODES: &'static [&'static str] = &Self::CODE_ARRAY;

            /// Backing storage for [`CODES`](Self::CODES), sized by `ALL`.
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
            /// Writes [`as_str`](Self::as_str).
            fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                f.pad(self.as_str())
            }
        }

        #[cfg(feature = "serde")]
        impl ::serde::Serialize for $ty {
            fn serialize<S: ::serde::Serializer>(
                &self,
                serializer: S,
            ) -> ::std::result::Result<S::Ok, S::Error> {
                serializer.serialize_str(self.as_str())
            }
        }

        #[cfg(feature = "serde")]
        impl<'de> ::serde::Deserialize<'de> for $ty {
            fn deserialize<D: ::serde::Deserializer<'de>>(
                deserializer: D,
            ) -> ::std::result::Result<Self, D::Error> {
                let text = <::std::borrow::Cow<'de, str> as ::serde::Deserialize>::deserialize(
                    deserializer,
                )?;
                text.parse().map_err(::serde::de::Error::custom)
            }
        }

        impl ::std::str::FromStr for $ty {
            type Err = $crate::error::ParseError;

            /// Parses the [`CODES`](Self::CODES) and aliases, case-insensitively
            /// and ignoring surrounding whitespace; any other string is an error.
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
