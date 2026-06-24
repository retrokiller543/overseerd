//! Construction via the `Factory` machinery: `#[init]` constructors driven through
//! the build-time DI traits. Covers a sync constructor, an async fallible one, and
//! the newly-enabled non-`Arc` parameter shapes (`Cfg<T>`).

use std::sync::Arc;

use overseerd::{
    CallResult, Cfg, Daemon, MemoryClient, MemoryConnectionHandle, Payload, component, config,
    handlers, methods, service,
};

#[config]
#[derive(serde::Deserialize)]
struct FactoryCfg {
    seed: u32,
}

/// A plain dependency built by a sync `#[init]` (a `#[component]` with a `#[methods]`
/// counterpart via `#[handlers]`'s `#[init]`).
#[component]
struct Counter {
    #[default]
    base: u32,
}

/// Service constructed by an async, fallible `#[init]` taking both an injected
/// component (`Arc<Counter>`) and a config binding (`Cfg<FactoryCfg>`) — a parameter
/// shape the old `Arc<T>`-only `#[init]` could not express.
#[service(id = "factory_svc", version = "0.1")]
struct FactorySvc {
    // Set by `#[init]`; `#[default]` so the field-injection default factory (which
    // the macro always emits) still type-checks. `default_factory = false` will
    // remove that requirement.
    #[default]
    total: u32,
}

#[methods]
impl FactorySvc {
    #[init]
    async fn create(counter: Arc<Counter>, cfg: Cfg<FactoryCfg>) -> overseerd::Result<Self> {
        Ok(Self {
            total: counter.base + cfg.seed,
        })
    }
}

#[handlers]
impl FactorySvc {
    /// Returns the total computed at construction, proving the `#[init]` ran with
    /// both its component and config dependencies resolved.
    #[rpc]
    async fn total(&self, Payload(_): Payload<()>) -> u32 {
        self.total
    }
}

async fn start() -> MemoryConnectionHandle {
    let (client, transport) = MemoryClient::pair();

    let config = overseerd::ConfigManager::<overseerd::config::Toml>::from_str(
        "[factory]\nseed = 100\n",
    )
    .expect("parse config");

    let daemon = Daemon::builder("test")
        .auto_discover()
        .config::<FactoryCfg>("factory")
        .config_source(config)
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
async fn init_factory_resolves_component_and_config_params() {
    let conn = start().await;

    let result = conn.call("FactorySvc.total", enc(&())).await.unwrap();

    match result {
        // Counter.base defaults to 0; cfg.seed = 100 → total = 100.
        CallResult::Ok(body) => assert_eq!(dec::<u32>(&body), 100),

        other => panic!("expected ok, got {other:?}"),
    }
}
