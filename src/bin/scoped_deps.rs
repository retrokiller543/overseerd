//! Demonstrates connection-scoped dependencies *and* axum-style typed handlers.
//!
//! A `ConnectionHandler` runs once when a connection is accepted and stores
//! per-connection state on `ConnectionInfo` (here: an authenticated identity
//! plus a checked-out DB handle). Handlers then declare what they need as
//! typed parameters — `Payload<T>`, `Extension<T>`, `Conn` — instead of
//! receiving the raw `RpcCallContext`.

use std::{
    collections::HashMap,
    future::Future,
    net::IpAddr,
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use serde::{Deserialize, Serialize};

use overseer_core::{
    Conn, ConnectionHandler, ConnectionInfo, Daemon, Extension, Payload, handlers, service,
};
use overseer_transport::TcpTransport;

// ---------------------------------------------------------------------------
// Shared dependency: a pool the daemon owns once, shared across connections.
// ---------------------------------------------------------------------------

/// A database pool owned by the daemon. Cloning is cheap (it shares one inner
/// pool); each connection checks out its own handle.
#[derive(Clone)]
struct DbPool {
    inner: Arc<PoolInner>,
}

struct PoolInner {
    issued: AtomicU64,
}

impl DbPool {
    fn new() -> Self {
        Self {
            inner: Arc::new(PoolInner {
                issued: AtomicU64::new(0),
            }),
        }
    }

    /// Checks out a handle for the lifetime of one client connection.
    fn acquire(&self) -> DbConn {
        let conn_id = self.inner.issued.fetch_add(1, Ordering::Relaxed);

        DbConn { conn_id }
    }
}

/// A database handle scoped to a single client connection.
struct DbConn {
    conn_id: u64,
}

impl DbConn {
    async fn lookup_display_name(&self, user_id: &str) -> String {
        format!("'{user_id}' (served via db handle #{})", self.conn_id)
    }
}

// ---------------------------------------------------------------------------
// Connection-scoped state: built per connection, read by every handler.
// ---------------------------------------------------------------------------

/// Cheap, cloneable identity — suited to the `Extension<T>` extractor.
#[derive(Clone)]
struct Identity {
    user_id: String,
    api_key: String,
}

/// The checked-out DB handle. Not `Clone`, so handlers reach it through `Conn`
/// and `get::<Db>()` rather than `Extension<T>`.
struct Db {
    conn: DbConn,
}

// ---------------------------------------------------------------------------
// The connection handler: authenticates and attaches the scoped state.
// ---------------------------------------------------------------------------

/// Authenticates each new connection and attaches its scoped state.
struct Authenticator {
    pool: DbPool,
    directory: Arc<HashMap<IpAddr, (String, String)>>,
}

impl ConnectionHandler for Authenticator {
    fn on_connect<'a>(
        &'a self,
        info: &'a mut ConnectionInfo,
    ) -> Pin<Box<dyn Future<Output = overseer_core::Result<()>> + Send + 'a>> {
        Box::pin(async move {
            let peer_ip = info.peer().addr.map(|addr| addr.ip());

            let (user_id, api_key) = peer_ip
                .and_then(|ip| self.directory.get(&ip))
                .cloned()
                .unwrap_or_else(|| ("anonymous".to_string(), "none".to_string()));

            info.insert(Identity { user_id, api_key });
            info.insert(Db {
                conn: self.pool.acquire(),
            });

            Ok(())
        })
    }
}

// ---------------------------------------------------------------------------
// Handlers — typed parameters, no raw context.
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct GreetRequest {
    name: String,
}

#[derive(Serialize)]
struct GreetReply {
    message: String,
}

#[derive(Serialize)]
struct WhoAmI {
    display_name: String,
    api_key: String,
}

// ---------------------------------------------------------------------------
// Service — stateless: request state comes from the connection (`Conn`,
// `Extension<T>`), not a singleton, so it's a unit struct.
// ---------------------------------------------------------------------------

#[service(id = "account", version = "0.1")]
struct AccountService;

#[handlers]
impl AccountService {
    /// Body via `Payload`, identity via the cloned `Extension`.
    #[rpc]
    async fn greet(
        Payload(req): Payload<GreetRequest>,
        Extension(identity): Extension<Identity>,
    ) -> overseer_core::Result<GreetReply> {
        Ok(GreetReply {
            message: format!("Hello, {}! (signed for {})", req.name, identity.user_id),
        })
    }

    /// Reaches the non-clone DB handle through the full connection context.
    #[rpc]
    async fn whoami(conn: Conn) -> overseer_core::Result<WhoAmI> {
        let identity = conn
            .0
            .get::<Identity>()
            .expect("Authenticator inserts Identity on connect");
        let db = conn
            .0
            .get::<Db>()
            .expect("Authenticator inserts Db on connect");

        let display_name = db.conn.lookup_display_name(&identity.user_id).await;

        Ok(WhoAmI {
            display_name,
            api_key: identity.api_key.clone(),
        })
    }
}

#[tokio::main]
async fn main() -> overseer_core::Result<()> {
    let local: IpAddr = "127.0.0.1".parse().unwrap();
    let mut directory = HashMap::new();

    directory.insert(local, ("alice".to_string(), "sk-alice-123".to_string()));

    let daemon = Daemon::builder("account")
        .auto_discover()
        .connection_handler(Authenticator {
            pool: DbPool::new(),
            directory: Arc::new(directory),
        })
        .build()
        .await?;

    let transport = TcpTransport::bind("127.0.0.1:9100").await?;

    daemon.serve(transport).await
}
