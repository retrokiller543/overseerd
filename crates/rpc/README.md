# upwell-rpc

> The Upwell native first-class RPC protocol.

Part of the [Upwell](../../README.md) framework — the native RPC protocol built on the
protocol-agnostic `upwell-app` core.

## Role

This crate provides the `Rpc -> PreparedRpc -> RpcRuntime` protocol states, the RPC router (`RpcRouter`), the `FromContext` extractors
(`Payload`, `Inject`, `Peer`, `Streaming`, …), the tower middleware stack (`Guard`, `RouterService`,
`ErrorHandler`), the wire transports, and the serve loop on top of `upwell-app`. It exposes the
`Rpc`, the specialized `App`/`AppBuilder` aliases, the descriptor model (`ServiceDescriptor`,
`RpcDescriptor`, `SERVICES`, …) that runtime routing and client generation consume, and — under the
`client` feature — the RPC `ProtocolTransport` carry (`StreamClientTransport`, `connect_tcp`,
`connect_unix`) that plugs into the agnostic `upwell-client`. It re-exports the RPC macros
(`#[service]`, `#[handlers]`, `#[rpc]`) it owns via `upwell-rpc-macros`, and the agnostic app
surface so a standalone user has one import.

## Usage

Most users depend on the [`upwell`](../../README.md) facade, which re-exports this crate through
its `daemon` feature (as `upwell::daemon`) — you rarely name it directly. You can also depend on
`upwell-rpc` directly for a self-contained RPC framework (the macros' generated code roots at
`::upwell_rpc::*` unless the `facade` feature switches the root).

```rust
use upwell::{daemon::prelude::*, prelude::*};

#[service(id = "notifications", version = "0.1")]
struct Notifications;

#[handlers]
impl Notifications {
    #[rpc]
    async fn notify(&self, Payload(req): Payload<NotifyRequest>) -> NotifyResponse {
        // ...
    }
}

app! {
    app NotifyApplication {
        name: "notifyd",
        protocol: Rpc,
        serve(_context, app) {
            let transport = TcpTransport::bind("127.0.0.1:7000").await?;

            app.serve(transport).await
        },
    }
}

#[tokio::main]
async fn main() -> Result<(), upwell::CliError> {
    NotifyApplication::run().await
}
```

The [named application migration guide](../../docs/named-application-migration.md) covers the
removed expression form, generated runner and lifecycle APIs, RPC command contexts, static plugins,
tooling mode, and when to use a custom `main` or direct `App::<Rpc>::builder(..)`.

## Internal role

Sits above `upwell-app` (and through it `upwell-di`, `upwell-config`, `upwell-hooks`,
`upwell-transport`, `upwell-dirs`, `upwell-core`) as the concrete RPC protocol. It pairs
with `upwell-rpc-macros`, whose generated code targets the types re-exported here, and with
`upwell-client` for the client side. The `upwell` facade wraps this crate as its `daemon`
module and turns on the `facade` feature so the macros root generated protocol types at
`::upwell::daemon::*`.

## Feature flags

| Feature | Effect |
|---|---|
| `client` | Generate the typed RPC client and its `ProtocolTransport` carry (pulls in `upwell-client`, `async-trait`, and `upwell-rpc-macros/client`). |
| `di-check` | Compile-time DI-graph validation, forwarded across the app/di/config/transport crates and the macros. |
| `yaml` | YAML config sources alongside TOML. |
| `watch` | Reload config on file change. |
| `tracing-subscriber` | The `init_tracing` helper. |
| `facade` | Set by the `upwell` facade: root the macros' generated protocol types at `::upwell::daemon::*`. Off (the default) keeps them at `::upwell_rpc::*` so depending on this crate directly works. |
