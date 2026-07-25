//! The axum protocol's component scopes.
//!
//! Plain HTTP and WebSocket traffic are separate topology branches. Each HTTP request opens an
//! [`HttpRequest`] scope directly below the application root. With WebSocket support enabled, each
//! upgraded socket opens one [`WebsocketConnection`] scope below the root and each inbound
//! application message opens a [`WebsocketMessage`] child below that connection.

use overseerd_app::{ScopeBoundary, ScopeParent, ScopeTopology};
use overseerd_core::{ScopeId, StaticScope};

/// Stable identity of the HTTP request scope.
pub const HTTP_REQUEST_SCOPE_ID: ScopeId =
    overseerd_core::namespaced_id!(ScopeId, "overseerd/axum-http-request");

/// Stable identity of the WebSocket connection scope.
#[cfg(feature = "ws")]
pub const WEBSOCKET_CONNECTION_SCOPE_ID: ScopeId =
    overseerd_core::namespaced_id!(ScopeId, "overseerd/axum-websocket-connection");

/// Stable identity of the WebSocket message scope.
#[cfg(feature = "ws")]
pub const WEBSOCKET_MESSAGE_SCOPE_ID: ScopeId =
    overseerd_core::namespaced_id!(ScopeId, "overseerd/axum-websocket-message");

/// One inbound HTTP request.
pub struct HttpRequest;

/// One live upgraded WebSocket connection.
#[cfg(feature = "ws")]
pub struct WebsocketConnection;

/// One inbound application message on a WebSocket connection.
#[cfg(feature = "ws")]
pub struct WebsocketMessage;

impl StaticScope for HttpRequest {
    const ID: ScopeId = HTTP_REQUEST_SCOPE_ID;
    const RANK: u8 = 100;
    const NAME: &'static str = "HttpRequest";
}

#[cfg(feature = "ws")]
impl StaticScope for WebsocketConnection {
    const ID: ScopeId = WEBSOCKET_CONNECTION_SCOPE_ID;
    const RANK: u8 = 200;
    const NAME: &'static str = "WebsocketConnection";
}

#[cfg(feature = "ws")]
impl StaticScope for WebsocketMessage {
    const ID: ScopeId = WEBSOCKET_MESSAGE_SCOPE_ID;
    const RANK: u8 = 100;
    const NAME: &'static str = "WebsocketMessage";
}

#[cfg(not(feature = "ws"))]
static AXUM_SCOPE_BOUNDARIES: [ScopeBoundary; 1] =
    [ScopeBoundary::new(&HttpRequest, ScopeParent::Root)];

#[cfg(feature = "ws")]
static AXUM_SCOPE_BOUNDARIES: [ScopeBoundary; 3] = [
    ScopeBoundary::new(&HttpRequest, ScopeParent::Root),
    ScopeBoundary::new(&WebsocketConnection, ScopeParent::Root),
    ScopeBoundary::new(
        &WebsocketMessage,
        ScopeParent::Boundary(WEBSOCKET_CONNECTION_SCOPE_ID),
    ),
];

/// Axum-owned scope topology for HTTP and optional WebSocket traffic.
pub const SCOPE_TOPOLOGY: ScopeTopology = ScopeTopology::new(&AXUM_SCOPE_BOUNDARIES);

#[cfg(test)]
mod tests;
