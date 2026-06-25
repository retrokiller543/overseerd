//! Macro-supplied field defaults, applied by filling missing leaves before
//! deserialization.
//!
//! A config type declares per-field default *template strings* (via `#[default = ".."]`).
//! Those defaults are not evaluated in Rust — that would lose templating. Instead they are
//! parsed into config-string leaves and merged *under* the file values, so a missing field
//! falls back to its default and resolves through the normal `${...}` pipeline, producing
//! the real typed value.

use crate::error::ConfigError;
use crate::value::ConfigValue;

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
        /// The `#[default]` variant's `(serde tag, is_unit)`, selected when the config
        /// names no variant. `is_unit` chooses the synthesized shape: a bare tag string
        /// for a unit variant, or a `{ tag: { ..defaults } }` table otherwise.
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
    /// the variant present in the subtree (a single-entry tag table or a bare unit string)
    /// are filled; when no variant is present the `#[default]` variant (if any) is
    /// synthesized.
    pub fn fill_missing(&self, subtree: &mut ConfigValue) -> Result<(), ConfigError> {
        match self {
            DefaultSpec::None => Ok(()),
            DefaultSpec::Fields(fields) => fill_fields(subtree, fields),
            DefaultSpec::Variants { default, fields } => fill_variants(subtree, default, fields),
        }
    }
}

/// Ensures `subtree` is a table and adds any field absent from it, parsed from its default
/// template. A non-table subtree is left as-is (the deserializer reports the type mismatch).
fn fill_fields(subtree: &mut ConfigValue, fields: &[(String, String)]) -> Result<(), ConfigError> {
    let ConfigValue::Table(entries) = subtree else {
        return Ok(());
    };

    for (field, raw) in fields {
        let present = entries.iter().any(|(key, _)| key == field);

        if !present {
            let value = ConfigValue::parsed_str(raw)?;

            entries.push((field.clone(), value));
        }
    }

    Ok(())
}

/// Applies enum defaults.
///
/// When a variant is already selected — a single-entry tag table or a bare unit string —
/// only that variant's fields are filled (an unselected variant is never materialized). When
/// no variant is selected (an empty table, e.g. an absent or bare `[section]`), the
/// `#[default]` variant is synthesized: a bare tag string for a unit variant, or a
/// `{ tag: { ..defaults } }` table otherwise. With no default variant the subtree is left
/// empty for the deserializer to reject.
fn fill_variants(
    subtree: &mut ConfigValue,
    default: &Option<(String, bool)>,
    fields: &[(String, Vec<(String, String)>)],
) -> Result<(), ConfigError> {
    let no_variant_selected = matches!(subtree, ConfigValue::Table(entries) if entries.is_empty());

    if !no_variant_selected {
        if let ConfigValue::Table(entries) = subtree {
            for (tag, inner) in entries.iter_mut() {
                if let Some((_, variant_fields)) = fields.iter().find(|(name, _)| name == tag) {
                    fill_fields(inner, variant_fields)?;
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

    if let Some((_, variant_fields)) = fields.iter().find(|(name, _)| name == tag) {
        fill_fields(&mut inner, variant_fields)?;
    }

    *subtree = ConfigValue::Table(vec![(tag.clone(), inner)]);

    Ok(())
}
