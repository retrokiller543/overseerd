# upwell-hooks

> A general lifecycle/event hook framework for Upwell, generic over the core resolver.

Part of the [Upwell](../../README.md) framework — the hook layer, above `upwell-core` and below the DI engine.

## Role

`upwell-hooks` defines a general lifecycle/event **hook** system. A hook is an `async` method on a component or service, marked `#[hook(Kind)]`, that the framework calls when an event of that [`HookKind`] occurs. A kind owns the contract: its `Output` (what each hook returns) and its `Cx` (the typed per-invocation context that a hook's [`HookParam`]s read). Built-in kinds are [`Startup`] and [`Shutdown`] (config reload is a kind defined in `upwell-config`), and user-defined kinds are added the same way. Hooks are collected per type into a `{Type}Hooks` distributed slice (exposed via [`ComponentHooks`]) and run by a [`HookManager`], which indexes them by kind for O(1) listener lookup. The crate is generic over the [`ResolverCtx`](upwell_core::ResolverCtx): an erased [`HookCall`] resolves its `&self` receiver through the resolver context, so this layer never names the DI container that sits above it.

## Usage

Most users depend on the [`upwell`](../../README.md) facade, which re-exports this crate — you rarely name it directly. You meet it through the `#[hook(..)]` attribute on a method, using the re-exported [`Startup`]/[`Shutdown`] kinds.

```rust
use upwell::prelude::*;

#[component]
pub struct Cache;

#[methods]
impl Cache {
    // Runs when the framework fires the Startup kind; reached via `&self`,
    // dependencies are not passed as parameters.
    #[hook(Startup)]
    async fn warm(&self) {
        // ...
    }
}
```

## Internal role

`upwell-di` carries a hook slice on every [`ComponentDescriptor`] (via the [`ComponentHooks`] fn pointer) so the registry can reach a type's hooks without holding its type. `upwell-app` and the protocol crates build a [`HookManager`] from all registered hooks, seed it as a framework singleton ([`HOOK_MANAGER_ID`]/[`HOOK_MANAGER_NAME`]), attach the root resolver context, and fire kinds like [`Startup`]/[`Shutdown`] during the lifecycle. `upwell-config` defines its config-reload kind on top of this same system.

## Feature flags

None.
