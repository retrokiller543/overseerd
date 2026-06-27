//! Construction via the `Factory` machinery: `#[init]` constructors driven through
//! the build-time DI traits. Covers a sync constructor, an async fallible one, and
//! the newly-enabled non-`Arc` parameter shapes (`Cfg<T>`).

use std::sync::Arc;

use overseerd::{
    CallResult, Cfg, App, Inject, MemoryClient, MemoryConnectionHandle, Payload, component,
    config, handlers, methods, service,
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
            total: counter.base + cfg.get().seed,
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

    /// Returns the `factory = ..`-built component's label.
    #[rpc]
    async fn label(Inject(tagged): Inject<Arc<Tagged>>, Payload(_): Payload<()>) -> String {
        tagged.label.clone()
    }

    /// Returns the manually-provided component's note.
    #[rpc]
    async fn note(Inject(manual): Inject<Arc<Manual>>, Payload(_): Payload<()>) -> String {
        manual.note.clone()
    }

    /// Returns the value computed by the boxed-error `#[init]`, proving that
    /// constructor's `Box<dyn Error + Send + Sync>` result type was accepted.
    #[rpc]
    async fn boxed(Inject(comp): Inject<Arc<BoxedErrComp>>, Payload(_): Payload<()>) -> u32 {
        comp.value
    }
}

/// Built by an async, fallible `#[init]` returning
/// `Result<Self, Box<dyn Error + Send + Sync>>`, proving an app-defined boxed error
/// converts into `overseerd::Error` (via the catch-all `Error::Other`) and satisfies the
/// factory's `E: Into<Error>` bound. The `?` on a `ParseIntError` exercises that path.
#[component]
struct BoxedErrComp {
    #[default]
    value: u32,
}

#[methods]
impl BoxedErrComp {
    #[init]
    async fn create(
        cfg: Cfg<FactoryCfg>,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let parsed: u32 = "7".parse()?;

        Ok(Self {
            value: cfg.get().seed + parsed,
        })
    }
}

/// Built by an explicit async `factory = path` (not field injection); its `label`
/// field is a plain `String`, which compiles only because no default factory is
/// emitted when `factory = ..` is set.
#[component(factory = Tagged::make)]
struct Tagged {
    label: String,
}

impl Tagged {
    async fn make(counter: Arc<Counter>) -> Self {
        Self {
            label: format!("tagged-{}", counter.base),
        }
    }
}

/// A manual component: no factory is emitted, so it must be provided via
/// `with_component`. Its field is a plain `String` the framework never injects.
#[component(default_factory = false)]
struct Manual {
    note: String,
}

async fn start() -> MemoryConnectionHandle {
    let (client, transport) = MemoryClient::pair();

    let config =
        overseerd::ConfigManager::<overseerd::config::Toml>::from_str("[factory]\nseed = 100\n")
            .expect("parse config");

    let daemon = App::builder("test")
        .auto_discover()
        .config::<FactoryCfg>("factory")
        .config_source(config)
        .with_component(Manual {
            note: "manual-note".to_string(),
        })
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

#[tokio::test]
async fn explicit_factory_path_constructs() {
    let conn = start().await;

    let result = conn.call("FactorySvc.label", enc(&())).await.unwrap();

    match result {
        // Tagged::make ran with its injected Counter (base 0).
        CallResult::Ok(body) => assert_eq!(dec::<String>(&body), "tagged-0"),

        other => panic!("expected ok, got {other:?}"),
    }
}

#[tokio::test]
async fn manual_component_is_provided_not_built() {
    let conn = start().await;

    let result = conn.call("FactorySvc.note", enc(&())).await.unwrap();

    match result {
        CallResult::Ok(body) => assert_eq!(dec::<String>(&body), "manual-note"),

        other => panic!("expected ok, got {other:?}"),
    }
}

#[tokio::test]
async fn boxed_error_init_constructs() {
    let conn = start().await;

    let result = conn.call("FactorySvc.boxed", enc(&())).await.unwrap();

    match result {
        // cfg.seed = 100, parsed = 7 → value = 107.
        CallResult::Ok(body) => assert_eq!(dec::<u32>(&body), 107),

        other => panic!("expected ok, got {other:?}"),
    }
}
