//! Type-based registration: a daemon assembled purely from `.service::<T>()`
//! (no `auto_discover()`), proving the `Descriptor<D>` connection registers a
//! service's header + factory and that `ServiceRpcs` pulls its `#[handlers]`
//! blocks keyed to the type. Without client codegen, this also covers a service
//! split across two blocks.
//!
//! Client codegen currently emits one `<Service>Client` per `#[handlers]` block,
//! so the second block remains excluded under `client` until that separate
//! codegen limitation is removed.
#![cfg(feature = "daemon")]

mod common;

use upwell::CallResult;
use upwell::daemon::{App, Payload, RpcAppBuilder, handlers, service};

use common::{MemoryServer, deadline};

/// A service whose RPCs are contributed by two separate `#[handlers]` blocks.
#[cfg(not(feature = "client"))]
#[service(id = "typed_svc", version = "0.1")]
struct TypedSvc;

#[cfg(not(feature = "client"))]
#[handlers]
impl TypedSvc {
    /// First block.
    #[rpc]
    async fn increment(Payload(n): Payload<u32>) -> u32 {
        n + 1
    }
}

/// A single-block service proving type registration with generated clients enabled.
#[cfg(feature = "client")]
#[service(id = "client_typed_svc", version = "0.1")]
struct ClientTypedSvc;

#[cfg(feature = "client")]
#[handlers]
impl ClientTypedSvc {
    /// One client-compatible registration block.
    #[rpc]
    async fn increment(Payload(n): Payload<u32>) -> u32 {
        n + 1
    }
}

#[cfg(not(feature = "client"))]
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
    #[cfg(not(feature = "client"))]
    let daemon = App::builder("test")
        .service::<TypedSvc>()
        .build()
        .await
        .expect("build daemon");

    #[cfg(feature = "client")]
    let daemon = App::builder("test")
        .service::<ClientTypedSvc>()
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

    #[cfg(not(feature = "client"))]
    let first = deadline(
        "TypedSvc.increment call",
        conn.call("TypedSvc.increment", enc(&10u32)),
    )
    .await
    .expect("increment call succeeds");

    #[cfg(feature = "client")]
    let first = deadline(
        "ClientTypedSvc.increment call",
        conn.call("ClientTypedSvc.increment", enc(&10u32)),
    )
    .await
    .expect("increment call succeeds");

    match first {
        CallResult::Ok(body) => assert_eq!(dec::<u32>(&body), 11),

        other => panic!("expected ok from first block, got {other:?}"),
    }

    #[cfg(not(feature = "client"))]
    {
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
    }

    server.shutdown([conn]).await;
}
