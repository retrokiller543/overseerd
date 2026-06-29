//! A faithful port of serde's `rename_all` case-conversion rules.
//!
//! `#[config]` field defaults must key on the *same* name serde deserializes into, so a
//! `#[serde(rename_all = "..")]` container (or a `#[serde(rename = "..")]` field/variant)
//! moves the default to the matching key. This mirrors `serde_derive`'s `RenameRule` so
//! the two never disagree. Variants are assumed `PascalCase` and fields `snake_case` at
//! the source, exactly as serde assumes.

/// A serde `rename_all` rule.
// Variant names mirror serde's own case names verbatim, so the shared `Case` suffix is
// intentional and the lint does not apply.
#[allow(clippy::enum_variant_names)]
#[derive(Clone, Copy)]
pub enum RenameRule {
    LowerCase,
    UpperCase,
    PascalCase,
    CamelCase,
    SnakeCase,
    ScreamingSnakeCase,
    KebabCase,
    ScreamingKebabCase,
}

impl RenameRule {
    /// Parses a serde `rename_all` literal, returning `None` for an unrecognized value
    /// (serde itself rejects those at derive time, so falling back to the raw name here is
    /// harmless).
    pub fn from_str(rule: &str) -> Option<Self> {
        let parsed = match rule {
            "lowercase" => RenameRule::LowerCase,
            "UPPERCASE" => RenameRule::UpperCase,
            "PascalCase" => RenameRule::PascalCase,
            "camelCase" => RenameRule::CamelCase,
            "snake_case" => RenameRule::SnakeCase,
            "SCREAMING_SNAKE_CASE" => RenameRule::ScreamingSnakeCase,
            "kebab-case" => RenameRule::KebabCase,
            "SCREAMING-KEBAB-CASE" => RenameRule::ScreamingKebabCase,
            _ => return None,
        };

        Some(parsed)
    }

    /// Applies this rule to a `PascalCase` variant name.
    pub fn apply_to_variant(self, variant: &str) -> String {
        match self {
            RenameRule::PascalCase => variant.to_owned(),
            RenameRule::LowerCase => variant.to_ascii_lowercase(),
            RenameRule::UpperCase => variant.to_ascii_uppercase(),
            RenameRule::CamelCase => variant[..1].to_ascii_lowercase() + &variant[1..],

            RenameRule::SnakeCase => {
                let mut snake = String::new();

                for (i, ch) in variant.char_indices() {
                    if i > 0 && ch.is_uppercase() {
                        snake.push('_');
                    }

                    snake.push(ch.to_ascii_lowercase());
                }

                snake
            }

            RenameRule::ScreamingSnakeCase => RenameRule::SnakeCase
                .apply_to_variant(variant)
                .to_ascii_uppercase(),

            RenameRule::KebabCase => RenameRule::SnakeCase
                .apply_to_variant(variant)
                .replace('_', "-"),

            RenameRule::ScreamingKebabCase => RenameRule::ScreamingSnakeCase
                .apply_to_variant(variant)
                .replace('_', "-"),
        }
    }

    /// Applies this rule to a `snake_case` field name.
    pub fn apply_to_field(self, field: &str) -> String {
        match self {
            RenameRule::LowerCase | RenameRule::SnakeCase => field.to_owned(),
            RenameRule::UpperCase | RenameRule::ScreamingSnakeCase => field.to_ascii_uppercase(),

            RenameRule::PascalCase => {
                let mut pascal = String::new();
                let mut capitalize = true;

                for ch in field.chars() {
                    if ch == '_' {
                        capitalize = true;
                    } else if capitalize {
                        pascal.push(ch.to_ascii_uppercase());
                        capitalize = false;
                    } else {
                        pascal.push(ch);
                    }
                }

                pascal
            }

            RenameRule::CamelCase => {
                let pascal = RenameRule::PascalCase.apply_to_field(field);

                pascal[..1].to_ascii_lowercase() + &pascal[1..]
            }

            RenameRule::KebabCase => field.replace('_', "-"),

            RenameRule::ScreamingKebabCase => RenameRule::ScreamingSnakeCase
                .apply_to_field(field)
                .replace('_', "-"),
        }
    }
}
