//! End-to-end tests for the framework builtins, driven over the in-memory
//! transport. A handler injects the seeded [`ShutdownHandle`] and the call
//! completes, proving the builtin resolves through the request scope chain.

mod common;

use overseerd::daemon::{App, Inject, Payload, handlers, service};
use overseerd::{CallResult, ShutdownHandle};

use common::{MemoryServer, deadline};

/// A service whose handler injects the framework-seeded shutdown handle.
#[service(id = "builtins_svc", version = "0.1")]
struct BuiltinsSvc;

#[handlers]
impl BuiltinsSvc {
    /// Resolves the seeded [`ShutdownHandle`] from the call scope and echoes back a
    /// marker, proving the builtin is injectable from inside a handler.
    #[rpc]
    async fn ping(Inject(_shutdown): Inject<ShutdownHandle>, Payload(n): Payload<u32>) -> u32 {
        n + 1
    }
}

async fn start() -> MemoryServer {
    let daemon = App::builder("builtins-test")
        .auto_discover()
        .build()
        .await
        .expect("build daemon");

    MemoryServer::start(daemon)
}

fn enc<T: serde::Serialize>(value: &T) -> Vec<u8> {
    postcard::to_allocvec(value).unwrap()
}

fn dec<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> T {
    postcard::from_bytes(bytes).unwrap()
}

#[tokio::test]
async fn handler_can_inject_shutdown_handle() {
    let server = start().await;
    let conn = server.connect().await;

    let result = deadline(
        "BuiltinsSvc.ping call",
        conn.call("BuiltinsSvc.ping", enc(&41u32)),
    )
    .await
    .expect("call succeeds");

    match result {
        CallResult::Ok(bytes) => {
            let value: u32 = dec(&bytes);

            assert_eq!(value, 42);
        }

        other => panic!("expected an ok response, got {other:?}"),
    }

    server.shutdown([conn]).await;
}
