//! Type-based registration: a daemon assembled purely from `.service::<T>()`
//! (no `auto_discover()`), proving the `Descriptor<D>` connection registers a
//! service's header + factory and that `ServiceRpcs` pulls every `#[handlers]`
//! block keyed to the type — including a service split across two blocks.
//!
//! Gated off the `client` feature: the generated client emits one
//! `<Service>Client` per `#[handlers]` block, so a service split across two
//! blocks produces duplicate client definitions (a separate codegen limitation,
//! unrelated to registration).
#![cfg(not(feature = "client"))]

mod common;

use overseerd::CallResult;
use overseerd::daemon::{App, Payload, RpcAppBuilder, handlers, service};

use common::{MemoryServer, deadline};

/// A service whose RPCs are contributed by two separate `#[handlers]` blocks.
#[service(id = "typed_svc", version = "0.1")]
struct TypedSvc;

#[handlers]
impl TypedSvc {
    /// First block.
    #[rpc]
    async fn increment(Payload(n): Payload<u32>) -> u32 {
        n + 1
    }
}

#[handlers]
impl TypedSvc {
    /// Second block on the same type.
    #[rpc]
    async fn double(Payload(n): Payload<u32>) -> u32 {
        n * 2
    }
}

async fn start() -> MemoryServer {
    // No auto_discover(): the service, its factory, and both RPC groups come from
    // the type alone.
    let daemon = App::builder("test")
        .service::<TypedSvc>()
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
async fn service_by_type_registers_all_handler_blocks() {
    let server = start().await;
    let conn = server.connect().await;

    let first = deadline(
        "TypedSvc.increment call",
        conn.call("TypedSvc.increment", enc(&10u32)),
    )
    .await
    .expect("increment call succeeds");

    match first {
        CallResult::Ok(body) => assert_eq!(dec::<u32>(&body), 11),

        other => panic!("expected ok from first block, got {other:?}"),
    }

    let second = deadline(
        "TypedSvc.double call",
        conn.call("TypedSvc.double", enc(&10u32)),
    )
    .await
    .expect("double call succeeds");

    match second {
        CallResult::Ok(body) => assert_eq!(dec::<u32>(&body), 20),

        other => panic!("expected ok from second block, got {other:?}"),
    }

    server.shutdown([conn]).await;
}
