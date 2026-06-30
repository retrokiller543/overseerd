//! End-to-end tests for scoped dependencies over the in-memory transport.
//!
//! Each scope's identity is a `#[default]` field seeded from an atomic counter, so
//! an instance's id is fixed for its lifetime: a connection-scoped instance keeps
//! one id across the calls on its connection, a request-scoped instance gets a
//! fresh id per call, and a transient gets a fresh id per resolution. The tests
//! assert exactly those relationships by reading the ids back through handlers.

use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

use overseerd::daemon::{App, Inject, handlers, service};
use overseerd::{CallResult, MemoryClient, MemoryConnectionHandle, PeerInfo, component};

static CONNECTION_IDS: AtomicU64 = AtomicU64::new(1);
static REQUEST_IDS: AtomicU64 = AtomicU64::new(1);
static TRACE_IDS: AtomicU64 = AtomicU64::new(1);

/// Connection-scoped: one id per connection.
struct ConnId(u64);

impl Default for ConnId {
    fn default() -> Self {
        Self(CONNECTION_IDS.fetch_add(1, Ordering::Relaxed))
    }
}

/// Request-scoped: one id per call.
struct ReqId(u64);

impl Default for ReqId {
    fn default() -> Self {
        Self(REQUEST_IDS.fetch_add(1, Ordering::Relaxed))
    }
}

/// Transient: one id per resolution.
struct TraceId(u64);

impl Default for TraceId {
    fn default() -> Self {
        Self(TRACE_IDS.fetch_add(1, Ordering::Relaxed))
    }
}

/// Connection-scoped component; depends on the framework-seeded peer.
#[component(scope = overseerd::daemon::Connection)]
struct ConnState {
    _peer: PeerInfo,
    #[default]
    id: ConnId,
}

/// Request-scoped component; depends on the connection-scoped one.
#[component(scope = overseerd::daemon::Request)]
struct ReqState {
    conn: Arc<ConnState>,
    #[default]
    id: ReqId,
}

/// Transient component, rebuilt on each resolution.
#[component(scope = overseerd::scope::Transient)]
struct Trace {
    #[default]
    id: TraceId,
}

#[service(id = "scopes", version = "0.1")]
struct ScopeSvc;

#[handlers]
impl ScopeSvc {
    /// Returns (connection id, request id) for this call.
    #[rpc]
    async fn ids(Inject(req): Inject<Arc<ReqState>>) -> overseerd::daemon::Result<(u64, u64)> {
        Ok((req.conn.id.0, req.id.0))
    }

    /// Returns the ids of two transients resolved in one call.
    #[rpc]
    async fn two_traces(
        Inject(a): Inject<Arc<Trace>>,
        Inject(b): Inject<Arc<Trace>>,
    ) -> overseerd::daemon::Result<(u64, u64)> {
        Ok((a.id.0, b.id.0))
    }
}

/// Builds the daemon, serves it on a memory transport, and returns the client so
/// the test can open several independent connections.
async fn start() -> MemoryClient {
    let (client, transport) = MemoryClient::pair();

    let daemon = App::builder("scopes-test")
        .auto_discover()
        .build()
        .await
        .expect("build daemon");

    tokio::spawn(async move {
        let _ = daemon.serve(transport).await;
    });

    client
}

fn enc<T: serde::Serialize>(value: &T) -> Vec<u8> {
    postcard::to_allocvec(value).unwrap()
}

fn dec<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> T {
    postcard::from_bytes(bytes).unwrap()
}

async fn ids(conn: &MemoryConnectionHandle) -> (u64, u64) {
    match conn.call("ScopeSvc.ids", enc(&())).await.unwrap() {
        CallResult::Ok(body) => dec::<(u64, u64)>(&body),
        CallResult::Err { .. } => panic!("ids call errored"),
    }
}

#[tokio::test]
async fn connection_scope_is_stable_within_a_connection() {
    let client = start().await;
    let conn = client.connect().await.expect("connect");

    let (c1, r1) = ids(&conn).await;
    let (c2, r2) = ids(&conn).await;

    assert_eq!(c1, c2, "connection-scoped id is stable across calls");
    assert_ne!(r1, r2, "request-scoped id is fresh per call");
}

#[tokio::test]
async fn connection_scope_differs_across_connections() {
    let client = start().await;

    let first = client.connect().await.expect("connect");
    let second = client.connect().await.expect("connect");

    let (c1, _) = ids(&first).await;
    let (c2, _) = ids(&second).await;

    assert_ne!(
        c1, c2,
        "each connection gets its own connection-scoped instance"
    );
}

#[tokio::test]
async fn transient_is_fresh_per_resolution() {
    let client = start().await;
    let conn = client.connect().await.expect("connect");

    let (a, b) = match conn.call("ScopeSvc.two_traces", enc(&())).await.unwrap() {
        CallResult::Ok(body) => dec::<(u64, u64)>(&body),
        CallResult::Err { .. } => panic!("two_traces call errored"),
    };

    assert_ne!(a, b, "two transient resolutions yield distinct instances");
}
