# Overseer
*small change*

Overseer is a Rust framework for building long-running daemon services using strongly typed components, services, and generated infrastructure.

The goal is to make daemon development feel like writing ordinary Rust business logic while Overseer handles service discovery, dependency wiring, RPC registration, lifecycle management, and operational concerns.

Unlike fully convention-driven frameworks, Overseer does not take ownership of your application entrypoint or runtime configuration. Developers remain in control of process startup, runtime construction, and deployment decisions while benefiting from generated infrastructure and convention-assisted wiring.

## Philosophy

Overseer is built around a simple idea:

> Boilerplate should be generated. Ownership should remain explicit.

The framework embraces code generation and metadata discovery to reduce repetitive daemon infrastructure while preserving the ability to inspect, customize, and override behavior when necessary.

Overseer aims to sit between minimal frameworks and fully managed application containers:

* More automation than low-level runtime libraries
* More explicitness than large convention-driven frameworks
* Strongly typed Rust APIs instead of stringly-typed configuration
* Convention-assisted, not convention-required

## Vision

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

Overseer automatically discovers and registers metadata describing those services, dependencies, and RPC endpoints.

The runtime then consumes that metadata to construct a runnable daemon.

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

## Core Principles

### User-Owned Runtime

Overseer should never require ownership of `main`.

Users must remain free to:

* Configure logging before runtime startup
* Build custom Tokio runtimes
* Load environment variables
* Perform startup validation
* Integrate with external tooling

Overseer provides runtime helpers and convenience macros, but they should remain optional.

### Convention-Assisted Discovery

Components and services can be discovered automatically through generated metadata.

However, everything that can be discovered automatically should also be configurable explicitly.

```rust
Daemon::builder()
    .component(custom_database)
    .service::<BackupService>()
```

Automatic registration is a convenience feature, not a requirement.

### Magic Must Be Inspectable

Generated infrastructure should never become invisible infrastructure.

Developers should be able to inspect:

* Registered services
* Registered RPC endpoints
* Component dependency graphs
* Generated API contracts
* Active transports

Overseer should make generated behavior easy to understand and debug.

### Metadata First

Overseer's procedural macros primarily generate metadata and descriptors rather than runtime behavior.

Examples include:

* Component descriptors
* Service descriptors
* RPC descriptors
* Dependency graphs
* API contracts

Runtime systems consume these descriptors to provide execution, routing, validation, and tooling.

This architecture enables future capabilities such as:

* SDK generation
* API inspection
* Validation tooling
* Documentation generation
* Plugin systems

## Initial Goals

The first versions of Overseer should focus on a coherent daemon foundation:

* Component registration and dependency injection
* Service registration
* Typed RPC handlers
* Graceful startup and shutdown
* Task supervision
* Unix socket transport
* Request context extraction
* Runtime introspection
* Generated Rust client SDKs

## Future Directions

Potential future capabilities include:

* Additional transports
* Configuration management
* Health checks
* Metrics integration
* Authentication and authorization
* Background job systems
* Multi-language SDK generation
* Service plugins
* Deployment tooling

These features should build upon the same metadata model rather than introducing separate abstractions.

## Non-Goals

Overseer is not intended to:

* Replace Tokio
* Replace existing observability ecosystems
* Hide all runtime decisions
* Become a distributed systems platform
* Require framework ownership of application startup
* Force a specific deployment model

## Project Status

Overseer is currently in the design and foundation phase.

Public APIs, macro syntax, dependency injection semantics, and runtime architecture are expected to evolve as real-world daemon implementations are built on top of the framework.

The current focus is establishing a strong metadata model capable of powering dependency injection, RPC registration, runtime introspection, and future SDK generation from a single source of truth.
