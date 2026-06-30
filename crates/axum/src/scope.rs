//! The axum protocol's component scope.
//!
//! HTTP is request-oriented: axum does not surface a connection lifecycle to handlers, so
//! the protocol opens a single [`Request`] scope per inbound request, slotting in between
//! the universal [`Singleton`](overseerd_core::Singleton) (root) and
//! [`Transient`](overseerd_core::Transient) anchors. Request-scoped components are built
//! lazily the first time a route injects them and dropped when the request ends.

use overseerd_core::StaticScope;

/// A per-request scope: one inbound HTTP request.
pub struct Request;

impl StaticScope for Request {
    const RANK: u8 = 100;
    const NAME: &'static str = "Request";
}
