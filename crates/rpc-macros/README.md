# overseerd-rpc-macros

> The Overseerd RPC daemon macros: `#[service]`, `#[handlers]`, `#[rpc]`.

Part of the [Overseerd](../../README.md) framework — the per-protocol macro crate for the native RPC
plugin.

## Role

A `proc-macro = true` crate holding the RPC-protocol-specific macros. Because they emit
protocol plugin types (`::overseerd::daemon::*` or `::overseerd_rpc::*`), they can't live in the
protocol-agnostic `overseerd-macros`, so they get their own crate built on the shared
[`overseerd-macros-core`](../macros-core) codegen via its extension seam:

- `#[service]` is a **router component** — a `#[component]` (field-injected singleton) plus a service
  header, its `{Service}Rpcs` slice, its `ServiceDescriptor`, and (under `client`) the generated
  `{Service}Client<C>`. Expands via `expand_component` with the `Router` extension.
- `#[handlers]` is `MethodArgs<Rpcs>` — the base impl macro (`#[methods]`: `#[init]` + `#[hook]`)
  plus the RPC extension that registers each `#[rpc]` method into the service's slice and contributes
  the client methods. Several `#[handlers]` blocks merge with no coordination.
- `#[rpc]` marks a method inside a `#[handlers]` impl; a marker stripped by `#[handlers]` (used
  standalone it emits a `compile_error!`).

`app!`/`daemon!` are **not** here — they are protocol-agnostic core macros in `overseerd-macros`,
selecting the protocol via a `protocol:` field.

## Usage

Most users depend on the [`overseerd`](../../README.md) facade, which re-exports these through its
`daemon` module (via `overseerd-rpc`) — you rarely name this crate directly. They arrive as
attributes on your service types:

```rust
use overseerd::daemon::{handlers, rpc, service, Payload};

#[service(id = "notifications", version = "0.1")]
struct Notifications;

#[handlers]
impl Notifications {
    #[rpc]
    async fn notify(&self, Payload(req): Payload<NotifyRequest>) -> NotifyResponse {
        // ...
    }
}
```

## Internal role

Built on [`overseerd-macros-core`](../macros-core), reusing `expand_component`, `methods::expand`,
`MethodArgs`, `Paths`, and the `run` parse-and-expand harness. It is re-exported by
[`overseerd-rpc`](../rpc) (`pub use overseerd_rpc_macros::{handlers, rpc, service}`), which the
`overseerd` facade surfaces as `overseerd::daemon`. The `facade` feature (set by the facade) switches
the generated plugin-type root between `::overseerd::daemon` and the standalone `::overseerd_rpc`.

## Feature flags

| Feature | Effect |
|---|---|
| `client` | Emit the generated RPC client (the `cfg!(feature = "client")` gates in `#[service]`/`#[handlers]`). Forwards to `overseerd-macros-core/client`. |
| `di-check` | Emit the compile-time dependency-injection assertions. Forwards to `overseerd-macros-core/di-check`. |
| `facade` | Root generated plugin types at the `overseerd` facade (`::overseerd::daemon::*`) instead of this protocol crate (`::overseerd_rpc::*`). Enabled by the facade; off means standalone `overseerd-rpc` is the root. Core vocabulary is always `::overseerd` either way. |
