use thiserror::Error;

/// An error from config parsing, placeholder resolution, or typed deserialization.
///
/// Most failures carry the dotted path of the offending node (`At`) so a message
/// reads like `at 'server.port': cannot parse resolved value as u16`. Failures that occur
/// before a node path is known (template parsing) surface the bare [`TemplateErrorKind`].
#[derive(Debug, Error)]
pub enum TemplateError {
    #[error("at '{path}': {kind}")]
    At {
        path: String,
        #[source]
        kind: TemplateErrorKind,
    },

    #[error(transparent)]
    Bare(#[from] TemplateErrorKind),
}

impl TemplateError {
    /// Wraps a kind with the dotted node path it occurred at.
    pub fn at(path: impl Into<String>, kind: TemplateErrorKind) -> Self {
        TemplateError::At {
            path: path.into(),
            kind,
        }
    }
}

impl serde::de::Error for TemplateError {
    fn custom<T: std::fmt::Display>(msg: T) -> Self {
        TemplateError::Bare(TemplateErrorKind::Message(msg.to_string()))
    }
}

/// The specific failure, kept separate from path context so the same kind can be
/// reported with or without a node path.
#[derive(Debug, Error)]
pub enum TemplateErrorKind {
    #[error("unterminated placeholder: expected '}}' to close '${{'")]
    UnterminatedPlaceholder,

    #[error("no value for placeholder '{key}' (no env var, config path, or default)")]
    MissingPlaceholder { key: String },

    #[error(
        "no resolver registered for namespace placeholder '{key}' (and no default was given) — \
         is the namespace wired up? (for `${{@dir}}`, build the config via \
         `ConfigManager::load_from`/`with_directories`)"
    )]
    UnknownNamespaceKey { key: String },

    #[error("placeholder '{key}' references a value that is not a string")]
    NotStringRenderable { key: String },

    #[error("resolution cycle: {} -> {key}", .chain.join(" -> "))]
    ResolutionCycle { chain: Vec<String>, key: String },

    #[error(
        "placeholder resolution exceeded the maximum depth of {limit} (chain too long or deeply nested)"
    )]
    ResolutionDepthExceeded { limit: usize },

    #[error(
        "placeholder resolution for one configuration deserialization exceeded the aggregate work budget of {limit} steps"
    )]
    ResolutionBudgetExceeded { limit: usize },

    #[error(
        "aggregate rendered template output for one configuration deserialization exceeds {limit} bytes"
    )]
    RenderedOutputTooLarge { limit: usize },

    #[error("a templated placeholder can only produce a string, but a {target} was expected here")]
    PartialInNonString { target: &'static str },

    // Placeholder results commonly contain credentials. Never retain the rejected
    // value in an error: reload errors are logged by default.
    #[error("cannot parse resolved value as {target}")]
    ParseAs { target: &'static str },

    #[error("resolved value is out of range for {target}")]
    OutOfRange { target: &'static str },

    #[error("expected {expected}, found {found}")]
    TypeMismatch {
        expected: &'static str,
        found: &'static str,
    },

    #[error("{0}")]
    Message(String),
}
