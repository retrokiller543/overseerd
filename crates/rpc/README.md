# overseerd-rpc

> The Overseerd native RPC protocol plugin.

Part of the [Overseerd](../../README.md) framework — the native RPC protocol built on the
protocol-agnostic `overseerd-app` core.

## Role

This crate is a `ProtocolPlugin`: it adds the RPC router (`RpcRouter`), the `FromContext` extractors
(`Payload`, `Inject`, `Peer`, `Streaming`, …), the tower middleware stack (`Guard`, `RouterService`,
`ErrorHandler`), the wire transports, and the serve loop on top of `overseerd-app`. It exposes the
`RpcPlugin`, the specialized `App`/`AppBuilder` aliases, the descriptor model (`ServiceDescriptor`,
`RpcDescriptor`, `SERVICES`, …) that runtime routing and client generation consume, and — under the
`client` feature — the RPC `ProtocolTransport` carry (`StreamClientTransport`, `connect_tcp`,
`connect_unix`) that plugs into the agnostic `overseerd-client`. It re-exports the RPC macros
(`#[service]`, `#[handlers]`, `#[rpc]`) it owns via `overseerd-rpc-macros`, and the agnostic app
surface so a standalone user has one import.

## Usage

Most users depend on the [`overseerd`](../../README.md) facade, which re-exports this crate through
its `daemon` feature (as `overseerd::daemon`) — you rarely name it directly. You can also depend on
`overseerd-rpc` directly for a self-contained RPC framework (the macros' generated code roots at
`::overseerd_rpc::*` unless the `facade` feature switches the root).

```rust
use overseerd::{daemon::prelude::*, prelude::*};

#[service(id = "notifications", version = "0.1")]
struct Notifications;

#[handlers]
impl Notifications {
    #[rpc]
    async fn notify(&self, Payload(req): Payload<NotifyRequest>) -> NotifyResponse {
        // ...
    }
}

#[tokio::main]
async fn main() -> overseerd::daemon::Result<()> {
    let app = app! { name: "notifyd", protocol: RpcPlugin }.build().await?;

    app.serve(TcpTransport::bind("127.0.0.1:7000").await?).await
}
```

## Internal role

Sits above `overseerd-app` (and through it `overseerd-di`, `overseerd-config`, `overseerd-hooks`,
`overseerd-transport`, `overseerd-dirs`, `overseerd-core`) as the concrete RPC protocol. It pairs
with `overseerd-rpc-macros`, whose generated code targets the types re-exported here, and with
`overseerd-client` for the client side. The `overseerd` facade wraps this crate as its `daemon`
module and turns on the `facade` feature so the macros root generated plugin types at
`::overseerd::daemon::*`.

## Feature flags

| Feature | Effect |
|---|---|
| `client` | Generate the typed RPC client and its `ProtocolTransport` carry (pulls in `overseerd-client`, `async-trait`, and `overseerd-rpc-macros/client`). |
| `di-check` | Compile-time DI-graph validation, forwarded across the app/di/config/transport crates and the macros. |
| `yaml` | YAML config sources alongside TOML. |
| `watch` | Reload config on file change. |
| `tracing-subscriber` | The `init_tracing` helper. |
| `facade` | Set by the `overseerd` facade: root the macros' generated plugin types at `::overseerd::daemon::*`. Off (the default) keeps them at `::overseerd_rpc::*` so depending on this crate directly works. |
