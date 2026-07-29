use overseerd_tooling_schema::ValidationError;
use thiserror::Error;

/// The selector namespace in which exact resolution failed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum GraphSelectorKind {
    /// Any resource.
    Resource,
    /// A resource kind capable of owning contributions.
    Contributor,
    /// A plugin resource.
    Plugin,
}

impl std::fmt::Display for GraphSelectorKind {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let name = match self {
            Self::Resource => "resource",
            Self::Contributor => "contributor",
            Self::Plugin => "plugin",
        };

        formatter.write_str(name)
    }
}

/// A typed graph selection or source-document failure.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum GraphQueryError {
    /// The cloned and canonicalized source tooling document is invalid.
    #[error("source tooling document is invalid: {source}")]
    InvalidDocument {
        /// Structural source validation failure.
        #[source]
        source: Box<ValidationError>,
    },
    /// No eligible resource has the exact selector ID or name.
    #[error("{kind} selector '{selector}' was not found")]
    NotFound {
        /// Selector namespace.
        kind: GraphSelectorKind,
        /// Exact unmodified selector.
        selector: String,
    },
    /// More than one eligible resource has the exact selector name.
    #[error("{kind} selector '{selector}' is ambiguous")]
    Ambiguous {
        /// Selector namespace.
        kind: GraphSelectorKind,
        /// Exact unmodified selector.
        selector: String,
        /// Matching stable resource IDs in sorted order.
        candidates: Vec<String>,
    },
}

/// A typed canonical graph JSON emission failure.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum GraphEmitError {
    /// JSON serialization failed.
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}
