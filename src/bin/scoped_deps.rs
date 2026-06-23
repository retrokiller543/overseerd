//! Demonstrates **scoped dependencies** wired through the DI system.
//!
//! Every scope here is a first-class component the container builds and injects —
//! no hand-rolled `on_connect` hook:
//! - `Stats` is a **singleton**, shared by every connection and call.
//! - `Session` is **connection-scoped**: one per connection, built when the
//!   connection is accepted. It depends on the framework-seeded `Arc<PeerInfo>`,
//!   so it sees the remote peer — the DI-native replacement for the old
//!   connection handler.
//! - `RequestCtx` is **request-scoped**: one per RPC call. It depends on the
//!   connection-scoped `Session` (a shorter-lived scope depending on a
//!   longer-lived one, which the captive-dependency rule permits).
//! - `Tracer` is **transient**: rebuilt on every resolution, so two `Inject`s in
//!   one handler get two distinct instances.
//!
//! Handlers reach scoped components through the `Inject<H>` extractor; the
//! per-scope identity is a `#[default]` field seeded from an atomic counter, so a
//! connection keeps one id across its calls while each call gets a fresh one.

use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

use overseer::{Daemon, Inject, PeerInfo, Result, component, handlers, service};
use overseer::transport::TcpTransport;
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Per-scope identities: each `default()` pulls the next value from a counter, so
// an instance's id is fixed for its lifetime and unique among its scope's peers.
// ---------------------------------------------------------------------------

static CONNECTION_IDS: AtomicU64 = AtomicU64::new(1);
static REQUEST_IDS: AtomicU64 = AtomicU64::new(1);
static TRACE_IDS: AtomicU64 = AtomicU64::new(1);

/// A connection's unique id, allocated once when its `Session` is built.
struct ConnectionId(u64);

impl Default for ConnectionId {
    fn default() -> Self {
        Self(CONNECTION_IDS.fetch_add(1, Ordering::Relaxed))
    }
}

/// A call's unique id, allocated once when its `RequestCtx` is built.
struct RequestId(u64);

impl Default for RequestId {
    fn default() -> Self {
        Self(REQUEST_IDS.fetch_add(1, Ordering::Relaxed))
    }
}

/// A transient id, allocated afresh on every resolution.
struct TraceId(u64);

impl Default for TraceId {
    fn default() -> Self {
        Self(TRACE_IDS.fetch_add(1, Ordering::Relaxed))
    }
}

// ---------------------------------------------------------------------------
// Singleton: process-wide call counter, shared across every connection.
// ---------------------------------------------------------------------------

/// Process-wide statistics, owned for the daemon's lifetime.
#[component]
struct Stats {
    #[default]
    calls: AtomicU64,
}

impl Stats {
    /// Records a call and returns the running total.
    fn record(&self) -> u64 {
        self.calls.fetch_add(1, Ordering::Relaxed) + 1
    }
}

// ---------------------------------------------------------------------------
// Connection scope: one Session per connection, aware of the remote peer.
// ---------------------------------------------------------------------------

/// Per-connection state, built once per accepted connection. Depends on the
/// framework-seeded peer (a by-value, connection-scoped injectable provided by
/// every daemon) to label the connection.
#[component(scope = connection)]
struct Session {
    peer: PeerInfo,
    #[default]
    id: ConnectionId,
}

impl Session {
    fn label(&self) -> String {
        match self.peer.addr {
            Some(addr) => format!("connection #{} from {addr}", self.id.0),
            None => format!("connection #{} (local)", self.id.0),
        }
    }
}

// ---------------------------------------------------------------------------
// Request scope: one RequestCtx per call, carrying its connection's Session.
// ---------------------------------------------------------------------------

/// Per-call state. Depends on the connection-scoped `Session`, so each call sees
/// the connection it belongs to plus its own request id.
#[component(scope = request)]
struct RequestCtx {
    session: Arc<Session>,
    #[default]
    id: RequestId,
}

// ---------------------------------------------------------------------------
// Transient: a fresh Tracer on every resolution.
// ---------------------------------------------------------------------------

/// A throwaway tracer, rebuilt on each resolution rather than cached.
#[component(scope = transient)]
struct Tracer {
    #[default]
    id: TraceId,
}

// ---------------------------------------------------------------------------
// Service — stateless: everything it needs is injected per call.
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize)]
struct WhoAmI {
    connection: String,
    request_id: u64,
    total_calls: u64,
}

#[service(id = "scoped", version = "0.1")]
struct ScopedDemo;

#[handlers]
impl ScopedDemo {
    /// Reports the calling connection (stable across its calls), this call's
    /// request id (fresh each call), and the process-wide call total.
    #[rpc]
    async fn whoami(
        Inject(stats): Inject<Arc<Stats>>,
        Inject(ctx): Inject<Arc<RequestCtx>>,
    ) -> Result<WhoAmI> {
        Ok(WhoAmI {
            connection: ctx.session.label(),
            request_id: ctx.id.0,
            total_calls: stats.record(),
        })
    }

    /// Resolves two transients in one call; their ids differ, showing each
    /// resolution builds a fresh instance.
    #[rpc]
    async fn two_tracers(
        Inject(first): Inject<Arc<Tracer>>,
        Inject(second): Inject<Arc<Tracer>>,
    ) -> Result<(u64, u64)> {
        Ok((first.id.0, second.id.0))
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let daemon = Daemon::builder("scoped").auto_discover().build().await?;

    println!("{daemon}");

    let transport = TcpTransport::bind("127.0.0.1:9100").await?;

    daemon.serve(transport).await
}
