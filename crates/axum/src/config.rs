//! Configuration owned and automatically bound by the axum protocol plugin.

use std::net::{IpAddr, SocketAddr};

use overseerd_config::{ConfigProperties, DefaultSpec};
use serde::Deserialize;

/// The property path the [`AxumPlugin`](crate::AxumPlugin) always binds.
pub const AXUM_CONFIG_PATH: &str = "axum";

/// Listener settings for the axum HTTP server.
///
/// The plugin always binds this type at [`AXUM_CONFIG_PATH`], even when application config
/// auto-discovery is disabled. All fields have environment-aware defaults, so an axum app can be
/// built and served without declaring configuration of its own:
///
/// ```toml
/// [axum]
/// bind = "0.0.0.0"
/// port = 8080
/// ```
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct AxumConfig {
    /// The IP address on which the HTTP listener accepts connections.
    pub bind: IpAddr,

    /// The TCP port on which the HTTP listener accepts connections.
    pub port: u16,

    /// Maximum request body accepted by Axum's buffered body extractors, in bytes.
    pub max_request_body_bytes: usize,

    /// Maximum time allowed for routing, middleware, and handler execution, in milliseconds.
    /// A value of `0` disables the request deadline.
    pub request_timeout_ms: u64,

    /// Time allowed for in-flight requests to finish after shutdown starts, in milliseconds.
    /// Once elapsed, remaining connections are forcibly dropped. A value of `0` waits forever.
    pub graceful_shutdown_timeout_ms: u64,
}

impl AxumConfig {
    /// The configured listener address.
    pub fn socket_addr(&self) -> SocketAddr {
        SocketAddr::new(self.bind, self.port)
    }
}

impl Default for AxumConfig {
    fn default() -> Self {
        Self {
            bind: IpAddr::from([127, 0, 0, 1]),
            port: 3000,
            max_request_body_bytes: 2 * 1024 * 1024,
            request_timeout_ms: 30_000,
            graceful_shutdown_timeout_ms: 30_000,
        }
    }
}

impl ConfigProperties for AxumConfig {
    const NAME: &'static str = "AxumConfig";
    const DEFAULTS: DefaultSpec = DefaultSpec::Fields(&[
        ("bind", "${AXUM_BIND:127.0.0.1}"),
        ("port", "${AXUM_PORT:3000}"),
        (
            "max_request_body_bytes",
            "${AXUM_MAX_REQUEST_BODY_BYTES:2097152}",
        ),
        ("request_timeout_ms", "${AXUM_REQUEST_TIMEOUT_MS:30000}"),
        (
            "graceful_shutdown_timeout_ms",
            "${AXUM_GRACEFUL_SHUTDOWN_TIMEOUT_MS:30000}",
        ),
    ]);
}

#[cfg(test)]
mod tests;
