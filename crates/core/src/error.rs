use thiserror::Error;

/// Errors emitted by the Overseer framework during registry validation and operation.
#[derive(Debug, Error)]
pub enum Error {
    #[error("duplicate component id: {0}")]
    DuplicateComponentId(String),

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
}
