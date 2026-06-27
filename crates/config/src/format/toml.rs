use toml::Value as TomlValue;

use crate::error::{TemplateError, TemplateErrorKind};
use crate::value::{ConfigStr, ConfigValue};

/// Parses TOML source text and normalizes it into the [`ConfigValue`] tree.
pub fn from_str(text: &str) -> Result<ConfigValue, TemplateError> {
    let value: TomlValue =
        toml::from_str(text).map_err(|e| TemplateErrorKind::Message(e.to_string()))?;

    from_toml(value)
}

/// Normalizes a parsed TOML value into the shared [`ConfigValue`] tree, parsing
/// placeholders in every string leaf. Datetimes are rendered to their TOML string
/// form (a literal leaf), so they round-trip into `String` targets.
pub fn from_toml(value: TomlValue) -> Result<ConfigValue, TemplateError> {
    let normalized = match value {
        TomlValue::String(s) => ConfigValue::Str(ConfigStr::parse(&s)?),
        TomlValue::Integer(n) => ConfigValue::Int(i128::from(n)),
        TomlValue::Float(f) => ConfigValue::Float(f),
        TomlValue::Boolean(b) => ConfigValue::Bool(b),
        TomlValue::Datetime(dt) => ConfigValue::Str(ConfigStr::parse(&dt.to_string())?),

        TomlValue::Array(items) => {
            let mut out = Vec::with_capacity(items.len());

            for item in items {
                out.push(from_toml(item)?);
            }

            ConfigValue::Array(out)
        }

        TomlValue::Table(table) => {
            let mut out = Vec::with_capacity(table.len());

            for (key, item) in table {
                out.push((key, from_toml(item)?));
            }

            ConfigValue::Table(out)
        }
    };

    Ok(normalized)
}
