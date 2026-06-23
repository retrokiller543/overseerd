use serde_yaml_ng::Value as YamlValue;

use crate::error::{ConfigError, ConfigErrorKind};
use crate::value::{ConfigStr, ConfigValue};

/// Parses YAML source text and normalizes it into the [`ConfigValue`] tree.
pub fn from_str(text: &str) -> Result<ConfigValue, ConfigError> {
    let value: YamlValue =
        serde_yaml_ng::from_str(text).map_err(|e| ConfigErrorKind::Message(e.to_string()))?;

    from_yaml(value)
}

/// Normalizes a parsed YAML value into the shared [`ConfigValue`] tree, parsing
/// placeholders in every string leaf. Non-string mapping keys are stringified;
/// tagged values are unwrapped to their inner value.
pub fn from_yaml(value: YamlValue) -> Result<ConfigValue, ConfigError> {
    let normalized = match value {
        YamlValue::Null => ConfigValue::Null,
        YamlValue::Bool(b) => ConfigValue::Bool(b),
        YamlValue::String(s) => ConfigValue::Str(ConfigStr::parse(&s)?),
        YamlValue::Number(n) => number_to_value(n)?,

        YamlValue::Sequence(items) => {
            let mut out = Vec::with_capacity(items.len());

            for item in items {
                out.push(from_yaml(item)?);
            }

            ConfigValue::Array(out)
        }

        YamlValue::Mapping(mapping) => {
            let mut out = Vec::with_capacity(mapping.len());

            for (key, item) in mapping {
                out.push((stringify_key(key), from_yaml(item)?));
            }

            ConfigValue::Table(out)
        }

        YamlValue::Tagged(tagged) => from_yaml(tagged.value)?,
    };

    Ok(normalized)
}

/// Converts a YAML number to an integer or float `ConfigValue`, widening integers to
/// `i128`.
fn number_to_value(n: serde_yaml_ng::Number) -> Result<ConfigValue, ConfigError> {
    if let Some(i) = n.as_i64() {
        return Ok(ConfigValue::Int(i128::from(i)));
    }

    if let Some(u) = n.as_u64() {
        return Ok(ConfigValue::Int(i128::from(u)));
    }

    if let Some(f) = n.as_f64() {
        return Ok(ConfigValue::Float(f));
    }

    Err(ConfigErrorKind::Message(format!("unrepresentable YAML number: {n}")).into())
}

/// Renders a mapping key to its string form. Scalar keys use their natural text;
/// anything else falls back to YAML's debug rendering.
fn stringify_key(key: YamlValue) -> String {
    match key {
        YamlValue::String(s) => s,
        YamlValue::Bool(b) => b.to_string(),
        YamlValue::Number(n) => n.to_string(),
        other => format!("{other:?}"),
    }
}
