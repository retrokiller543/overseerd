//! The axum protocol's component scopes.
//!
//! HTTP is request-oriented: axum does not surface a connection lifecycle to plain HTTP
//! handlers, so an HTTP request opens a single [`Request`] scope parented directly at the
//! [`Singleton`](overseerd_core::Singleton) root. A **WebSocket** connection, by contrast, is
//! long-lived and multiplexes many messages, so it opens a [`Connection`] scope once per upgraded
//! socket and a fresh [`Request`] scope per inbound message parented at it — mirroring the RPC
//! connection/request chain. `Connection` therefore only matters for WebSocket controllers; a plain
//! HTTP app never opens it. Both slot between the universal `Singleton` (root) and
//! [`Transient`](overseerd_core::Transient) anchors.

use overseerd_core::StaticScope;

/// A per-connection scope: one live WebSocket connection. Outlives the messages multiplexed over
/// it, so it ranks above [`Request`]. Only opened for WebSocket controllers — a plain HTTP request
/// parents its [`Request`] scope at the singleton root directly.
pub struct Connection;

/// A per-request scope: one inbound HTTP request, or one inbound WebSocket message.
pub struct Request;

impl StaticScope for Connection {
    const RANK: u8 = 200;
    const NAME: &'static str = "Connection";
}

impl StaticScope for Request {
    const RANK: u8 = 100;
    const NAME: &'static str = "Request";
}
