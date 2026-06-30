use thiserror::Error;

/// Errors from the protocol-agnostic application core: registry validation, the DI
/// engine, config, and hooks.
///
/// A protocol's own error type wraps this (typically via `#[from]`), so a protocol's
/// `build`/`serve` can absorb assembly failures while adding its own variants.
#[derive(Debug, Error)]
pub enum Error {
    #[error(
        "component '{component}' declares scope '{scope}', which the active protocol does not open"
    )]
    UndeclaredScope {
        component: String,
        scope: &'static str,
    },

    #[error(
        "missing config for component '{component}': no binding of type '{type_name}' \
         at path '{path}'"
    )]
    MissingConfig {
        component: String,
        type_name: String,
        path: String,
    },

    #[error(
        "ambiguous config for component '{component}': type '{type_name}' is bound at \
         {count} paths ({paths}); name one with `#[config(\"..\")]`"
    )]
    AmbiguousConfig {
        component: String,
        type_name: String,
        count: usize,
        paths: String,
    },

    /// A component-graph failure from the DI engine (cycle, missing dependency, ambiguous
    /// provider, scope violation, duplicate/ambiguous factory, …).
    #[error(transparent)]
    Di(#[from] overseerd_di::Error),

    /// A configuration loading, binding, or substitution failure.
    #[error(transparent)]
    Config(#[from] overseerd_config::ConfigError),

    /// A hook failure (e.g. an unresolvable receiver or parameter).
    #[error(transparent)]
    Hook(#[from] overseerd_hooks::Error),

    /// An application-defined error surfaced through the framework.
    #[error(transparent)]
    Other(#[from] Box<dyn std::error::Error + Send + Sync>),
}

/// The app-layer result type.
pub type Result<T, E = Error> = core::result::Result<T, E>;
