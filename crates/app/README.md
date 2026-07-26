# overseerd-app

> The Overseerd protocol-agnostic application core: `App`/`AppBuilder`, protocol states, plugins, the DI runtime handle, scope planning, lifecycle, and builtins.

Part of the [Overseerd](../../README.md) framework — the application core, sitting above the config/DI/hooks/dirs layers and below the protocol crates (`overseerd-rpc`, `overseerd-axum`).

## Role

This crate ties the DI engine, config, hooks, and dirs into a runnable [`App`] that is generic over its [`ProtocolDefinition`]. It owns the [`AppBuilder`], the agnostic [`AppRegistry`], scope planning, the lifecycle/serve envelope, the [`AppRuntime`] handle a protocol drives requests through, and the `ProtocolDefinition -> PreparedProtocol -> ProtocolRuntime` transition. It is *protocol-agnostic*: it knows nothing of RPC, HTTP, or any wire format. A protocol is a sibling crate that implements these traits over this foundation.

## Usage

Most users depend on the [`overseerd`](../../README.md) facade, which re-exports this crate — you rarely name it directly. You meet it through the `app!` macro (which expands to an [`AppBuilder`]), the `.build().await` / `.serve(..)` lifecycle it produces, and builtin config types like [`ServerConfig`] and [`LoggingConfig`]. The selected definition, such as `Rpc` or `Axum`, is the protocol-specific choice.

```rust
use overseerd::{daemon::prelude::*, prelude::*};

#[tokio::main]
async fn main() -> overseerd::daemon::Result<()> {
    let app = app! {
        name: "notifyd",
        protocol: Rpc,
    }
    .build()
    .await?;

    app.serve(TcpTransport::bind("127.0.0.1:7000").await?).await
}
```

## Internal role

The protocol crates (`overseerd-rpc`, `overseerd-axum`) build directly on this crate: each provides definition, prepared, and runtime states plus [`Serve`] implementations, and drives requests through the [`AppRuntime`] handle. The `overseerd` facade re-exports the whole surface, and the `app!` macro (in `overseerd-macros`) targets the [`AppBuilder`] here.

## Feature flags

| Feature | Effect |
|---|---|
| `yaml` | forward YAML config support (`overseerd-config/yaml`) |
| `watch` | forward config file watching/reload (`overseerd-config/watch`) |
| `tracing-subscriber` | pull in `tracing-subscriber` for the `init_tracing` helper |
| `di-check` | compile-time DI graph validation (forwards to `overseerd-di`/`overseerd-config`) |
