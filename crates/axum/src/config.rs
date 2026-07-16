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

    /// A global path prefix every route is served under. Empty (or `"/"`) mounts routes at the
    /// root; `"/api"` mounts the whole application (controllers and any OpenAPI routes) under
    /// `/api`. Surfaced in the OpenAPI document as a `server` URL so documented paths stay relative.
    pub base_path: String,

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
            base_path: String::new(),
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
        ("base_path", "${AXUM_BASE_PATH:}"),
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

/// The property path the [`AxumPlugin`](crate::AxumPlugin) binds the OpenAPI settings at, under the
/// `openapi` feature. A subtree of `[axum]`, bound separately so its own field defaults apply.
#[cfg(feature = "openapi")]
pub const AXUM_OPENAPI_CONFIG_PATH: &str = "axum.openapi";

/// Which bundled OpenAPI UI the server mounts. Each non-`None` choice also requires its crate
/// feature (`openapi-swagger-ui`, `openapi-redoc`, `openapi-rapidoc`, `openapi-scalar`) to be
/// compiled; a choice whose feature is absent falls back to serving JSON only (with a warning).
#[cfg(feature = "openapi")]
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OpenApiUi {
    /// Serve only `/openapi.json`, no UI.
    None,

    /// Swagger UI (`utoipa-swagger-ui`).
    Swagger,

    /// Redoc (`utoipa-redoc`).
    Redoc,

    /// RapiDoc (`utoipa-rapidoc`).
    Rapidoc,

    /// Scalar (`utoipa-scalar`).
    Scalar,
}

/// OpenAPI generation and serving settings, bound at [`AXUM_OPENAPI_CONFIG_PATH`].
///
/// Disabled by default: a build with the `openapi` feature still serves nothing until
/// `enabled = true`, so turning the spec on is an explicit deployment choice.
///
/// ```toml
/// [axum.openapi]
/// enabled = true
/// ui = "swagger"
/// ```
#[cfg(feature = "openapi")]
#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize)]
pub struct OpenApiConfig {
    /// Whether to assemble the document and mount the routes at all.
    pub enabled: bool,

    /// The path the JSON document is served at.
    pub json_path: String,

    /// Which bundled UI to mount (or [`None`](OpenApiUi::None) for JSON only).
    pub ui: OpenApiUi,

    /// The path the UI is mounted at (ignored when `ui = none`).
    pub ui_path: String,

    /// The document title (OpenAPI `info.title`).
    pub title: String,

    /// The document version (OpenAPI `info.version`).
    pub version: String,
}

#[cfg(feature = "openapi")]
impl Default for OpenApiConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            json_path: String::from("/openapi.json"),
            ui: OpenApiUi::None,
            ui_path: String::from("/docs"),
            title: String::from("API"),
            version: String::from("0.0.0"),
        }
    }
}

#[cfg(feature = "openapi")]
impl ConfigProperties for OpenApiConfig {
    const NAME: &'static str = "OpenApiConfig";
    const DEFAULTS: DefaultSpec = DefaultSpec::Fields(&[
        ("enabled", "${AXUM_OPENAPI_ENABLED:false}"),
        ("json_path", "${AXUM_OPENAPI_JSON_PATH:/openapi.json}"),
        ("ui", "${AXUM_OPENAPI_UI:none}"),
        ("ui_path", "${AXUM_OPENAPI_UI_PATH:/docs}"),
        ("title", "${AXUM_OPENAPI_TITLE:API}"),
        ("version", "${AXUM_OPENAPI_VERSION:0.0.0}"),
    ]);
}

#[cfg(test)]
mod tests;
