use thiserror::Error;

/// Errors from the DI engine: graph validation and component construction.
#[derive(Debug, Error)]
pub enum Error {
    #[error("duplicate component id: {0}")]
    DuplicateComponentId(String),

    #[error("multiple explicit constructors registered for component type: {0}")]
    DuplicateComponentType(String),

    #[error(
        "ambiguous factory for component '{0}': more than one explicit factory (e.g. an #[init] and a factory = ..)"
    )]
    AmbiguousFactory(String),

    #[error("missing dependency for component '{component}': type '{type_name}'")]
    MissingDependency {
        component: String,
        type_name: String,
    },

    #[error(
        "ambiguous provider for '{0}': multiple components provide it; mark one `#[primary]`, \
         or inject `Vec`/`HashMap<String, _>` to receive all of them"
    )]
    AmbiguousProvider(String),

    #[error("dependency cycle: no construction order for components: {0}")]
    DependencyCycle(String),

    #[error("missing component: {0}")]
    MissingComponent(&'static str),

    #[error(
        "scope violation: component '{component}' ({component_scope}) depends on \
         '{dependency}' ({dependency_scope}), which is shorter-lived"
    )]
    ScopeViolation {
        component: String,
        dependency: String,
        component_scope: &'static str,
        dependency_scope: &'static str,
    },

    /// An application-defined error surfaced through the DI engine — typically from a
    /// component's `#[init]` constructor or a custom factory. The `#[from]` lets app
    /// authors use `?` with any `Error + Send + Sync` source.
    #[error(transparent)]
    Other(#[from] Box<dyn std::error::Error + Send + Sync>),
}

pub type Result<T, E = Error> = core::result::Result<T, E>;
