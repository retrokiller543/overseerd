# overseerd-di

> The Overseerd dependency-injection engine: scope containers, factories, and component descriptors over the core resolver.

Part of the [Overseerd](../../README.md) framework — the runtime DI engine, sitting above `overseerd-core` and `overseerd-hooks`.

## Role

`overseerd-di` owns the runtime DI machinery: the parent-linked [`ScopeContainer`], the construction-time [`Factory`]/[`FromContainer`] extractors, the component and provider descriptors ([`ComponentDescriptor`], [`ProviderDescriptor`]), and the [`ComponentRegistry`] that validates the graph. It builds on the leaf vocabulary in `overseerd-core` (type descriptors, the dependency model, the resolver abstraction) and on `overseerd-hooks` for the per-component hook slice each descriptor carries. Config is deliberately *not* here — it is an external resolver (`overseerd-config`) reached through the [`ResolverCtx`](overseerd_core::ResolverCtx), so the container stays unaware of it.

## Usage

Most users depend on the [`overseerd`](../../README.md) facade, which re-exports this crate — you rarely name it directly. You meet it through the `#[component]`/`#[service]` macros, which generate the [`Component`]/[`Injectable`] impls and register [`ComponentDescriptor`]s into the [`COMPONENTS`] distributed slice; field injection and `Inject<T>` resolve through the container it builds.

```rust
use overseerd::prelude::*;

// `#[component]` generates the Component/Injectable impls and registers a
// descriptor; the DI engine wires `db` into `Store` at construction time.
#[component(by_value)]
#[derive(Clone)]
pub struct Db;

#[component]
pub struct Store {
    db: Db,
}
```

## Internal role

This is the engine the application layer drives. `overseerd-dirs` implements [`Component`]/[`Injectable`] for its `Dir<K>` and `DirectoriesManager` against these traits. `overseerd-config` seeds itself as an external resolver reachable via the [`ResolverCtx`], and `overseerd-app` plus the protocol crates (`overseerd-rpc`, `overseerd-axum`) build [`ScopeContainer`]s, seed framework singletons (including the [`HookManager`](overseerd_hooks::HookManager)), and resolve components through this crate.

## Feature flags

| Feature | Effect |
|---|---|
| `di-check` | Emit compile-time DI checks (`Wiring: Provide<Dep>` bounds) so a missing provider is a `cargo check` error rather than a runtime failure. |
