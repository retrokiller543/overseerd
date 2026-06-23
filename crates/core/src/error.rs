use crate::{ErrorResponse, ResponseError};
use overseer_transport::{PredefinedCode, StatusCode};
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

    #[error(
        "ambiguous provider for '{0}': multiple components provide it; mark one `#[primary]`, \
         or inject `Vec`/`HashMap<String, _>` to receive all of them"
    )]
    AmbiguousProvider(String),

    #[error("service '{0}' has no RPC methods")]
    EmptyService(String),

    #[error("dependency cycle: no construction order for components: {0}")]
    DependencyCycle(String),

    #[error("route not found: {0}")]
    RouteNotFound(String),

    #[error("transport error: {0}")]
    Transport(#[from] overseer_transport::Error),

    #[error("invalid request payload: {0}")]
    InvalidPayload(String),

    #[error("handler expects a request stream but the call did not open one")]
    NotStreaming,

    #[error("response serialization failed: {0}")]
    Serialization(String),

    #[error("missing component: {0}")]
    MissingComponent(&'static str),

    #[error(
        "scope violation: component '{component}' ({component_scope:?}) depends on \
         '{dependency}' ({dependency_scope:?}), which is shorter-lived"
    )]
    ScopeViolation {
        component: String,
        dependency: String,
        component_scope: crate::ComponentScope,
        dependency_scope: crate::ComponentScope,
    },
}

impl Error {
    /// Maps a framework error to the status code its response carries.
    ///
    /// This is the framework's own category mapping, used by its
    /// `ResponseError` impl: invalid-input errors become `BadInput`, missing
    /// routes become `NotFound`, and everything else falls through to `Internal`.
    /// Registry-validation variants only occur at build time, never on a call, so
    /// they map to `Internal` if ever surfaced.
    pub fn status_code(&self) -> StatusCode {
        let predefined = match self {
            Error::InvalidPayload(_) | Error::NotStreaming => PredefinedCode::BadInput,
            Error::RouteNotFound(_) => PredefinedCode::NotFound,
            _ => PredefinedCode::Internal,
        };

        StatusCode::from(predefined)
    }
}

impl ResponseError for Error {
    type Body = String;
    
    fn status_code(&self) -> StatusCode {
        self.status_code()
    }

    fn error_response(self) -> ErrorResponse {
        ErrorResponse::with_serialized_body(self.status_code(), &self.to_string())
    }
}
