use thiserror::Error;

/// An error from config parsing, placeholder resolution, or typed deserialization.
///
/// Most failures carry the dotted path of the offending node (`At`) so a message
/// reads like `at 'server.port': cannot parse "abc" as u16`. Failures that occur
/// before a node path is known (template parsing) surface the bare [`ConfigErrorKind`].
#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("at '{path}': {kind}")]
    At {
        path: String,
        #[source]
        kind: ConfigErrorKind,
    },

    #[error(transparent)]
    Bare(#[from] ConfigErrorKind),
}

impl ConfigError {
    /// Wraps a kind with the dotted node path it occurred at.
    pub fn at(path: impl Into<String>, kind: ConfigErrorKind) -> Self {
        ConfigError::At {
            path: path.into(),
            kind,
        }
    }
}

impl serde::de::Error for ConfigError {
    fn custom<T: std::fmt::Display>(msg: T) -> Self {
        ConfigError::Bare(ConfigErrorKind::Message(msg.to_string()))
    }
}

/// The specific failure, kept separate from path context so the same kind can be
/// reported with or without a node path.
#[derive(Debug, Error)]
pub enum ConfigErrorKind {
    #[error("unterminated placeholder: expected '}}' to close '${{'")]
    UnterminatedPlaceholder,

    #[error("no value for placeholder '{key}' (no env var, config path, or default)")]
    MissingPlaceholder { key: String },

    #[error("no resolver answered namespace placeholder '{key}' (and no default was given)")]
    UnknownNamespaceKey { key: String },

    #[error("placeholder '{key}' references a value that is not a string")]
    NotStringRenderable { key: String },

    #[error("resolution cycle: {} -> {key}", .chain.join(" -> "))]
    ResolutionCycle { chain: Vec<String>, key: String },

    #[error(
        "placeholder resolution exceeded the maximum depth of {limit} (chain too long or deeply nested)"
    )]
    ResolutionDepthExceeded { limit: usize },

    #[error("a templated placeholder can only produce a string, but a {target} was expected here")]
    PartialInNonString { target: &'static str },

    #[error("cannot parse {value:?} as {target}")]
    ParseAs { target: &'static str, value: String },

    #[error("value {value} is out of range for {target}")]
    OutOfRange { target: &'static str, value: i128 },

    #[error("expected {expected}, found {found}")]
    TypeMismatch {
        expected: &'static str,
        found: &'static str,
    },

    #[error("{0}")]
    Message(String),
}
