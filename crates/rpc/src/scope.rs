//! RPC-protocol component scopes.
//!
//! The framework core knows only the universal anchors
//! [`Singleton`](overseerd_core::Singleton) and [`Transient`](overseerd_core::Transient).
//! The connection/request lifetimes are specific to a connection-oriented request
//! protocol, so they are defined here in the daemon (RPC) layer and slot in at ranks
//! between the two anchors.

use overseerd_core::StaticScope;

/// A per-connection scope: a live session between the daemon and one remote peer.
/// Outlives the requests multiplexed over it, so it ranks above [`Request`].
pub struct Connection;

/// A per-request scope: one inbound RPC call.
pub struct Request;

impl StaticScope for Connection {
    const RANK: u8 = 200;
    const NAME: &'static str = "Connection";
}

impl StaticScope for Request {
    const RANK: u8 = 100;
    const NAME: &'static str = "Request";
}
