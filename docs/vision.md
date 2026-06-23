# Overseerd Project Vision

## Vision Statement

Overseerd is a Rust framework for building long-running daemon services from
strongly typed components, services, and generated infrastructure.

The framework should make daemon development feel like writing ordinary Rust
business logic. Developers define reusable components, services, RPC handlers,
and runtime assembly points while Overseerd handles service discovery, dependency
wiring, RPC registration, lifecycle management, and operational concerns.

The guiding idea is:

> Boilerplate should be generated. Ownership should remain explicit.

Overseerd should sit between low-level runtime libraries and fully managed
application containers: more automation than hand-rolled daemon infrastructure,
more explicitness than convention-driven frameworks, and strongly typed Rust APIs
instead of stringly-typed configuration.

## Problem

Long-running services repeatedly need the same infrastructure:

- service and component registration
- dependency wiring
- RPC routing and endpoint registration
- startup sequencing
- graceful shutdown coordination
- signal handling
- spawned task tracking and supervision
- local IPC transports
- request context extraction
- runtime introspection
- typed client libraries
- operational surfaces such as logs, health, and metrics

This code is essential, but it often distracts from domain logic. It is also easy
to implement inconsistently: services drift from clients, dependency graphs are
implicit, shutdown paths are incomplete, and generated or discovered behavior is
hard to inspect.

Overseerd should reduce that repetition by using metadata-driven generation and
runtime assembly while preserving explicit control over application startup and
configuration.

## Target Developer Experience

A daemon should be defined as a collection of components and services.

Components provide reusable dependencies:

```rust
#[component]
struct BackupRepository {
    // ...
}
```

Services expose daemon functionality:

```rust
#[service]
struct BackupService;

#[rpc]
impl BackupService {
    async fn start_backup(
        repo: Component<BackupRepository>,
        Payload(input): Payload<BackupInput>,
    ) -> Result<JobId> {
        // ...
    }
}
```

Overseerd discovers and registers metadata describing those components, services,
dependencies, and RPC endpoints. The runtime then consumes that metadata to
construct a runnable daemon.

```rust
fn main() -> anyhow::Result<()> {
    setup_logging();

    let runtime = tokio::runtime::Runtime::new()?;

    runtime.block_on(async {
        Daemon::builder("backupd")
            .auto_discover()
            .run()
            .await
    })
}
```

The important constraint is that Overseerd does not take ownership of `main`.
Users remain free to configure logging, construct the Tokio runtime, load
environment variables, perform startup validation, and integrate with external
tooling before handing control to the daemon runtime.

## Core Product Pillars

### 1. User-Owned Runtime

Overseerd should never require ownership of application startup. Runtime helpers
and convenience macros can reduce boilerplate, but developers must remain in
control of process startup, runtime construction, configuration loading, and
deployment decisions.

This keeps Overseerd usable in real services where startup order, observability
setup, runtime tuning, and host integration are application-specific.

### 2. Convention-Assisted Discovery

Components and services can be discovered automatically through generated
metadata, but automatic discovery must be optional. Anything that can be
discovered by convention should also be configurable explicitly.

```rust
Daemon::builder()
    .component(custom_database)
    .service::<BackupService>()
```

Automatic registration is a convenience feature, not a requirement. Explicit
registration must remain available for tests, unusual deployments, plugin-style
systems, and applications that prefer manual assembly.

### 3. Magic Must Be Inspectable

Generated infrastructure should never become invisible infrastructure.
Developers should be able to inspect registered services, registered RPC
endpoints, component dependency graphs, generated API contracts, active
transports, and other runtime metadata.

Generated behavior should be understandable, debuggable, and overrideable.
Overseerd can reduce boilerplate, but it must not make service behavior opaque.

### 4. Metadata First

Overseerd's procedural macros should primarily generate metadata and descriptors
rather than embedding large amounts of runtime behavior. Examples include:

- component descriptors
- service descriptors
- RPC descriptors
- dependency graphs
- API contracts

Runtime systems consume these descriptors to provide execution, routing,
validation, introspection, and tooling. This makes future SDK generation, API
inspection, validation tooling, documentation generation, and plugin systems
possible from the same source of truth.

### 5. Typed Interfaces Over Protocol Glue

Public daemon APIs should be described with Rust types. Overseerd should use those
types and the generated metadata model to derive RPC contracts, request and
response serialization, and Rust client SDKs.

The goal is to reduce drift between server and client implementations without
forcing users into stringly-typed configuration or hand-written protocol glue.

## Initial Goals

The first versions of Overseerd should focus on a coherent daemon foundation that
proves the metadata model and user-owned runtime approach:

- component registration and dependency injection
- service registration
- typed RPC handlers
- graceful startup and shutdown
- task supervision
- Unix socket transport
- request context extraction
- runtime introspection
- generated Rust client SDKs

These goals intentionally prioritize the core framework shape over broad
operational integrations. Configuration management, health checks, metrics,
authentication, background jobs, and additional transports can build on the same
metadata model after the foundation is validated.

## Future Directions

Potential future capabilities include:

- additional transports
- configuration management
- health checks
- metrics integration
- authentication and authorization
- background job systems
- multi-language SDK generation
- service plugins
- deployment tooling

These features should extend the component, service, RPC, and metadata model
rather than introducing separate abstractions.

## Possible Architecture Direction

A future Overseerd stack may include:

- `overseerd-core`: component and service descriptors, dependency graph metadata,
  daemon builder traits, lifecycle types, shutdown coordination, task supervision
  primitives, and runtime introspection models
- `overseerd-runtime`: Tokio-oriented runtime integration, daemon execution,
  lifecycle orchestration, and default implementations
- `overseerd-macros`: `#[component]`, `#[service]`, `#[rpc]`, and related metadata
  generation macros
- `overseerd-ipc`: Unix socket transport and transport abstraction points
- `overseerd-client`: generated or derived Rust client SDK support
- `overseerd-observability`: optional tracing, metrics, health, and readiness
  helpers once the core metadata model is proven
- `overseerd-config`: optional configuration loading and reload helpers
- `overseerd-jobs`: optional background job registration, progress, cancellation,
  and status tracking

This layout is illustrative. The actual crate structure should follow validated
implementation needs rather than premature modularity.

## Initial Development Strategy

Early development should prioritize a narrow but complete vertical slice:

1. Define the metadata model for components, services, RPC descriptors,
   dependency graphs, and API contracts.
2. Implement explicit component and service registration APIs.
3. Add convention-assisted discovery on top of the explicit registration model.
4. Build typed RPC routing over a Unix socket transport.
5. Support request context extraction for handler parameters.
6. Provide lifecycle management, graceful shutdown, and supervised task execution.
7. Expose runtime introspection for registered components, services, endpoints,
   dependencies, contracts, and transports.
8. Generate a Rust client SDK from the same metadata used by the runtime.

The framework should not begin by hiding behavior behind macros alone. The macro
API must sit on top of runtime components and metadata descriptors that are
understandable, testable, inspectable, and useful on their own.

## Success Criteria

Overseerd is succeeding when:

- a daemon can be assembled from strongly typed components and services
- users retain explicit ownership of `main`, runtime construction, and startup
  configuration
- automatic discovery can be replaced with explicit registration without changing
  the daemon model
- generated metadata exposes services, endpoints, dependency graphs, API
  contracts, and active transports for inspection
- typed RPC handlers reduce protocol glue and client/server drift
- graceful shutdown and supervised task behavior are predictable and testable
- generated Rust clients are derived from the same source of truth as the daemon
  runtime

## Guiding Tradeoffs

- Prefer generated boilerplate with explicit ownership boundaries.
- Prefer metadata descriptors over hidden runtime behavior.
- Prefer convention-assisted workflows over convention-required frameworks.
- Prefer strongly typed Rust APIs over stringly-typed configuration.
- Prefer explicit registration paths even when auto-discovery exists.
- Prefer runtime behavior that can be inspected, debugged, and overridden.
- Prefer a small validated core before broad transport, deployment, or SDK scope.
