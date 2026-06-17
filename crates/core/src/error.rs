use thiserror::Error;

/// Errors emitted by the Overseer framework during registry validation and operation.
#[derive(Debug, Error)]
pub enum Error {
    #[error("duplicate component id: {0}")]
    DuplicateComponentId(String),

    #[error("multiple explicit constructors registered for component type: {0}")]
    DuplicateComponentType(String),

    #[error("#[handlers] impl for type '{0}' has no matching #[service] declaration")]
    OrphanRpcs(String),

    #[error("duplicate service id: {0}")]
    DuplicateServiceId(String),

    #[error("duplicate RPC '{rpc}' in service '{service}'")]
    DuplicateRpcName { service: String, rpc: String },

    #[error("duplicate RPC path: {0}")]
    DuplicateRpcPath(String),

    #[error("missing dependency for component '{component}': type '{type_name}'")]
    MissingDependency {
        component: String,
        type_name: String,
    },

    #[error("service '{0}' has no RPC methods")]
    EmptyService(String),

    #[error("dependency cycle detected in component graph")]
    DependencyCycle,

    #[error("route not found: {0}")]
    RouteNotFound(String),

    #[error("transport error: {0}")]
    Transport(#[from] overseer_transport::Error),

    #[error("invalid request payload: {0}")]
    InvalidPayload(String),

    #[error("handler expects a request stream but the call did not open one")]
    NotStreaming,

    #[error("missing connection extension: {0}")]
    MissingExtension(&'static str),

    #[error("response serialization failed: {0}")]
    Serialization(String),

    #[error("missing component: {0}")]
    MissingComponent(&'static str),
}
