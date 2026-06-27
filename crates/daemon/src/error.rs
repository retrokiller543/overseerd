use crate::extract::{ErrorResponse, ResponseError};
use overseerd_transport::{PredefinedCode, StatusCode};
use thiserror::Error;

/// Errors emitted by the daemon runtime: registry validation, request dispatch, and the
/// lower layers it aggregates.
///
/// Component-graph failures (cycles, missing deps, scope violations, …) come from the DI
/// engine and arrive through [`Di`](Error::Di); config and hook failures arrive likewise.
/// The daemon owns the service/RPC and config-binding validation variants directly.
#[derive(Debug, Error)]
pub enum Error {
    #[error("duplicate service id: {0}")]
    DuplicateServiceId(String),

    #[error("duplicate RPC '{rpc}' in service '{service}'")]
    DuplicateRpcName { service: String, rpc: String },

    #[error("duplicate RPC path: {0}")]
    DuplicateRpcPath(String),

    #[error("service '{0}' has no RPC methods")]
    EmptyService(String),

    #[error("route not found: {0}")]
    RouteNotFound(String),

    #[error("invalid request payload: {0}")]
    InvalidPayload(String),

    #[error("handler expects a request stream but the call did not open one")]
    NotStreaming,

    #[error("response serialization failed: {0}")]
    Serialization(String),

    #[error("missing component: {0}")]
    MissingComponent(&'static str),

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

    #[error("transport error: {0}")]
    Transport(#[from] overseerd_transport::Error),

    /// An application-defined error surfaced through the framework.
    #[error(transparent)]
    Other(#[from] Box<dyn std::error::Error + Send + Sync>),
}

impl Error {
    /// Maps a framework error to the status code its response carries.
    ///
    /// Invalid-input errors become `BadInput`, missing routes become `NotFound`, and
    /// everything else falls through to `Internal`. Registry-validation variants only
    /// occur at build time, never on a call, so they map to `Internal` if ever surfaced.
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
