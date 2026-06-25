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
/// phantom variant branch would confuse the deserializer.
#[derive(Debug, Clone, Default)]
pub enum DefaultSpec {
    /// No field carries a default.
    #[default]
    None,

    /// `(field, raw default template)` pairs for a struct.
    Fields(Vec<(String, String)>),

    /// `(variant, [(field, raw default template)])` for an enum.
    Variants(Vec<(String, Vec<(String, String)>)>),
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
    /// are filled; an absent variant is left untouched.
    pub fn fill_missing(&self, subtree: &mut ConfigValue) -> Result<(), ConfigError> {
        match self {
            DefaultSpec::None => Ok(()),
            DefaultSpec::Fields(fields) => fill_fields(subtree, fields),
            DefaultSpec::Variants(variants) => fill_variants(subtree, variants),
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

/// Fills the fields of the present variant only. A unit-string subtree carries no fields, so
/// it is left untouched; a single-entry tag table has its inner fields filled.
fn fill_variants(
    subtree: &mut ConfigValue,
    variants: &[(String, Vec<(String, String)>)],
) -> Result<(), ConfigError> {
    let ConfigValue::Table(entries) = subtree else {
        return Ok(());
    };

    for (tag, inner) in entries.iter_mut() {
        let fields = match variants.iter().find(|(name, _)| name == tag) {
            Some((_, fields)) => fields,
            None => continue,
        };

        fill_fields(inner, fields)?;
    }

    Ok(())
}
