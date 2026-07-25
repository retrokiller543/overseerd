//! RPC-protocol component scopes.
//!
//! The framework core knows only the universal anchors
//! [`Singleton`](overseerd_core::Singleton) and [`Transient`](overseerd_core::Transient).
//! The connection/request lifetimes are specific to a connection-oriented request
//! protocol, so they are defined here in the daemon (RPC) layer with stable identities
//! and an explicit root-to-connection-to-request topology.

use overseerd_app::{ScopeBoundary, ScopeParent, ScopeTopology};
use overseerd_core::{ScopeId, StaticScope};

/// A per-connection scope: a live session between the daemon and one remote peer.
/// Outlives the requests multiplexed over it, so it ranks above [`Request`].
pub struct Connection;

/// A per-request scope: one inbound RPC call.
pub struct Request;

impl StaticScope for Connection {
    const ID: ScopeId = overseerd_core::namespaced_id!(ScopeId, "overseerd/rpc-connection");
    const RANK: u8 = 200;
    const NAME: &'static str = "Connection";
}

impl StaticScope for Request {
    const ID: ScopeId = overseerd_core::namespaced_id!(ScopeId, "overseerd/rpc-request");
    const RANK: u8 = 100;
    const NAME: &'static str = "Request";
}

static RPC_SCOPE_BOUNDARIES: [ScopeBoundary; 2] = [
    ScopeBoundary::new(&Connection, ScopeParent::Root),
    ScopeBoundary::new(&Request, ScopeParent::of::<Connection>()),
];

/// RPC-owned scope topology: `root -> connection -> request`.
pub const SCOPE_TOPOLOGY: ScopeTopology = ScopeTopology::new(&RPC_SCOPE_BOUNDARIES);
