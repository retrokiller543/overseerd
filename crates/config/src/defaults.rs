//! Macro-supplied field defaults, applied by filling missing leaves before
//! deserialization.
//!
//! A config type declares per-field default *template strings* (via `#[default = ".."]`).
//! Those defaults are not evaluated in Rust — that would lose templating. Instead they are
//! parsed into config-string leaves and merged *under* the file values, so a missing field
//! falls back to its default and resolves through the normal `${...}` pipeline, producing
//! the real typed value.

use crate::error::ConfigError;
use crate::value::{ConfigStr, ConfigValue};

/// How an enum is tagged, so the merge can synthesize the same shape serde deserializes.
///
/// Mirrors serde's representations: externally tagged (`{ Variant: {..} }`), internally
/// tagged (`{ tag: "variant", ..fields }`), adjacently tagged (`{ tag: "variant", content:
/// {..} }`), and untagged (no tag — defaults cannot pick a variant).
#[derive(Debug, Clone, Default)]
pub enum EnumTag {
    /// `#[serde]` default: `{ Variant: {..} }` or a bare `"Variant"` unit string.
    #[default]
    External,

    /// `#[serde(tag = "..")]`: the tag field lives inline beside the variant's fields.
    Internal { tag: String },

    /// `#[serde(tag = "..", content = "..")]`: the variant's fields live under `content`.
    Adjacent { tag: String, content: String },

    /// `#[serde(untagged)]`: no tag, so a default variant cannot be synthesized.
    Untagged,
}

/// The shape of a config type's field defaults, emitted by the `#[config]` macro and
/// consumed by the merge step.
///
/// Struct defaults fill every missing field; enum defaults fill only the fields of the
/// variant actually present in the config, since variants are mutually exclusive and a
/// phantom variant branch would confuse the deserializer. An enum may also name a
/// **default variant** (`#[default]` on a variant), used when the config selects none.
#[derive(Debug, Clone, Default)]
pub enum DefaultSpec {
    /// No field carries a default.
    #[default]
    None,

    /// `(field, raw default template)` pairs for a struct.
    Fields(Vec<(String, String)>),

    /// Enum defaults.
    Variants {
        /// How the enum is tagged, which decides the synthesized shape.
        tagging: EnumTag,

        /// The `#[default]` variant's `(serde tag, is_unit)`, selected when the config
        /// names no variant. `is_unit` chooses the synthesized shape: no field payload for
        /// a unit variant, the variant's defaults otherwise.
        default: Option<(String, bool)>,

        /// Per-variant field defaults, keyed by serde tag:
        /// `(variant, [(field, raw default template)])`.
        fields: Vec<(String, Vec<(String, String)>)>,
    },
}

impl DefaultSpec {
    /// The empty default — the trait default when a type declares no field defaults.
    pub fn none() -> Self {
        DefaultSpec::None
    }

    /// Fills missing leaves of `subtree` in place from these defaults. Existing values are
    /// never overwritten; only absent fields are added.
    ///
    /// For [`Fields`](DefaultSpec::Fields) the subtree is treated as a table and every
    /// missing field is filled. For [`Variants`](DefaultSpec::Variants) only the fields of
    /// the variant present in the config are filled; when no variant is present the
    /// `#[default]` variant (if any) is synthesized in the enum's tagged shape.
    pub fn fill_missing(&self, subtree: &mut ConfigValue) -> Result<(), ConfigError> {
        match self {
            DefaultSpec::None => Ok(()),
            DefaultSpec::Fields(fields) => fill_fields(subtree, fields, false),

            DefaultSpec::Variants {
                tagging,
                default,
                fields,
            } => fill_variants(subtree, tagging, default, fields),
        }
    }
}

/// Ensures `subtree` is a table and adds any field absent from it, parsed from its default
/// template. A non-table subtree is left as-is (the deserializer reports the type mismatch).
///
/// `coerce` types literal scalar defaults (numbers/bools) as `Int`/`Bool`/`Float` rather than
/// strings — required where serde buffers content (internally/adjacently-tagged enums) and so
/// would not coerce a string default to a numeric field. See [`default_value`].
fn fill_fields(
    subtree: &mut ConfigValue,
    fields: &[(String, String)],
    coerce: bool,
) -> Result<(), ConfigError> {
    let ConfigValue::Table(entries) = subtree else {
        return Ok(());
    };

    fill_entries(entries, fields, coerce)
}

/// Adds any field absent from `entries`, parsed from its default template.
fn fill_entries(
    entries: &mut Vec<(String, ConfigValue)>,
    fields: &[(String, String)],
    coerce: bool,
) -> Result<(), ConfigError> {
    for (field, raw) in fields {
        let present = entries.iter().any(|(key, _)| key == field);

        if !present {
            let value = default_value(raw, coerce)?;

            entries.push((field.clone(), value));
        }
    }

    Ok(())
}

/// Parses a default template into a config value.
///
/// A template (anything with a `${..}` placeholder) is always a string leaf, since it resolves
/// through the pipeline. When `coerce` is set, a placeholder-free literal that reads as an
/// integer, bool, or float is stored as that scalar — matching how a config file would carry
/// it, so serde's content-buffered deserialization (internally/adjacently-tagged enums) sees a
/// numeric field as a number rather than a string it refuses to coerce.
fn default_value(raw: &str, coerce: bool) -> Result<ConfigValue, ConfigError> {
    let parsed = ConfigStr::parse(raw)?;

    if coerce && let Some(literal) = parsed.as_literal() {
        if let Ok(int) = literal.parse::<i128>() {
            return Ok(ConfigValue::Int(int));
        }

        if let Ok(boolean) = literal.parse::<bool>() {
            return Ok(ConfigValue::Bool(boolean));
        }

        if let Ok(float) = literal.parse::<f64>() {
            return Ok(ConfigValue::Float(float));
        }
    }

    Ok(ConfigValue::Str(parsed))
}

/// Applies enum defaults in the enum's tagged shape.
fn fill_variants(
    subtree: &mut ConfigValue,
    tagging: &EnumTag,
    default: &Option<(String, bool)>,
    fields: &[(String, Vec<(String, String)>)],
) -> Result<(), ConfigError> {
    match tagging {
        EnumTag::External => fill_external(subtree, default, fields),
        EnumTag::Internal { tag } => fill_internal(subtree, tag, default, fields),
        EnumTag::Adjacent { tag, content } => fill_adjacent(subtree, tag, content, default, fields),
        // Untagged enums carry no discriminant, so a default variant cannot be chosen.
        EnumTag::Untagged => Ok(()),
    }
}

/// The variant's field defaults, by serde tag.
fn variant_fields<'a>(
    fields: &'a [(String, Vec<(String, String)>)],
    tag: &str,
) -> Option<&'a [(String, String)]> {
    fields
        .iter()
        .find(|(name, _)| name == tag)
        .map(|(_, defaults)| defaults.as_slice())
}

/// Externally tagged (`{ Variant: {..} }` / `"Variant"`): a selected variant is a
/// single-entry tag table whose inner fields are filled; when none is selected (an empty
/// table) the default variant is synthesized as a bare tag string (unit) or a
/// `{ tag: { ..defaults } }` table.
fn fill_external(
    subtree: &mut ConfigValue,
    default: &Option<(String, bool)>,
    fields: &[(String, Vec<(String, String)>)],
) -> Result<(), ConfigError> {
    let no_variant_selected = matches!(subtree, ConfigValue::Table(entries) if entries.is_empty());

    if !no_variant_selected {
        if let ConfigValue::Table(entries) = subtree {
            for (tag, inner) in entries.iter_mut() {
                if let Some(variant_fields) = variant_fields(fields, tag) {
                    // Externally tagged: the variant payload deserializes directly with its
                    // known field types, so string defaults coerce — no need to pre-type them.
                    fill_fields(inner, variant_fields, false)?;
                }
            }
        }

        return Ok(());
    }

    let (tag, is_unit) = match default {
        Some(default) => default,
        None => return Ok(()),
    };

    if *is_unit {
        *subtree = ConfigValue::parsed_str(tag)?;

        return Ok(());
    }

    let mut inner = ConfigValue::Table(Vec::new());

    if let Some(variant_fields) = variant_fields(fields, tag) {
        fill_fields(&mut inner, variant_fields, false)?;
    }

    *subtree = ConfigValue::Table(vec![(tag.clone(), inner)]);

    Ok(())
}

/// Internally tagged (`{ tag: "variant", ..fields }`): the variant is read from the inline
/// `tag` field and its fields are filled at the same level; when the tag field is absent the
/// default variant is synthesized by inserting the tag field (plus its field defaults).
fn fill_internal(
    subtree: &mut ConfigValue,
    tag_key: &str,
    default: &Option<(String, bool)>,
    fields: &[(String, Vec<(String, String)>)],
) -> Result<(), ConfigError> {
    let ConfigValue::Table(entries) = subtree else {
        return Ok(());
    };

    if let Some(variant) = tag_value(entries, tag_key) {
        if let Some(variant_fields) = variant_fields(fields, &variant) {
            fill_entries(entries, variant_fields, true)?;
        }

        return Ok(());
    }

    let (tag, is_unit) = match default {
        Some(default) => default,
        None => return Ok(()),
    };

    entries.push((tag_key.to_string(), ConfigValue::parsed_str(tag)?));

    if !*is_unit && let Some(variant_fields) = variant_fields(fields, tag) {
        fill_entries(entries, variant_fields, true)?;
    }

    Ok(())
}

/// Adjacently tagged (`{ tag: "variant", content: {..} }`): the variant is read from the
/// `tag` field and its fields filled under `content`; when the tag field is absent the
/// default variant is synthesized as `{ tag: "variant" }` (unit) or with a filled `content`.
fn fill_adjacent(
    subtree: &mut ConfigValue,
    tag_key: &str,
    content_key: &str,
    default: &Option<(String, bool)>,
    fields: &[(String, Vec<(String, String)>)],
) -> Result<(), ConfigError> {
    let ConfigValue::Table(entries) = subtree else {
        return Ok(());
    };

    if let Some(variant) = tag_value(entries, tag_key) {
        if let Some(variant_fields) = variant_fields(fields, &variant) {
            fill_content(entries, content_key, variant_fields)?;
        }

        return Ok(());
    }

    let (tag, is_unit) = match default {
        Some(default) => default,
        None => return Ok(()),
    };

    entries.push((tag_key.to_string(), ConfigValue::parsed_str(tag)?));

    if !*is_unit {
        let variant_fields = variant_fields(fields, tag).unwrap_or(&[]);

        fill_content(entries, content_key, variant_fields)?;
    }

    Ok(())
}

/// Fills `fields` into the `content` sub-table of `entries`, creating it if absent. Coerces
/// scalar defaults, since adjacently-tagged content is also buffered by serde.
fn fill_content(
    entries: &mut Vec<(String, ConfigValue)>,
    content_key: &str,
    fields: &[(String, String)],
) -> Result<(), ConfigError> {
    let content = match entries.iter_mut().find(|(key, _)| key == content_key) {
        Some((_, value)) => value,

        None => {
            entries.push((content_key.to_string(), ConfigValue::Table(Vec::new())));

            &mut entries.last_mut().expect("just pushed").1
        }
    };

    fill_fields(content, fields, true)
}

/// The literal value of the `tag_key` field in `entries`, if it is a plain string — the
/// selected variant's tag. `None` when absent or not a literal string.
fn tag_value(entries: &[(String, ConfigValue)], tag_key: &str) -> Option<String> {
    let value = entries.iter().find(|(key, _)| key == tag_key)?;

    match &value.1 {
        ConfigValue::Str(s) => s.as_literal().map(str::to_string),
        _ => None,
    }
}
