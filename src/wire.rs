//! How instants, dates and quantities travel, when the `serde` feature is on.
//!
//! | Format | Instant | Date | Quantity |
//! |---|---|---|---|
//! | JSON, YAML, TOML | `"2026-06-01T12:00:00Z"` — **RFC 3339** | `"2026-06-01"` — **ISO 8601** | `"12.345"` — the exact decimal string |
//! | bincode, postcard, MessagePack | `time`'s compact tuple | as above | as above |
//!
//! Instants and dates split on `is_human_readable`: the readable form is what a
//! `TIMESTAMPTZ` cast and a JSON Schema `format: date-time` understand, and the
//! binary one keeps `time`'s nine-integer packing, which matters because
//! [`MeterInterval`](crate::MeterInterval) carries two instants and is the
//! hottest type here. `time`'s own `serde-human-readable` feature splits the
//! same way but writes `2026-06-01 12:00:00.0 +00:00:00`, which is not RFC 3339.
//!
//! A quantity does not split — see [`decimal`].
//!
//! **Every field states its representation.** Nothing here relies on an
//! inherited impl for a timestamp or a quantity, which is what makes the format
//! a property of this crate rather than of whichever features happened to
//! unify. Two scans in `tests/serde_representation.rs` fail if a field forgets.

#![cfg(feature = "serde")]

/// UTC instants as RFC 3339 in a human-readable format, compact otherwise.
pub(crate) mod rfc3339 {
    use serde::{Deserialize, Deserializer, Serialize, Serializer, de, ser};
    use time::OffsetDateTime;
    use time::format_description::well_known::Rfc3339;

    pub(crate) fn serialize<S: Serializer>(
        value: &OffsetDateTime,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        if serializer.is_human_readable() {
            let text = value.format(&Rfc3339).map_err(ser::Error::custom)?;
            return serializer.serialize_str(&text);
        }
        value.serialize(serializer)
    }

    pub(crate) fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<OffsetDateTime, D::Error> {
        if deserializer.is_human_readable() {
            let text = <std::borrow::Cow<'_, str>>::deserialize(deserializer)?;
            return OffsetDateTime::parse(&text, &Rfc3339).map_err(de::Error::custom);
        }
        OffsetDateTime::deserialize(deserializer)
    }
}

/// [`rfc3339`] for an optional instant.
///
/// A separate module because `serde(with)` is applied to the field's own type,
/// and `Option<OffsetDateTime>` is a different type from `OffsetDateTime`.
pub(crate) mod rfc3339_option {
    use serde::{Deserialize, Deserializer, Serializer};
    use time::OffsetDateTime;

    #[expect(
        clippy::ref_option,
        reason = "the signature `serde(with)` requires of a serialize function"
    )]
    pub(crate) fn serialize<S: Serializer>(
        value: &Option<OffsetDateTime>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        match value {
            Some(instant) => serializer.serialize_some(&Wrapper(*instant)),
            None => serializer.serialize_none(),
        }
    }

    pub(crate) fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<OffsetDateTime>, D::Error> {
        Ok(Option::<Wrapper>::deserialize(deserializer)?.map(|w| w.0))
    }

    /// Carries the one-field `serde(with)` through `Option`'s own impls.
    #[derive(serde::Serialize, serde::Deserialize)]
    #[serde(transparent)]
    struct Wrapper(#[serde(with = "super::rfc3339")] OffsetDateTime);
}

/// Calendar dates as ISO 8601 (`2026-06-01`) in a human-readable format,
/// compact otherwise.
///
/// The German market's validity bounds — a Zählzeitdefinition's year, a
/// measurement point's `valid_from` — are calendar dates in local time rather
/// than instants, and travel as dates.
pub(crate) mod iso_date {
    use serde::{Deserialize, Deserializer, Serialize, Serializer, de, ser};
    use time::Date;
    use time::format_description::BorrowedFormatItem;

    /// `YYYY-MM-DD`, and nothing else.
    ///
    /// Spelled out rather than reached for through `Iso8601`, whose default
    /// configuration formats time components — which a [`Date`] does not have,
    /// and which fails at compile time if you ask it to.
    const FORMAT: &[BorrowedFormatItem<'_>] =
        time::macros::format_description!("[year]-[month]-[day]");

    pub(crate) fn serialize<S: Serializer>(value: &Date, serializer: S) -> Result<S::Ok, S::Error> {
        if serializer.is_human_readable() {
            let text = value.format(FORMAT).map_err(ser::Error::custom)?;
            return serializer.serialize_str(&text);
        }
        value.serialize(serializer)
    }

    pub(crate) fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Date, D::Error> {
        if deserializer.is_human_readable() {
            let text = <std::borrow::Cow<'_, str>>::deserialize(deserializer)?;
            return Date::parse(&text, FORMAT).map_err(de::Error::custom);
        }
        Date::deserialize(deserializer)
    }
}

/// [`iso_date`] for an optional date.
pub(crate) mod iso_date_option {
    use serde::{Deserialize, Deserializer, Serializer};
    use time::Date;

    #[expect(
        clippy::ref_option,
        reason = "the signature `serde(with)` requires of a serialize function"
    )]
    pub(crate) fn serialize<S: Serializer>(
        value: &Option<Date>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        match value {
            Some(date) => serializer.serialize_some(&Wrapper(*date)),
            None => serializer.serialize_none(),
        }
    }

    pub(crate) fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<Date>, D::Error> {
        Ok(Option::<Wrapper>::deserialize(deserializer)?.map(|w| w.0))
    }

    #[derive(serde::Serialize, serde::Deserialize)]
    #[serde(transparent)]
    struct Wrapper(#[serde(with = "super::iso_date")] Date);
}

/// Quantities as their exact decimal string, in **every** format.
///
/// No `is_human_readable` split: a decimal string is also the compact form
/// (`"0.25"` is five postcard bytes against sixteen for a packed mantissa).
///
/// Reading asks for a **string** — so a JSON number is a type error rather than
/// a silent trip through `f64`, and `deserialize_any`, the one question
/// postcard and bincode cannot answer, is never asked — and parses with
/// [`from_str_exact`](rust_decimal::Decimal::from_str_exact), so excess digits
/// are refused rather than rounded away.
///
/// **Do not replace this with `rust_decimal`'s own `serde` modules.** Reaching
/// for them means enabling one of its features, and Cargo features are global
/// to a build graph: `serde-str` would change how every `Decimal` in the
/// consumer's workspace deserialises, and `serde-float` set by any crate in
/// that graph would decide how these quantities serialise.
pub(crate) mod decimal {
    use core::fmt;
    use rust_decimal::Decimal;
    use serde::{Deserializer, Serializer, de};

    pub(crate) fn serialize<S: Serializer>(
        value: &Decimal,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        // `collect_str` rather than `serialize_str(&value.to_string())`: it
        // hands the serialiser the `Display` impl and lets it decide how to
        // render it. `serde_json` writes the digits straight into its output
        // buffer; a format that needs the byte length up front finds it its own
        // way, rather than paying for an allocation this crate imposed.
        serializer.collect_str(value)
    }

    pub(crate) fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Decimal, D::Error> {
        deserializer.deserialize_str(DecimalVisitor)
    }

    struct DecimalVisitor;

    impl de::Visitor<'_> for DecimalVisitor {
        type Value = Decimal;

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("an exact decimal quantity as a string, such as \"12.345\"")
        }

        fn visit_str<E: de::Error>(self, value: &str) -> Result<Decimal, E> {
            Decimal::from_str_exact(value)
                .map_err(|_| E::invalid_value(de::Unexpected::Str(value), &self))
        }
    }

    /// Carries the field-level representation through a container's own impls.
    ///
    /// `serde(with)` names functions over the field's exact type, and
    /// `Vec<Decimal>` is not `Decimal`. A transparent newtype is what the
    /// sequence, array and map modules below hand to `serde`'s own container
    /// impls so that the element representation stays this one.
    #[derive(serde::Serialize, serde::Deserialize)]
    #[serde(transparent)]
    pub(super) struct Dec(#[serde(with = "self")] pub(super) Decimal);
}

/// [`decimal`] for an optional quantity.
pub(crate) mod decimal_option {
    use super::decimal::Dec;
    use rust_decimal::Decimal;
    use serde::{Deserialize, Deserializer, Serializer};

    #[expect(
        clippy::ref_option,
        reason = "the signature `serde(with)` requires of a serialize function"
    )]
    pub(crate) fn serialize<S: Serializer>(
        value: &Option<Decimal>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        match value {
            Some(quantity) => serializer.serialize_some(&Dec(*quantity)),
            None => serializer.serialize_none(),
        }
    }

    pub(crate) fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<Decimal>, D::Error> {
        Ok(Option::<Dec>::deserialize(deserializer)?.map(|d| d.0))
    }
}

/// [`decimal`] for a sequence of quantities — a day's profile values.
pub(crate) mod decimal_vec {
    use super::decimal::Dec;
    use rust_decimal::Decimal;
    use serde::{Deserialize, Deserializer, Serializer};

    pub(crate) fn serialize<S: Serializer>(
        values: &[Decimal],
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        serializer.collect_seq(values.iter().map(|&q| Dec(q)))
    }

    pub(crate) fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Vec<Decimal>, D::Error> {
        Ok(Vec::<Dec>::deserialize(deserializer)?
            .into_iter()
            .map(|d| d.0)
            .collect())
    }
}

/// [`decimal`] for a fixed-length array of quantities — the seven weekday
/// factors of a gas SLP.
pub(crate) mod decimal_array {
    use super::decimal::Dec;
    use core::fmt;
    use rust_decimal::Decimal;
    use serde::ser::SerializeTuple;
    use serde::{Deserializer, Serializer, de};

    // A fixed-length array is a **tuple** to `serde`, not a sequence: that is
    // how `[T; N]`'s own impls spell it, and it is what lets a binary format
    // leave out the length it already knows. Both halves here must agree, or
    // postcard writes a count that the reader does not expect.
    pub(crate) fn serialize<S: Serializer, const N: usize>(
        values: &[Decimal; N],
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        let mut tuple = serializer.serialize_tuple(N)?;
        for quantity in values {
            tuple.serialize_element(&Dec(*quantity))?;
        }
        tuple.end()
    }

    pub(crate) fn deserialize<'de, D: Deserializer<'de>, const N: usize>(
        deserializer: D,
    ) -> Result<[Decimal; N], D::Error> {
        deserializer.deserialize_tuple(N, ArrayVisitor::<N>)
    }

    struct ArrayVisitor<const N: usize>;

    impl<'de, const N: usize> de::Visitor<'de> for ArrayVisitor<N> {
        type Value = [Decimal; N];

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(formatter, "{N} exact decimal quantities, each a string")
        }

        fn visit_seq<A: de::SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
            let mut values = [Decimal::ZERO; N];
            for (index, slot) in values.iter_mut().enumerate() {
                *slot = seq
                    .next_element::<Dec>()?
                    .ok_or_else(|| de::Error::invalid_length(index, &self))?
                    .0;
            }
            Ok(values)
        }
    }
}

/// [`decimal`] for a map of quantities — an allocation key's per-participant
/// fractions.
pub(crate) mod decimal_map {
    use super::decimal::Dec;
    use rust_decimal::Decimal;
    use serde::{Deserialize, Deserializer, Serializer};
    use std::collections::BTreeMap;

    pub(crate) fn serialize<S: Serializer>(
        values: &BTreeMap<String, Decimal>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        serializer.collect_map(values.iter().map(|(key, &q)| (key, Dec(q))))
    }

    pub(crate) fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<BTreeMap<String, Decimal>, D::Error> {
        Ok(BTreeMap::<String, Dec>::deserialize(deserializer)?
            .into_iter()
            .map(|(key, d)| (key, d.0))
            .collect())
    }
}
