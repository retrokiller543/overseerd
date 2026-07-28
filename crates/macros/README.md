# overseerd-macros

> The core procedural macros for the Overseerd framework.

Part of the [Overseerd](../../README.md) framework — the thin proc-macro crate exposing the core,
protocol-agnostic macros.

## Role

This is a `proc-macro = true` crate whose entry points are thin shims: each forwards its token
streams to the matching `expand` function in [`overseerd-macros-core`](../macros-core), the ordinary
library that holds all the parsing and codegen (a proc-macro crate can only export proc-macros).
It exposes the framework's core, protocol-agnostic macros — `#[component]`, `#[config]`,
`#[methods]`, `#[injectable]`, and `app!` — which span
components/DI, configuration, and lifecycle. Protocol-specific macros (`#[service]`, `#[handlers]`,
`#[rpc]`) live in their own crates; only the core stays here. Errors surface as `compile_error!` via
the core, never a panic.

## Usage

Most users depend on the [`overseerd`](../../README.md) facade, which re-exports these — you rarely
name this crate directly. The macros arrive as attributes/macros through the facade's prelude:

```rust
use overseerd::prelude::*;
use std::sync::Arc;

#[config(path = "app.db")]
#[derive(serde::Deserialize)]
struct DbConfig {
    #[default = "postgres://localhost/app"]
    url: String,
}

#[component]
struct Pool {
    config: Cfg<DbConfig>,          // config binding, resolved by path
    #[default]
    hits: std::sync::atomic::AtomicU64, // owned state, Default-built
}

#[methods]
impl Pool {
    #[init]
    async fn connect(config: Cfg<DbConfig>) -> Self { /* ... */ }
}
```

`app!` defines a named lifecycle host and its configured builder, selecting a protocol via its
`protocol:` field. Code that needs local values or post-declaration builder mutation should use
`App::<Protocol>::builder(name).auto_discover()` directly.

Expression-form `app!` and the `daemon!` alias have been removed. See the
[named application migration guide](../../docs/named-application-migration.md) when updating old
call sites. The [`app!` Rustdoc](https://docs.rs/overseerd/latest/overseerd/macro.app.html) is the
authoritative grammar and behavior reference for declaration fields, lifecycle stages, generated
types and methods, exact Clap defaults, command restrictions, plugin composition, tooling, features,
and errors.

When `cli` is enabled, every crate containing a named app needs a direct `clap` dependency with
`derive`, even if that app declares no custom arguments or commands. The macro always generates its
application-specific bootstrap parser. When `cli` is disabled, it generates only builder and
typestate lifecycle APIs; `tooling` remains independently available.

## Internal role

Built entirely on [`overseerd-macros-core`](../macros-core): every macro here is a one-line forward
to a core `expand` function. It sits under the `overseerd` facade, which re-exports these macros; the
per-protocol macro crates (e.g. `overseerd-rpc-macros`) are siblings, not dependents.

## Feature flags

| Feature | Effect |
|---|---|
| `cli` *(default)* | Generate application-specific Clap types and `run`/`run_with` dispatch for named hosts. |
| `tooling` | Generate each named host's private target-local tooling entry. |
| `di-check` | Emit compile-time dependency-injection checks (`impl Provide<Self> for Wiring` per component, plus a `Wiring: Provide<Dep>` bound per concrete dependency). Forwards to `overseerd-macros-core/di-check`. |
| `hybrid-registry` | Force generated registration through `inventory`; Apple/Mach-O hosts select it automatically. |
