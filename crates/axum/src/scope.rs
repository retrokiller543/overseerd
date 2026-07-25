//! The axum protocol's component scopes.
//!
//! Plain HTTP and WebSocket traffic are separate topology branches. Each HTTP request opens an
//! [`HttpRequest`] scope directly below the application root. With WebSocket support enabled, each
//! upgraded socket opens one [`WebsocketConnection`] scope below the root and each inbound
//! application message opens a [`WebsocketMessage`] child below that connection.

use overseerd_app::{ScopeBoundary, ScopeParent, ScopeTopology};
use overseerd_core::{ScopeId, StaticScope};

/// One inbound HTTP request.
pub struct HttpRequest;

/// One live upgraded WebSocket connection.
#[cfg(feature = "ws")]
pub struct WebsocketConnection;

/// One inbound application message on a WebSocket connection.
#[cfg(feature = "ws")]
pub struct WebsocketMessage;

impl StaticScope for HttpRequest {
    const ID: ScopeId = overseerd_core::namespaced_id!(ScopeId, "overseerd/axum-http-request");
    const RANK: u8 = 100;
    const NAME: &'static str = "HttpRequest";
}

#[cfg(feature = "ws")]
impl StaticScope for WebsocketConnection {
    const ID: ScopeId =
        overseerd_core::namespaced_id!(ScopeId, "overseerd/axum-websocket-connection");
    const RANK: u8 = 200;
    const NAME: &'static str = "WebsocketConnection";
}

#[cfg(feature = "ws")]
impl StaticScope for WebsocketMessage {
    const ID: ScopeId = overseerd_core::namespaced_id!(ScopeId, "overseerd/axum-websocket-message");
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
    ScopeBoundary::new(&WebsocketMessage, ScopeParent::of::<WebsocketConnection>()),
];

/// Axum-owned scope topology for HTTP and optional WebSocket traffic.
pub const SCOPE_TOPOLOGY: ScopeTopology = ScopeTopology::new(&AXUM_SCOPE_BOUNDARIES);

#[cfg(test)]
mod tests;
