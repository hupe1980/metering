//! Every timestamp and every quantity a `serde`-derived type carries names its
//! wire format, and the manifest reaches for no `rust_decimal` serde feature.
//!
//! These read the source, not the serialised output, so they run without the
//! `serde` feature too: a field that forgets its `serde(with)` is wrong whether
//! or not this build compiled the impl.

use super::{read, root, src_files};

/// One field of a `serde`-derived type.
#[derive(Debug)]
struct ScannedField {
    /// `file.rs: value: Decimal` — enough to find it by eye.
    location: String,
    /// The file, relative to `src/`.
    file: String,
    /// The declared type, with `pub`, inline attributes and the trailing
    /// comma stripped.
    ty: String,
    /// Every attribute on the field: the lines directly above it and any
    /// written inline, inside a tuple's parentheses.
    attrs: String,
}

/// Every field of every `serde`-derived `struct` and `enum` in `text`.
///
/// A line-by-line walk, because the question is *which attributes sit directly
/// above this field* and only adjacency answers it. Named fields, tuple
/// structs, tuple variants and one-line struct variants all count: a `Decimal`
/// in any of them travels the same wire.
fn scan_serde_fields(file: &str, text: &str) -> Vec<ScannedField> {
    let mut found = Vec::new();
    let mut attrs = String::new();
    let mut depth = 0usize;
    let mut item_is_serde = false;

    let push = |found: &mut Vec<ScannedField>, name: &str, ty: &str, attrs: &str| {
        found.push(ScannedField {
            location: format!("{file}: {name}: {ty}"),
            file: file.to_owned(),
            ty: ty.to_owned(),
            attrs: attrs.to_owned(),
        });
    };

    for line in text.lines() {
        let trimmed = line.trim();

        // An attribute may span several lines; it is finished when its
        // parentheses balance. Without this, the second line of a multi-line
        // `cfg_attr` reads as an ordinary statement and discards the `derive`
        // above it.
        let attrs_open = attrs.matches('(').count() != attrs.matches(')').count();
        if trimmed.starts_with("#[") || attrs_open {
            attrs.push_str(trimmed);
            continue;
        }

        if depth == 0 {
            let decl = strip_visibility(trimmed);
            let is_item = decl.starts_with("struct ") || decl.starts_with("enum ");
            if is_item && attrs.contains("Serialize") {
                // `struct Wrapper(#[serde(with = …)] Decimal);`
                if decl.starts_with("struct ") && decl.ends_with(';') && decl.contains('(') {
                    for (i, (inline, ty)) in tuple_fields(decl).into_iter().enumerate() {
                        push(&mut found, &i.to_string(), &ty, &format!("{attrs}{inline}"));
                    }
                } else if trimmed.ends_with('{') {
                    item_is_serde = true;
                    depth = 1;
                }
            } else if is_item && trimmed.ends_with('{') {
                item_is_serde = false;
                depth = 1;
            }
            if !trimmed.starts_with("//") {
                attrs.clear();
            }
            continue;
        }

        if !trimmed.starts_with("//") {
            if item_is_serde {
                if let Some((name, ty)) = field_decl(trimmed) {
                    push(&mut found, name, ty, &attrs);
                } else if depth == 1
                    && let Some((variant, rest)) = variant_head(trimmed)
                {
                    if rest.starts_with('(') {
                        // `Fixed(ObisCode),` — a tuple variant.
                        for (inline, ty) in tuple_fields(rest) {
                            push(&mut found, variant, &ty, &format!("{attrs}{inline}"));
                        }
                    } else if rest.starts_with('{') && rest.contains('}') {
                        // `Window { from: OffsetDateTime },` on one line.
                        let inner = &rest[1..rest.rfind('}').unwrap_or(rest.len())];
                        for part in split_top_level(inner) {
                            let (inline, decl) = strip_inline_attrs(part.trim());
                            if let Some((name, ty)) = decl.split_once(':') {
                                let ty = ty.trim();
                                push(&mut found, name.trim(), ty, &format!("{attrs}{inline}"));
                            }
                        }
                    }
                }
            }
            attrs.clear();
        }

        depth = (depth + trimmed.matches('{').count()).saturating_sub(trimmed.matches('}').count());
        if depth == 0 {
            item_is_serde = false;
            attrs.clear();
        }
    }

    found
}

fn strip_visibility(s: &str) -> &str {
    s.strip_prefix("pub(crate) ")
        .or_else(|| s.strip_prefix("pub(super) "))
        .or_else(|| s.strip_prefix("pub "))
        .unwrap_or(s)
}

/// `name: Type,` split into its two halves, or `None` for any other line.
fn field_decl(trimmed: &str) -> Option<(&str, &str)> {
    let body = strip_visibility(trimmed).strip_suffix(',')?;
    let (name, ty) = body.split_once(": ")?;
    let is_ident = !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    is_ident.then_some((name, ty))
}

/// `Variant(…),` or `Variant { … },` split into the name and the rest.
fn variant_head(trimmed: &str) -> Option<(&str, &str)> {
    if !trimmed.starts_with(char::is_uppercase) {
        return None;
    }
    let end = trimmed.find(|c: char| !(c.is_alphanumeric() || c == '_'))?;
    Some((&trimmed[..end], trimmed[end..].trim_start()))
}

/// The fields inside the first pair of parentheses of `decl`, each as
/// `(inline attributes, type)`.
fn tuple_fields(decl: &str) -> Vec<(String, String)> {
    let Some(open) = decl.find('(') else {
        return Vec::new();
    };
    let Some(close) = decl.rfind(')') else {
        return Vec::new();
    };
    split_top_level(&decl[open + 1..close])
        .into_iter()
        .filter(|part| !part.trim().is_empty())
        .map(|part| {
            let (inline, ty) = strip_inline_attrs(part.trim());
            (inline, strip_visibility(ty).trim().to_owned())
        })
        .collect()
}

/// Split on commas that are not nested inside `<>`, `()` or `[]`.
fn split_top_level(s: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let (mut depth, mut start) = (0i32, 0usize);
    for (i, c) in s.char_indices() {
        match c {
            '<' | '(' | '[' => depth += 1,
            '>' | ')' | ']' => depth -= 1,
            ',' if depth == 0 => {
                parts.push(&s[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    parts.push(&s[start..]);
    parts
}

/// Leading `#[…]` attributes of a tuple field, and the rest.
fn strip_inline_attrs(mut s: &str) -> (String, &str) {
    let mut attrs = String::new();
    while s.starts_with("#[") {
        let mut depth = 0i32;
        let mut end = s.len();
        for (i, c) in s.char_indices() {
            match c {
                '[' => depth += 1,
                ']' => {
                    depth -= 1;
                    if depth == 0 {
                        end = i + 1;
                        break;
                    }
                }
                _ => {}
            }
        }
        attrs.push_str(&s[..end]);
        s = s[end..].trim_start();
    }
    (attrs, s)
}

/// Every serde field under `src/`.
fn src_serde_fields() -> Vec<ScannedField> {
    src_files()
        .iter()
        .flat_map(|(file, text)| scan_serde_fields(file, text))
        .collect()
}

fn is_timestamp(ty: &str) -> bool {
    matches!(
        ty,
        "OffsetDateTime" | "Option<OffsetDateTime>" | "Date" | "Option<Date>"
    )
}

/// Whether a field names its representation: through `crate::wire`, or —
/// inside `wire.rs`, whose helpers wrap one another — through any
/// `serde(with)`.
fn names_wire_format(f: &ScannedField, module: &str) -> bool {
    f.attrs.contains(&format!("wire::{module}"))
        || (f.file == "wire.rs" && f.attrs.contains("serde(with"))
}

/// Fields of `fields` that are timestamps without a `crate::wire` format.
fn bare_timestamps(fields: &[ScannedField]) -> Vec<&str> {
    fields
        .iter()
        .filter(|f| is_timestamp(&f.ty) && !names_wire_format(f, ""))
        .map(|f| f.location.as_str())
        .collect()
}

/// Fields of `fields` that hold a `Decimal` without `crate::wire::decimal`.
fn bare_decimals(fields: &[ScannedField]) -> Vec<&str> {
    fields
        .iter()
        .filter(|f| f.ty.contains("Decimal") && !names_wire_format(f, "decimal"))
        .map(|f| f.location.as_str())
        .collect()
}

/// No timestamp field escapes the wire format.
///
/// `src/wire.rs` only applies where a field asks for it, and a field that
/// forgets falls silently back to `time`'s ordinal tuple — in JSON, in one
/// struct, next to siblings that are RFC 3339 strings. Nothing fails to compile
/// and nothing fails to round-trip; the format is just inconsistent, which is
/// the worst kind of wire mistake to find later.
#[test]
fn no_timestamp_field_escapes_the_wire_format() {
    let fields = src_serde_fields();
    let timestamps = fields.iter().filter(|f| is_timestamp(&f.ty)).count();
    assert!(
        timestamps > 15,
        "the scan found only {timestamps} timestamp fields — it has stopped working, not the crate",
    );
    let missing = bare_timestamps(&fields);
    assert!(
        missing.is_empty(),
        "timestamp fields with no wire format, so they travel as `time`'s \
         ordinal tuple while their siblings are RFC 3339: {missing:#?}",
    );
}

/// No quantity escapes the wire format either.
///
/// A `Decimal` field with no `serde(with)` travels however `rust_decimal`'s
/// features happened to unify in the consumer's build — including as an `f64`,
/// chosen by a crate that never named `metering`.
#[test]
fn no_decimal_field_escapes_the_wire_format() {
    let fields = src_serde_fields();
    let quantities = fields.iter().filter(|f| f.ty.contains("Decimal")).count();
    // A sentinel for a broken scan, not a count to defend.
    assert!(
        quantities > 55,
        "the scan found only {quantities} quantity fields — it has stopped working, not the crate",
    );
    let missing = bare_decimals(&fields);
    assert!(
        missing.is_empty(),
        "quantity fields with no wire format, so their representation is \
         whatever `rust_decimal` features the consumer's graph unified to: {missing:#?}",
    );
}

/// Lines of `manifest` that enable a `serde` feature of `rust_decimal`.
///
/// Both spellings count: `rust_decimal/serde-str` in a `[features]` list, and
/// a `features = […]` on the `rust_decimal` dependency itself — inline,
/// spread over several lines, or under a `[dependencies.rust_decimal]` table.
fn decimal_serde_features(manifest: &str) -> Vec<String> {
    let mut offenders = Vec::new();
    let mut table = String::new();
    let mut pending: Option<String> = None;

    for line in manifest.lines() {
        let code = line.split('#').next().unwrap_or_default().trim();
        if code.is_empty() {
            continue;
        }
        if let Some(acc) = &mut pending {
            acc.push(' ');
            acc.push_str(code);
            if balanced(acc) {
                if acc.contains("serde") {
                    offenders.push(acc.clone());
                }
                pending = None;
            }
            continue;
        }
        if code.starts_with('[') && !code.contains('=') {
            table = code.to_owned();
            continue;
        }
        if code.contains("rust_decimal/serde") {
            offenders.push(code.to_owned());
            continue;
        }
        let is_dependency = code.starts_with("rust_decimal ") || code.starts_with("rust_decimal=");
        let in_table = table.ends_with(".rust_decimal]") && code.starts_with("features");
        if is_dependency || in_table {
            let value = code.split_once('=').map_or("", |(_, v)| v);
            if balanced(value) {
                if value.contains("serde") {
                    offenders.push(code.to_owned());
                }
            } else {
                pending = Some(code.to_owned());
            }
        }
    }
    offenders
}

fn balanced(s: &str) -> bool {
    s.matches('[').count() == s.matches(']').count()
        && s.matches('{').count() == s.matches('}').count()
}

/// The crate enables no `rust_decimal` `serde` feature — the mechanism behind
/// the scans above.
///
/// Cargo features are global to a build graph, so one enabled here decides how
/// `Decimal` behaves in crates that never named `metering`. `src/wire.rs`
/// states the representation per field instead.
#[test]
fn the_crate_reaches_for_no_rust_decimal_serde_feature() {
    let manifest = read(&root().join("Cargo.toml"));
    assert!(
        manifest.contains("rust_decimal"),
        "the manifest no longer names rust_decimal — the scan reads the wrong file"
    );
    let offenders = decimal_serde_features(&manifest);
    assert!(
        offenders.is_empty(),
        "enabling a `rust_decimal` serde feature changes how `Decimal` \
         deserialises for every crate in the consumer's build graph: {offenders:#?}",
    );
}

/// The scanners in this file catch what they are for.
#[test]
fn the_wire_scanners_catch_planted_violations() {
    let planted = r#"
#[cfg_attr(
    feature = "serde",
    derive(Serialize, Deserialize)
)]
pub struct Planted {
    pub at: OffsetDateTime,
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::rfc3339"))]
    pub until: OffsetDateTime,
    value: Decimal,
    #[cfg_attr(feature = "serde", serde(with = "crate::wire::decimal"))]
    kept: Decimal,
}

#[derive(Serialize)]
pub struct Bare(pub Decimal);

#[derive(Serialize)]
struct Wrapped(#[serde(with = "crate::wire::decimal")] Decimal);

#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum Variants {
    Plain,
    Amount(Decimal),
    Stamped(#[cfg_attr(feature = "serde", serde(with = "crate::wire::rfc3339"))] OffsetDateTime),
    At(OffsetDateTime, u8),
    Window { from: OffsetDateTime },
    Spread {
        weights: BTreeMap<MeloId, Decimal>,
    },
}

#[derive(Debug)]
pub struct NotSerde {
    value: Decimal,
}
"#;
    let fields = scan_serde_fields("planted.rs", planted);
    assert_eq!(
        bare_timestamps(&fields),
        [
            "planted.rs: at: OffsetDateTime",
            "planted.rs: At: OffsetDateTime",
            "planted.rs: from: OffsetDateTime",
        ]
    );
    assert_eq!(
        bare_decimals(&fields),
        [
            "planted.rs: value: Decimal",
            "planted.rs: 0: Decimal",
            "planted.rs: Amount: Decimal",
            "planted.rs: weights: BTreeMap<MeloId, Decimal>",
        ]
    );

    // Inside `wire.rs` a wrapper names its sibling module; elsewhere that is
    // not a wire format.
    let wrapper = "#[derive(Serialize)]\nstruct Wrapper(#[serde(with = \"super::rfc3339\")] OffsetDateTime);\n";
    assert!(bare_timestamps(&scan_serde_fields("wire.rs", wrapper)).is_empty());
    assert_eq!(
        bare_timestamps(&scan_serde_fields("other.rs", wrapper)).len(),
        1
    );

    // The manifest, in every spelling that turns the feature on.
    for planted in [
        r#"rust_decimal = { version = "1.37", features = ["macros", "serde-str"] }"#,
        r#"rust_decimal = { version = "1.37", features = ["serde-with-float"] }"#,
        "rust_decimal = { version = \"1.37\", features = [\n    \"macros\",\n    \"serde-str\",\n] }",
        "[dependencies.rust_decimal]\nversion = \"1.37\"\nfeatures = [\"serde-arbitrary-precision\"]",
        "[features]\nserde = [\"dep:serde\", \"rust_decimal/serde-str\"]",
    ] {
        assert_eq!(decimal_serde_features(planted).len(), 1, "{planted}");
    }
    for clean in [
        r#"rust_decimal = { version = "1.37", features = ["macros"] }"#,
        "# rust_decimal = { features = [\"serde-str\"] }",
        r#"serde = { version = "1", features = ["derive"] }"#,
        "[dependencies.rust_decimal]\nversion = \"1.37\"\n[dependencies.serde]\nfeatures = [\"derive\"]",
    ] {
        assert!(decimal_serde_features(clean).is_empty(), "{clean}");
    }
}
