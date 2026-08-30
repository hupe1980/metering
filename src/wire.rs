//! How instants and dates travel, when the `serde` feature is on.
//!
//! Both directions ask the serializer whether it is human-readable:
//!
//! | Format | Instant | Date |
//! |---|---|---|
//! | JSON, YAML, TOML | `"2026-06-01T12:00:00Z"` — **RFC 3339** | `"2026-06-01"` — **ISO 8601** |
//! | bincode, postcard, MessagePack | `time`'s compact tuple | as above |
//!
//! The readable format gets the string a `TIMESTAMPTZ` cast, a JSON Schema
//! `format: date-time` and a log viewer all understand. The binary formats keep
//! `time`'s nine-integer packing, which matters because
//! [`MeterInterval`](crate::MeterInterval) carries two instants and is the
//! hottest type in the crate.
//!
//! `time`'s own `serde-human-readable` feature splits the same way but writes
//! `2026-06-01 12:00:00.0 +00:00:00`, which is readable and is not RFC 3339.
//! The twenty lines below buy the standard spelling.

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
