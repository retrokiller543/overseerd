//! End-to-end tests for the framework builtins, driven over the in-memory
//! transport. A handler injects the seeded [`ShutdownHandle`] and the call
//! completes, proving the builtin resolves through the request scope chain.

use overseerd::{
    CallResult, Daemon, Inject, MemoryClient, MemoryConnectionHandle, Payload, ShutdownHandle,
    handlers, service,
};

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

async fn start() -> MemoryConnectionHandle {
    let (client, transport) = MemoryClient::pair();

    let daemon = Daemon::builder("builtins-test")
        .auto_discover()
        .build()
        .await
        .expect("build daemon");

    tokio::spawn(async move {
        let _ = daemon.serve(transport).await;
    });

    client.connect().await.expect("connect")
}

fn enc<T: serde::Serialize>(value: &T) -> Vec<u8> {
    postcard::to_allocvec(value).unwrap()
}

fn dec<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> T {
    postcard::from_bytes(bytes).unwrap()
}

#[tokio::test]
async fn handler_can_inject_shutdown_handle() {
    let conn = start().await;

    let result = conn
        .call("BuiltinsSvc.ping", enc(&41u32))
        .await
        .expect("call succeeds");

    match result {
        CallResult::Ok(bytes) => {
            let value: u32 = dec(&bytes);

            assert_eq!(value, 42);
        }

        other => panic!("expected an ok response, got {other:?}"),
    }
}
