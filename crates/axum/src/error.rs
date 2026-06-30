//! The axum protocol's error type.

use thiserror::Error;

/// A failure raised while building or serving the axum protocol.
///
/// Absorbs the protocol-agnostic [`overseerd_app::Error`] (so it satisfies the
/// `ProtocolPlugin::Error: From<app::Error>` bound) and the I/O errors raised by binding
/// and serving the TCP listener.
#[derive(Debug, Error)]
pub enum Error {
    /// A failure originating in the agnostic app/runtime layer (scope build, hooks, …).
    #[error(transparent)]
    App(#[from] overseerd_app::Error),

    /// Binding the listener or serving the HTTP connection failed.
    #[error("axum serve error: {0}")]
    Serve(#[from] std::io::Error),
}

/// The axum protocol's [`Result`](std::result::Result) alias.
pub type Result<T> = std::result::Result<T, Error>;
