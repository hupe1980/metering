//! One value, one string — the property that makes a persisted key trustworthy.
//!
//! Every type here has a `Display` form that a consumer will end up storing:
//! in a database column, a merge key, a filename, a Kafka message key. The
//! moment one value has two spellings, two rows that mean the same thing stop
//! comparing equal, and nothing anywhere reports an error — a correction simply
//! fails to supersede the reading it corrects, and the total is overstated.
//!
//! `ObisCode` had exactly that bug: `FromStr` defaulted the storage group to 255
//! and `Display` always printed it, so `"1-0:1.8.0"` — the spelling MSCONS
//! carries and people type — came back out as `"1-0:1.8.0*255"`.
//!
//! The invariants below are what "canonical" has to mean:
//!
//! 1. **Stability** — `s.parse()?.to_string() == s` for every canonical `s`.
//! 2. **Totality** — `v.to_string().parse()? == v` for every value `v`.
//! 3. **Idempotence** — normalising twice equals normalising once, so every
//!    accepted spelling converges on one string.
//! 4. **Injectivity** — different values never render to the same string.
//!
//! Parsing is deliberately lenient and `Display` deliberately is not. That is
//! what makes 1–4 co-exist: a lenient parser accepting *n* spellings and a
//! canonical writer emitting one is a normalisation, and normalisations compose.

use metering::{IntervalResolution, ObisCode};
use proptest::prelude::*;

// ── ObisCode ─────────────────────────────────────────────────────────────────

prop_compose! {
    fn arb_obis()(
        a in 0u8..=255, b in 0u8..=255, c in 0u8..=255,
        d in 0u8..=255, e in 0u8..=255, f in 0u8..=255,
    ) -> ObisCode {
        ObisCode { a, b, c, d, e, f }
    }
}

proptest! {
    /// Totality: whatever a code renders as, it reads back as itself — through
    /// the canonical form and the explicit one alike.
    #[test]
    fn obis_display_round_trips(code in arb_obis()) {
        prop_assert_eq!(code.to_string().parse::<ObisCode>(), Ok(code));
        prop_assert_eq!(code.to_full_string().parse::<ObisCode>(), Ok(code));
    }

    /// Stability: the canonical form is a fixed point of parse-then-render.
    #[test]
    fn obis_canonical_form_is_stable(code in arb_obis()) {
        let canonical = code.to_string();
        prop_assert_eq!(ObisCode::normalize(&canonical), Ok(canonical.clone()));
    }

    /// Idempotence: both spellings converge, so a row written through either
    /// path lands on one key.
    #[test]
    fn obis_both_spellings_normalise_alike(code in arb_obis()) {
        let canonical = code.to_string();
        prop_assert_eq!(ObisCode::normalize(&code.to_full_string()), Ok(canonical));
    }

    /// Injectivity: two codes share a string only if they are the same code.
    /// Without this, eliding `*F` would merge distinct billing-period registers.
    #[test]
    fn obis_strings_are_injective(x in arb_obis(), y in arb_obis()) {
        prop_assert_eq!(x == y, x.to_string() == y.to_string());
    }

    /// `MAX_LEN` must actually bound the rendered length, in both spellings —
    /// it is what a consumer sizes a `VARCHAR` column from, and the `Display`
    /// impl writes through a stack buffer of exactly that size.
    #[test]
    fn max_len_bounds_every_rendering(code in arb_obis()) {
        prop_assert!(code.to_string().len() <= ObisCode::MAX_LEN);
        prop_assert!(code.to_full_string().len() <= ObisCode::MAX_LEN);
        prop_assert!(code.to_string().is_ascii());
    }

    /// Width, fill and alignment reach the output. A `Display` impl that writes
    /// straight to the formatter silently ignores them and misaligns tables.
    #[test]
    fn display_honours_width_and_alignment(code in arb_obis()) {
        let plain = code.to_string();
        let width = ObisCode::MAX_LEN + 2;
        let right = format!("{code:>width$}");
        prop_assert_eq!(right.len(), width);
        prop_assert!(right.ends_with(&plain));
        prop_assert_eq!(format!("{code:-<width$}").len(), width);
        // ...and no width still means no padding.
        prop_assert_eq!(format!("{code}"), plain);
    }

    /// Leading zeros and padding are accepted, and normalise away.
    #[test]
    fn obis_tolerates_padding_and_leading_zeros(code in arb_obis()) {
        let padded = format!(
            "  {:03}-{:03}:{:03}.{:03}.{:03}*{:03}  ",
            code.a, code.b, code.c, code.d, code.e, code.f
        );
        prop_assert_eq!(padded.parse::<ObisCode>(), Ok(code));
        prop_assert_eq!(ObisCode::normalize(&padded), Ok(code.to_string()));
    }
}

/// The reported failure, end to end: a reading written through one path and a
/// correction written through the other must share a merge key.
#[test]
fn obis_correction_supersedes_the_reading_it_corrects() {
    let reading_key = ObisCode::normalize("1-0:1.8.0").unwrap(); // raw SQL / MSCONS
    let correction_key = ObisCode::STROM_BEZUG_TOTAL.to_string(); // via the crate

    assert_eq!(
        reading_key, correction_key,
        "one channel must produce one merge key, whichever path wrote it"
    );
}

// ── IntervalResolution ───────────────────────────────────────────────────────

fn arb_resolution() -> impl Strategy<Value = IntervalResolution> {
    prop_oneof![
        Just(IntervalResolution::QuarterHour),
        Just(IntervalResolution::HalfHour),
        Just(IntervalResolution::Hour),
        Just(IntervalResolution::Day),
        Just(IntervalResolution::Month),
        Just(IntervalResolution::Year),
        (1u32..1_000_000).prop_map(|n| IntervalResolution::from_seconds(n).unwrap()),
    ]
}

proptest! {
    #[test]
    fn resolution_display_round_trips(r in arb_resolution()) {
        prop_assert_eq!(r.to_string().parse::<IntervalResolution>(), Ok(r));
        prop_assert_eq!(r.to_iso8601().parse::<IntervalResolution>(), Ok(r));
    }

    /// `Display` and `to_iso8601` are the same string, not two conventions.
    #[test]
    fn resolution_display_is_the_iso_form(r in arb_resolution()) {
        prop_assert_eq!(r.to_string(), r.to_iso8601());
    }

    #[test]
    fn resolution_strings_are_injective(x in arb_resolution(), y in arb_resolution()) {
        prop_assert_eq!(x == y, x.to_string() == y.to_string());
    }

    /// One duration, one value. Two resolutions with the same fixed length used
    /// to be constructible — `Custom(900)` alongside `QuarterHour` — which gave
    /// one 15-minute grid two database keys and broke the round trip, because
    /// `Custom(900)` writes `"PT900S"` and `"PT900S"` reads back as
    /// `QuarterHour`. The payload is opaque now, so this holds by construction.
    #[test]
    fn one_fixed_length_is_one_value(x in arb_resolution(), y in arb_resolution()) {
        if let (Some(a), Some(b)) = (x.fixed_seconds(), y.fixed_seconds()) {
            prop_assert_eq!(a == b, x == y);
        }
    }

    /// `from_seconds` is the normal form: it is the only constructor, it is
    /// idempotent through `fixed_seconds`, and what it builds round-trips.
    #[test]
    fn from_seconds_is_the_normal_form(secs in 1u32..1_000_000) {
        let r = IntervalResolution::from_seconds(secs).expect("nonzero");
        prop_assert_eq!(r.fixed_seconds(), Some(secs));
        prop_assert_eq!(IntervalResolution::from_seconds(secs), Some(r));
        prop_assert_eq!(r.to_string().parse::<IntervalResolution>(), Ok(r));
    }
}

// ── LoadProfile and the lifecycle codes ──────────────────────────────────────

/// Every closed-vocabulary enum with a `Display` form obeys the same four
/// invariants, so they are checked in one place rather than per module.
#[test]
fn closed_vocabularies_round_trip_and_stay_injective() {
    use metering::LoadProfile;
    use metering::lifecycle::{MeterLifecycleEventType, MeterStatus};
    use std::collections::BTreeSet;

    macro_rules! check {
        ($ty:ty) => {{
            let all = <$ty>::ALL;
            assert_eq!(all.len(), <$ty>::CODES.len(), stringify!($ty));
            let mut seen = BTreeSet::new();
            for (v, code) in all.iter().zip(<$ty>::CODES) {
                assert_eq!(v.as_str(), *code, "{v:?}");
                assert_eq!(&v.to_string(), *code, "{v:?}");
                assert_eq!(&v.to_string().parse::<$ty>().unwrap(), v, "{v:?}");
                // Lenient in, canonical out — as everywhere in this crate.
                assert_eq!(
                    &format!("  {}  ", code.to_lowercase())
                        .parse::<$ty>()
                        .unwrap(),
                    v,
                    "{v:?}"
                );
                assert!(seen.insert(*code), "duplicate code {code}");
            }
            assert!("NOT_A_CODE".parse::<$ty>().is_err(), stringify!($ty));
        }};
    }

    check!(LoadProfile);
    check!(MeterStatus);
    check!(MeterLifecycleEventType);
}

// ── MaloId / MeloId ──────────────────────────────────────────────────────────

mod market_ids {
    use metering::{MaloId, MeloId};
    use proptest::prelude::*;

    /// A structurally valid MaLo-ID: leading digit 1–9, nine free digits, and
    /// the check digit the Bildungsvorschrift derives from them.
    pub fn arb_malo() -> impl Strategy<Value = MaloId> {
        ("[1-9][0-9]{9}").prop_map(|body| {
            let check = MaloId::compute_check_digit(&body).expect("ten digits");
            format!("{body}{check}").parse().expect("constructed valid")
        })
    }

    proptest! {
        /// Stability + totality: eleven digits out, the same value back.
        #[test]
        fn malo_display_round_trips(id in arb_malo()) {
            let s = id.to_string();
            prop_assert_eq!(s.len(), MaloId::LEN);
            prop_assert_eq!(s.parse::<MaloId>(), Ok(id));
        }

        /// Injectivity is trivial here — the string *is* the value — but the
        /// check digit adds a stronger property: a single-digit corruption of
        /// the first ten digits is rejected, **except** the scheme's one blind
        /// spot — a ±5 change in an even position, which doubles to a shift of
        /// exactly 10 and so leaves the check digit unchanged. The exception
        /// is asserted explicitly below rather than glossed over.
        #[test]
        fn malo_single_digit_corruption_is_caught(id in arb_malo(), pos in 0usize..10, bump in 1u8..10) {
            let mut bytes = id.to_string().into_bytes();
            bytes[pos] = b'0' + ((bytes[pos] - b'0') + bump) % 10;
            let corrupted = String::from_utf8(bytes).unwrap();
            // 1-based even positions are the doubled group: a bump of 5 there
            // shifts the total by 10 and is undetectable by construction.
            let blind_spot = pos % 2 == 1 && bump == 5;
            if blind_spot {
                prop_assert!(corrupted.parse::<MaloId>().is_ok(), "{}", corrupted);
            } else {
                // Corrupting the leading digit to 0 fails on structure;
                // everything else fails on the check digit.
                prop_assert!(corrupted.parse::<MaloId>().is_err(), "{}", corrupted);
            }
        }

        /// A well-shaped MeLo round-trips, and lowercase input converges on
        /// the uppercase spelling.
        #[test]
        fn melo_round_trips_and_uppercases(tail in "[A-Z0-9]{25}") {
            let canonical = format!("DE000562{tail}");
            let id: MeloId = canonical.parse().unwrap();
            prop_assert_eq!(id.as_str(), canonical.as_str());
            let relaxed: MeloId = canonical.to_lowercase().parse().unwrap();
            prop_assert_eq!(relaxed, id);
        }
    }
}

// ── The serde form is the same string ────────────────────────────────────────

#[cfg(feature = "serde")]
mod serde_agrees_with_display {
    use super::{arb_obis, arb_resolution};
    use metering::{IntervalResolution, ObisCode};
    use proptest::prelude::*;

    proptest! {
        /// A value written by `serde` and a value written by `Display` must be
        /// the same bytes. Two writers, one spelling.
        #[test]
        fn obis_serde_form_is_the_display_form(code in arb_obis()) {
            let encoded = serde_json::to_string(&code).unwrap();
            prop_assert_eq!(&encoded, &format!("\"{code}\""));
            prop_assert_eq!(serde_json::from_str::<ObisCode>(&encoded).unwrap(), code);
        }

        #[test]
        fn resolution_serde_form_is_the_display_form(r in arb_resolution()) {
            let encoded = serde_json::to_string(&r).unwrap();
            prop_assert_eq!(&encoded, &format!("\"{r}\""));
            prop_assert_eq!(serde_json::from_str::<IntervalResolution>(&encoded).unwrap(), r);
        }
    }

    /// Both spellings deserialise, so archives written before the canonical
    /// form was pinned still read.
    #[test]
    fn obis_deserialises_from_either_spelling() {
        let short: ObisCode = serde_json::from_str("\"1-0:1.8.0\"").unwrap();
        let long: ObisCode = serde_json::from_str("\"1-0:1.8.0*255\"").unwrap();
        assert_eq!(short, long);
        assert_eq!(short, ObisCode::STROM_BEZUG_TOTAL);
    }

    /// Non-borrowing deserialisers must work.
    ///
    /// `serde(try_from = "&str")` required the deserialiser to hand out a
    /// borrowed `&str`, so `from_reader`, bincode, postcard and MessagePack all
    /// failed with "invalid type: string … expected a borrowed string" —
    /// regardless of what the payload said. Streaming a file of intervals was
    /// simply impossible.
    #[test]
    fn obis_deserialises_from_a_non_borrowing_deserialiser() {
        let json = r#"["1-0:1.8.0","7-0:3.0.0","1-0:1.8.0*1"]"#;
        let decoded: Vec<ObisCode> = serde_json::from_reader(std::io::Cursor::new(json))
            .expect("a reader-backed deserialiser must work");
        assert_eq!(
            decoded,
            vec![
                ObisCode::STROM_BEZUG_TOTAL,
                ObisCode::GAS_VOLUME_M3,
                "1-0:1.8.0*1".parse().unwrap(),
            ]
        );
    }

    #[test]
    fn resolution_deserialises_from_a_non_borrowing_deserialiser() {
        let json = r#"["PT15M","P1D","PT300S"]"#;
        let decoded: Vec<IntervalResolution> =
            serde_json::from_reader(std::io::Cursor::new(json)).expect("reader-backed");
        assert_eq!(
            decoded,
            vec![
                IntervalResolution::QuarterHour,
                IntervalResolution::Day,
                IntervalResolution::from_seconds(300).unwrap(),
            ]
        );
    }

    /// A malformed string is an error, never a silent default.
    #[test]
    fn malformed_strings_are_rejected() {
        assert!(serde_json::from_str::<ObisCode>("\"1-0:1.8\"").is_err());
        assert!(serde_json::from_str::<ObisCode>("\"\"").is_err());
        assert!(serde_json::from_str::<IntervalResolution>("\"hourly\"").is_err());
        // The old derived spellings are gone and must not be silently accepted.
        assert!(serde_json::from_str::<IntervalResolution>("\"QuarterHour\"").is_err());
        assert!(serde_json::from_str::<IntervalResolution>(r#"{"Custom":300}"#).is_err());
    }
}
