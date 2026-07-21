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

    #[error("provider ordering target '{target}' for component '{component}' is not registered")]
    MissingProviderOrderTarget { component: String, target: String },

    #[error("provider ordering component '{component}' cannot target itself")]
    SelfProviderOrder { component: String },

    #[error(
        "provider ordering component '{component}' does not provide restricted trait '{trait_name}'"
    )]
    ProviderOrderSourceTraitMismatch {
        component: String,
        trait_name: String,
    },

    #[error(
        "provider ordering target '{target}' does not provide trait '{trait_name}' required by '{component}'"
    )]
    ProviderOrderTargetTraitMismatch {
        component: String,
        target: String,
        trait_name: String,
    },

    #[error("provider ordering cycle for trait '{trait_name}': {components}")]
    ProviderOrderCycle {
        trait_name: String,
        components: String,
    },

    #[error("missing component: {0}")]
    MissingComponent(&'static str),

    #[error(
        "the root resolver is unavailable: the root container was never attached or has been dropped"
    )]
    RootUnavailable,

    #[error("the captured scope is unavailable: it was not attached or has been dropped")]
    ScopeUnavailable,

    #[error("fresh construction is unsupported for factory-less component '{0}'")]
    UnsupportedFreshFactory(String),

    #[error("invalid fresh dependency for component '{component}': {dependency}")]
    InvalidFreshDependency {
        component: String,
        dependency: String,
    },

    #[error(
        "component '{component}' cannot defer transient dependency '{dependency}': deferred targets must be stored in a scope"
    )]
    DeferredTransientDependency {
        component: String,
        dependency: String,
    },

    #[error(
        "duplicate provider qualifier '{qualifier}' for trait '{trait_name}' in scope '{scope}': qualifier selection is first-registered, so same-scope duplicates resolve nondeterministically"
    )]
    DuplicateProviderQualifier {
        trait_name: String,
        qualifier: String,
        scope: String,
    },

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
