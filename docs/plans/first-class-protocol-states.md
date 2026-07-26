# First-Class Protocol States

Issue: #181  
Parent: #146  
Tracking PR: #161

## Goal

Replace the protocol-plugin adapter model with explicit protocol definition, prepared protocol, and
runtime states. Generated applications select definitions directly:

```rust
app! {
    app Example {
        protocol: Rpc,
    }
}
```

The migration is intentionally breaking. `ProtocolPlugin`, `RpcPlugin`, and `AxumPlugin` are
removed rather than retained through compatibility aliases or wrappers.

## Lifecycle Contract

The application keeps its existing preparation and serving boundaries:

```text
ProtocolDefinition
  -> discovery and definition configuration
  -> protocol registration and pre-build contributions
  -> config, topology, DI, hook, and protocol validation
  -> PreparedProtocol
  -> ordinary root component construction
  -> protocol runtime construction
  -> ProtocolRuntime
  -> App::serve lifecycle envelope
  -> ProtocolRuntime::serve
```

The transitions are consuming. A prepared application physically owns the prepared protocol state,
not the original definition, and a built application physically owns the protocol runtime.

```rust
pub trait ProtocolDefinition: Default + 'static {
    type Prepared: PreparedProtocol<Error = Self::Error>;
    type Error: std::error::Error + Send + Sync + 'static + From<crate::Error>;

    const SCOPE_TOPOLOGY: ScopeTopology;

    fn auto_discover(&mut self) {}
    fn register(&self, registry: &mut AppRegistry);
    fn pre_build(
        &mut self,
        context: &mut PreBuildContext<'_>,
    ) -> Result<(), Self::Error>;
    fn prepare(
        self,
        context: &ValidationContext<'_>,
    ) -> Result<Self::Prepared, Self::Error>;
}

pub trait PreparedProtocol: Send + 'static {
    type Runtime: ProtocolRuntime;
    type Error: std::error::Error + Send + Sync + 'static + From<crate::Error>;

    fn build(self, runtime: &AppRuntime) -> Result<Self::Runtime, Self::Error>;
}

pub trait ProtocolRuntime: Send + 'static {
    type Error: std::error::Error + Send + Sync + 'static;
}

pub trait Serve<E>: ProtocolRuntime {
    fn serve(
        self,
        runtime: AppRuntime,
        shutdown: ShutdownSignal,
        endpoint: E,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send;
}
```

Method names may be adjusted during implementation when that improves diagnostics, but the three
states, consuming transitions, ordering, and ownership are fixed.

The existing concrete protocol error remains the typed error for application preparation and build
in this slice. Host and CLI execution continue to erase it only at the existing `PhaseError`
boundary. Provenance-aware contribution errors belong to the later retained plugin catalog slice.

## Application Types

The selected generic parameter is always the definition:

```rust
pub struct AppBuilder<D: ProtocolDefinition> {
    protocol: D,
    // existing application definition state
}

pub struct PreparedApp<D: ProtocolDefinition> {
    protocol: D::Prepared,
    // existing validated application plan
}

pub struct App<D: ProtocolDefinition> {
    protocol: <D::Prepared as PreparedProtocol>::Runtime,
    // existing runtime and lifecycle state
}
```

`PreparedApp::protocol()` exposes the prepared state. `App::protocol()` exposes the runtime state so
existing runtime inspection call patterns remain useful. Associated-type projections stay inside
`overseerd-app`; generated signatures remain `AppBuilder<D>`, `PreparedApp<D>`, `App<D>`, and
`AppStage<D>`.

No `Sync` bound is added to definitions, prepared states, or runtime states. Host and command paths
continue to move owned state and clone the root container before awaits.

## Scope Ownership

The definition declares `SCOPE_TOPOLOGY` because the application needs it before DI validation and
protocol preparation. `PreparedApp` stores the exact `PreparedScopeTopology` snapshot used for
validation and passes that same snapshot into `AppRuntime`.

The runtime remains responsible for opening protocol lifecycle boundaries and supplying dynamic
seeds. This slice does not add plugin-defined boundaries or change the topology introduced by #178.

## RPC State

- `Rpc` becomes the selected definition and owns discovered/explicit services, middleware, the
  error handler, and limits.
- `PreparedRpc` is a public opaque carrier with private fields. It owns validated resolved services,
  middleware appliers, the error handler, limits, and any construction decisions that do not require
  built components.
- `RpcRuntime` owns the router, folded service, error handler, peer requirement, and limits. The
  existing generic transport `Serve<T>` implementation moves to this type.
- `RpcAppBuilder` targets `AppBuilder<Rpc>`.
- RPC aliases target `App<Rpc>` and `AppBuilder<Rpc>`.

Preparation resolves and validates services exactly once. Runtime construction does not retain the
current fallback revalidation path.

## Axum State

- `Axum` becomes the selected definition and owns controllers, middleware, WebSocket registrations,
  and protocol-owned config and seed contributions.
- `PreparedAxum` is a public opaque carrier with private fields. It owns validated controller,
  middleware, WebSocket, OpenAPI, and config planning state.
- `AxumRuntime` owns the assembled router, reload-aware config handle, and WebSocket endpoint
  handles. Existing `Serve<SocketAddr>`, `Serve<TcpListener>`, and configured `Serve<()>`
  implementations move to this type.
- `AxumAppBuilder` targets `AppBuilder<Axum>`.
- `AxumAppServe` targets `App<Axum>`.
- Axum aliases target `App<Axum>` and `AppBuilder<Axum>`.

Router assembly, DI middleware resolution, controller resolution, and WebSocket protocol construction
remain after root component construction because they consume `AppRuntime`.

## Host And Macro Surface

- `AppHost::Protocol` becomes a `ProtocolDefinition`.
- `AppStage`, host runners, bootstrap helpers, `CommandState`, and `CommandContext` become generic
  over definitions.
- `PreBuild` continues to contain `PreparedApp<D>` and `Built` continues to contain `App<D>`.
- Configured commands still stop after protocol preparation and before ordinary component or runtime
  construction.
- `AppHost` remains a statically dispatched generic contract; no erased host registry is introduced.
- `app!` grammar is unchanged. Its missing-protocol diagnostic and generated documentation use
  protocol-definition terminology.
- Generated applications use `protocol: Rpc` and `protocol: Axum` without protocol-specific macro
  branches.

The common startup, reload, Ctrl-C, panic cleanup, and shutdown envelope remains owned by
`App::serve`. Protocol runtimes implement only the inner endpoint/transport serving operation.

## Public API Proof

`overseerd-app` and the main facade export all three contracts and the scope topology vocabulary a
third-party definition needs. A fixture importing only facade APIs implements:

- a definition with a non-empty topology;
- a distinct public opaque prepared state;
- a runtime and `Serve` implementation;
- a generated named application;
- preparation and build assertions proving runtime construction does not occur early.

The fixture includes state that is `Send` but not `Sync` to prevent accidental strengthening of the
public contract.

## Implementation Sequence

1. Add the three protocol contracts, unit implementations, and structural preparation/build tests.
2. Migrate `AppBuilder`, `PreparedApp`, `App`, host runners, typestate stages, bootstrap helpers, and
   command contexts.
3. Migrate RPC to `Rpc -> PreparedRpc -> RpcRuntime`, preserving service validation, middleware,
   transport, scope, and error behavior.
4. Migrate Axum to `Axum -> PreparedAxum -> AxumRuntime`, preserving config, router, middleware,
   OpenAPI, WebSocket, scope, and serving behavior.
5. Update macro diagnostics, generated documentation, aliases, preludes, examples, and application
   declarations.
6. Add direct-crate and facade-only third-party protocol proofs.
7. Remove obsolete adapter traits, types, exports, references, and documentation atomically.

Core, RPC, Axum, and generated-host changes form one compile-complete migration boundary. Temporary
public compatibility adapters must not survive the branch.

## Validation

- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets --all-features`
- `cargo nextest run --workspace --all-features`
- `cargo check --workspace --no-default-features`
- direct `overseerd-app`, RPC, and Axum feature combinations
- Axum WebSocket and OpenAPI combinations
- relevant wasm client combinations
- generated `Rpc` and `Axum` application compile coverage
- facade-only third-party protocol fixture
- CI and automated review with no remaining findings

## Exclusions

- retained plugin installations and immutable contribution lowering;
- protocol default-plugin and replacement resolution;
- RPC/Axum optional capability extraction;
- Jobs migration;
- plugin CLI providers;
- tooling projections and serialization;
- object-safe erased host registries;
- speculative async protocol construction;
- another scope-topology redesign;
- source-compatibility-only protocol adapters.
