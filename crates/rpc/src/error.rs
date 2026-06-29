use overseerd_transport::{PredefinedCode, StatusCode};
use thiserror::Error;

use crate::extract::{ErrorResponse, ResponseError};

/// Errors from the RPC protocol: service/RPC registration, request dispatch, and the
/// agnostic application core it builds on (via [`App`](Error::App)).
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

    /// An assembly failure from the protocol-agnostic application core (DI graph, config,
    /// hooks, scope planning). Boxed to keep this error small.
    #[error(transparent)]
    App(Box<overseerd_app::Error>),

    /// A configuration failure surfaced directly (e.g. the `app!` macro loading a config
    /// source); the same error also reaches here wrapped in [`App`](Error::App).
    #[error(transparent)]
    Config(#[from] overseerd_config::ConfigError),

    #[error("transport error: {0}")]
    Transport(#[from] overseerd_transport::Error),
}

impl From<overseerd_app::Error> for Error {
    fn from(error: overseerd_app::Error) -> Self {
        Error::App(Box::new(error))
    }
}

impl Error {
    /// Maps an error to the status code its response carries. Invalid-input errors become
    /// `BadInput`, missing routes become `NotFound`, everything else `Internal`.
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

/// The RPC-layer result type.
pub type Result<T, E = Error> = core::result::Result<T, E>;
