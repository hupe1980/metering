//! How instants, dates and quantities travel, when the `serde` feature is on.
//!
//! | Format | Instant | Date | Quantity |
//! |---|---|---|---|
//! | JSON, YAML, TOML | `"2026-06-01T12:00:00Z"` — **RFC 3339** | `"2026-06-01"` — **ISO 8601** | `"12.345"` — the exact decimal string |
//! | bincode, postcard, MessagePack | `time`'s compact tuple | as above | as above |
//!
//! Instants and dates split on `is_human_readable`; `time`'s own
//! `serde-human-readable` is not RFC 3339. A quantity does not split — see
//! [`decimal`].
//!
//! Every timestamp and quantity field names one of these modules rather than
//! inheriting an impl whose format depends on feature unification;
//! `tests/it/scanners/serde_wire.rs` enforces it.

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
pub(crate) mod iso_date {
    use serde::{Deserialize, Deserializer, Serialize, Serializer, de, ser};
    use time::Date;
    use time::format_description::BorrowedFormatItem;

    /// `YYYY-MM-DD`; `Iso8601`'s default configuration would format time
    /// components a [`Date`] does not have.
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
/// Reading asks for a string, so a JSON number is a type error rather than a
/// trip through `f64`, and parses with
/// [`from_str_exact`](rust_decimal::Decimal::from_str_exact), so excess digits
/// are refused rather than rounded.
///
/// Not `rust_decimal`'s `serde` features: Cargo features unify across the
/// build graph, so `serde-str`/`serde-float` set anywhere would change these
/// quantities' format.
pub(crate) mod decimal {
    use core::fmt;
    use rust_decimal::Decimal;
    use serde::{Deserializer, Serializer, de};

    pub(crate) fn serialize<S: Serializer>(
        value: &Decimal,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        // `collect_str` lets the serialiser render `Display` without an
        // intermediate `String`.
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

    /// Carries this representation through `serde`'s container impls (the
    /// option, sequence, array and map modules below).
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

/// [`decimal`] for a sequence of quantities.
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

/// [`decimal`] for a fixed-length array of quantities.
pub(crate) mod decimal_array {
    use super::decimal::Dec;
    use core::fmt;
    use rust_decimal::Decimal;
    use serde::ser::SerializeTuple;
    use serde::{Deserializer, Serializer, de};

    // A tuple, as `[T; N]`'s own impls spell it, so a binary format omits the
    // length; both halves must agree or postcard misreads the count.
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

/// [`decimal`] for a map of quantities.
pub(crate) mod decimal_map {
    use super::decimal::Dec;
    use rust_decimal::Decimal;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use std::collections::BTreeMap;

    pub(crate) fn serialize<K: Serialize, S: Serializer>(
        values: &BTreeMap<K, Decimal>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        serializer.collect_map(values.iter().map(|(key, &q)| (key, Dec(q))))
    }

    pub(crate) fn deserialize<'de, K: Deserialize<'de> + Ord, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<BTreeMap<K, Decimal>, D::Error> {
        Ok(BTreeMap::<K, Dec>::deserialize(deserializer)?
            .into_iter()
            .map(|(key, d)| (key, d.0))
            .collect())
    }
}
