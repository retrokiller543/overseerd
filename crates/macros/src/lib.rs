//! Procedural macros for the Overseerd framework.
//!
//! These are re-exported from the `overseerd` facade crate; depend on that rather
//! than this crate directly. They span three subsystems — components (dependency
//! injection), services (RPC), and configuration.
//!
//! | Macro                       | Applies to | Produces |
//! |-----------------------------|------------|----------|
//! | `#[derive(Component)]`      | struct/enum| `Component` impl only (metadata) |
//! | `#[component]`              | struct     | `Component` impl + a registered, system-built factory |
//! | `#[service]`                | struct     | `Component` + `ServiceComponent` impls + a service header + default factory |
//! | `#[handlers]`               | impl block | RPC handlers + RPC group; optional `#[init]` constructor |
//! | `#[rpc]` / `#[init]`        | method     | markers consumed by `#[handlers]` |
//! | `#[injectable]`             | trait      | `Provide<dyn Trait>` impl (under `di-check`) |
//! | `#[config]`                 | struct     | `ConfigProperties` impl; auto-registers a binding when given `#[config(path = "..")]` |
//!
//! # Components: two ways to provide one
//!
//! A *component* is a singleton dependency, resolved by type. There are two ways
//! to get one into the daemon's container:
//!
//! 1. **System-constructed** — annotate the type with `#[component]` (or a
//!    stateful `#[service]`). The macro registers a factory; the container builds
//!    the instance from its dependencies during startup.
//! 2. **Manually provided** — construct the instance yourself and hand it to
//!    `DaemonBuilder::with_component`. The type only needs a `Component` impl,
//!    which `#[derive(Component)]` supplies.
//!
//! Both forms register a descriptor in the `DescriptorRegistry`; the difference
//! is whether the descriptor carries a factory or expects a provided instance.
//!
//! # Field injection
//!
//! `#[component]` and `#[service]` build their factory by *field injection*. For
//! each field of the struct:
//!
//! - an `Arc<T>` field is treated as a **dependency** and resolved from the
//!   container (`cx.resolve::<T>()`);
//! - a `Cfg<T>` field carrying `#[config("path")]` is a **config binding** resolved
//!   by property path (omit the path for the sole-binding shorthand);
//! - a `#[default]` field is **owned state**, built with `Default::default()` — so
//!   its type must implement `Default`, otherwise construct the component another
//!   way (an `#[init]` constructor, or `with_component`).
//!
//! # Implementation
//!
//! Structure follows the dtolnay convention (see `thiserror-impl`): each
//! `#[proc_macro_*]` entry point is thin, delegating to an `expand` function that
//! returns `syn::Result`, with errors surfaced through
//! `syn::Error::into_compile_error` rather than panics. Registration is done by
//! emitting `#[linkme::distributed_slice]` elements into the per-kind descriptor
//! slices that `DaemonBuilder::auto_discover` collects.

extern crate proc_macro;

mod attr;
mod component;
mod config;
mod daemon;
mod derive;
mod di;
mod handle;
mod handlers;
mod inject;
mod injectable;
mod methods;
mod paths;
mod provide;
mod rpc;
mod service;

use proc_macro::TokenStream;
use syn::{DeriveInput, ItemFn, ItemImpl, ItemStruct, parse_macro_input};

/// Declares a **system-constructed singleton component** on a struct.
///
/// The container builds the instance during startup by *field injection*: each
/// field is resolved from the container as a dependency (its type is an injectable
/// handle — `Arc<T>`, `Cfg<T>` for config, a trait-object collection, …), unless it
/// carries `#[default]`, which makes it owned state built with `Default::default()`.
/// Use this for dependencies the system can assemble itself (pools, clients composed
/// from other components, …). For an instance you must build yourself, use
/// `DaemonBuilder::with_component` with a `#[derive(Component)]` type instead.
///
/// # Arguments
///
/// Both optional:
/// - `id` — unique component id. Defaults to the lowercased type name.
/// - `name` — display name. Defaults to the type name.
///
/// ```ignore
/// #[component]                         // id = "dbpool", name = "DbPool"
/// #[component(id = "db", name = "Db")] // explicit
/// ```
///
/// # What it generates
///
/// - `impl Component for T` (carrying `ID`/`NAME`);
/// - a field-injection factory and a `ComponentDescriptor` (with
///   `default_factory: false`), registered into the `COMPONENTS` slice and
///   picked up by `auto_discover`.
///
/// # Example
///
/// ```ignore
/// use overseerd::prelude::*;
/// use std::sync::Arc;
///
/// #[derive(Component)]
/// struct Config { url: String }
///
/// /// Built from `Config` (resolved) plus owned state (`#[default]`).
/// #[component]
/// struct Pool {
///     config: Arc<Config>,   // dependency, resolved from the container
///     #[default]
///     hits: std::sync::atomic::AtomicU64, // owned state, Default-built
/// }
/// ```
///
/// # Errors
///
/// Emits a `compile_error!` if applied to anything but a struct. If a `#[default]`
/// field doesn't implement `Default`, the *generated* factory fails to compile.
///
/// # See also
///
/// `#[service]` (a component that also exposes RPCs and a version) and
/// `#[derive(Component)]` (metadata only, for manually-provided instances).
#[proc_macro_attribute]
pub fn component(attr: TokenStream, item: TokenStream) -> TokenStream {
    let args = parse_macro_input!(attr as attr::ServiceArgs);
    let item = parse_macro_input!(item as ItemStruct);

    component::expand(args, item)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Implements the `Component` metadata trait (`ID`, `NAME`) for a type.
///
/// This generates **only** the trait impl — no factory, no registration. Use it
/// for a type you construct yourself and register at runtime via
/// `DaemonBuilder::with_component` (typically config or other data a factory
/// can't assemble). For a component the system should *build*, use the
/// `#[component]` attribute macro instead.
///
/// # Arguments
///
/// Override the defaults with a `#[component(...)]` helper attribute (both
/// optional):
/// - `id` — defaults to the lowercased type name.
/// - `name` — defaults to the type name.
///
/// ```ignore
/// #[derive(Component)]
/// #[component(id = "app_config", name = "AppConfig")]
/// struct Config { /* ... */ }
/// ```
///
/// # Example
///
/// ```ignore
/// use overseerd::prelude::*;
///
/// #[derive(Component)]
/// struct Config { greeting: String }
///
/// // ...later, at startup:
/// // Daemon::builder("app").with_component(Config { greeting: "Hi".into() })
/// ```
#[proc_macro_derive(Component, attributes(component))]
pub fn derive_component(item: TokenStream) -> TokenStream {
    let input = parse_macro_input!(item as DeriveInput);

    derive::expand(input)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Implements the `ConfigProperties` trait for a config struct, making it injectable
/// as `Cfg<T>` from a property path.
///
/// The type must also derive `Deserialize`. With `#[config(path = "..")]` the binding
/// is auto-registered (picked up by `auto_discover`); without a path, bind it
/// explicitly with `DaemonBuilder::config::<T>(path)` — needed when the same type is
/// bound at several paths. `#[config(name = "..")]` overrides the display name.
///
/// Distinct from the **field-level** `#[config("path")]` inside a `#[component]` /
/// `#[service]` struct, which marks a `Cfg<T>` injection site (consumed by that
/// macro's expansion); this struct-level form declares the config type itself.
///
/// ```ignore
/// #[config(path = "app.server")]
/// #[derive(Deserialize)]
/// struct ServerConfig { addr: String, port: u16 }
/// ```
#[proc_macro_attribute]
pub fn config(attr: TokenStream, item: TokenStream) -> TokenStream {
    let args = parse_macro_input!(attr as config::ConfigArgs);
    let item = parse_macro_input!(item as ItemStruct);

    config::expand(args, item)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Declares a **service** on a struct: its identity, version, and a default
/// singleton factory.
///
/// A service is a component that exposes RPC methods (added by `#[handlers]`
/// impls) and carries a version. The struct is the service's singleton: stateful
/// `&self` RPC methods read it; stateless methods ignore it. Like `#[component]`,
/// the default factory is built by field injection (`Arc<T>` fields resolved,
/// others `Default`-built); an `#[init]` constructor in a `#[handlers]` impl
/// overrides it.
///
/// # Arguments
///
/// All optional:
/// - `id` — unique service id. Defaults to the lowercased type name.
/// - `name` — display name; also the RPC path prefix (`Name.method`). Defaults to
///   the type name.
/// - `version` — e.g. `"0.1"`. Defaults to none.
/// - `rpc_slice` — overrides the generated per-service RPC slice name (default
///   `{Service}Rpcs`); an escape hatch for a name collision. Each `#[handlers]`
///   block for the service must then pass the same `rpc_slice = ..`.
///
/// ```ignore
/// #[service(id = "greeter", version = "0.1")]
/// struct Greeter { /* ... */ }
/// ```
///
/// # What it generates
///
/// - `impl Component for T` and `impl ServiceComponent for T` (the latter carries
///   `VERSION`, enabling `DaemonBuilder::with_service`);
/// - a public `{Service}Rpcs` distributed slice (e.g. `GreeterRpcs`) that the
///   service's `#[handlers]` blocks append their RPC groups to, plus an
///   `impl ServiceRpcs for T` returning it;
/// - a `ServiceDescriptor` header registered into the `SERVICES` slice, whose
///   `rpcs` field points at `ServiceRpcs::rpc_groups` — so the service *owns* its
///   RPC surface and registering the service registers its methods;
/// - a default field-injection factory (`default_factory: true`), overridable by
///   an `#[init]` constructor.
///
/// RPC methods are **not** declared here — add one or more `#[handlers] impl`
/// blocks for that.
///
/// # Example
///
/// ```ignore
/// use overseerd::prelude::*;
/// use std::sync::Arc;
///
/// #[derive(Component)]
/// struct Config { greeting: String }
///
/// #[service(id = "greeter", version = "0.1")]
/// struct Greeter { config: Arc<Config> }
///
/// #[handlers]
/// impl Greeter {
///     #[rpc]
///     async fn ping() -> Result<String> { Ok("pong".into()) }
/// }
/// ```
///
/// # Errors
///
/// Emits a `compile_error!` if applied to anything but a struct.
#[proc_macro_attribute]
pub fn service(attr: TokenStream, item: TokenStream) -> TokenStream {
    let args = parse_macro_input!(attr as attr::ServiceArgs);
    let item = parse_macro_input!(item as ItemStruct);

    service::expand(args, item)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Contributes the `#[rpc]` methods (and an optional `#[init]` constructor) of an
/// inherent `impl` block to the service of `Self`.
///
/// Several `#[handlers] impl T` blocks may target the same service — each appends
/// its RPC group to the service's `{Service}Rpcs` slice (declared by `#[service]`),
/// so the blocks merge with no coordination. A block in the **same module** as the
/// `#[service]` just works; a block in a **different module** must bring the slice
/// into scope first: `use path::{Service}Rpcs;`. A `#[handlers]` impl for a type
/// with no `#[service]` fails to compile — the slice it would append to does not
/// exist.
///
/// # `#[rpc]` methods
///
/// Each `#[rpc]` method must be `async` and return `Result<R, E>` where `R:
/// Serialize` and `E: Into<overseerd::Error>`. Parameters are *extractors* drawn
/// from the call context:
/// - `Payload<T>` — the deserialized request body;
/// - `Inject<H>` — a component resolved from the call scope (a connection- or
///   request-scoped component, or any singleton), by its handle `H`.
///
/// A method may take `&self` to read the service singleton's common
/// dependencies; one that needs none omits `self` and is dispatched directly
/// (no per-call singleton lookup). `&mut self` and `self`-by-value are rejected.
///
/// `#[rpc]` accepts an operation kind argument (only `#[rpc]` / unary is
/// implemented today; streaming kinds are reserved).
///
/// # `#[init]` constructor
///
/// An optional method marked `#[init]` becomes an explicit singleton factory
/// that overrides the `#[service]` field-injection default. Its parameters are
/// `Arc<T>` dependencies, resolved from the container; it may be `async` and/or
/// return `Result<Self>`. The constructor may have any name — the macro emits a
/// fixed-name `init` associated fn that forwards to it, which also makes a
/// **second `#[init]` on the same type a compile error** (duplicate `init`).
///
/// # What it generates
///
/// - one erased handler wrapper per `#[rpc]` method, plus an `RpcGroup` appended to
///   the service's `{Service}Rpcs` slice;
/// - if an `#[init]` is present, a component factory (and the `init` marker).
///
/// # Arguments
///
/// All optional: `client_trait = Name` (emit the generated client as a trait), and
/// `rpc_slice = Ident` — the per-service slice to append to, matching the owning
/// `#[service]`'s `rpc_slice` (only needed if the default `{Service}Rpcs` name was
/// overridden there, or this block lives in another module and you renamed it).
///
/// # Example
///
/// ```ignore
/// use overseerd::prelude::*;
/// use std::sync::Arc;
///
/// #[handlers]
/// impl Greeter {
///     #[init]
///     fn new(config: Arc<Config>) -> Self { Self { config } }
///
///     #[rpc]
///     async fn greet(&self, Payload(req): Payload<GreetReq>) -> Result<GreetResp> {
///         Ok(GreetResp { message: format!("{}, {}!", self.config.greeting, req.name) })
///     }
/// }
///
/// // A second impl contributing more RPCs to the same service:
/// #[handlers]
/// impl Greeter {
///     #[rpc]
///     async fn ping() -> Result<String> { Ok("pong".into()) }
/// }
/// ```
///
/// # Errors
///
/// Emits a `compile_error!` if applied to a non-inherent-impl, if a `#[rpc]`
/// method isn't `async`, if a receiver is `&mut self`/`self`, if a `#[rpc]`
/// carries arguments, or if a streaming signature is malformed (more than one
/// `Streaming<T>`, or `Payload<T>` alongside `Streaming<T>`).
#[proc_macro_attribute]
pub fn handlers(attr: TokenStream, item: TokenStream) -> TokenStream {
    let args = parse_macro_input!(attr as attr::HandlersArgs);
    let item = parse_macro_input!(item as ItemImpl);

    handlers::expand(args, item)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Registers a component's lifecycle methods from an inherent `impl` block.
///
/// Today that is the `#[init]` constructor — an explicit factory that overrides the
/// field-injection default. Works on any component (`#[component]` or `#[service]`),
/// so a plain component gets a full-flexibility constructor (sync or async, any
/// injectable parameter — `Arc<T>`, `Cfg<T>`, `Vec<Arc<dyn Tr>>`, a by-value
/// injectable) without the async-only `factory = ..` form.
///
/// The constructor's parameters are its dependencies (resolved from the container)
/// and its return is `Self` or `Result<Self, E>`; a non-`async` constructor is
/// wrapped to async. Two `#[init]`s on one type is a compile error.
///
/// # Arguments
///
/// Optional `factory_slice = Ident` — the per-type factory slice to append to,
/// matching the owning `#[component]`/`#[service]`'s `factory_slice` when overridden
/// (defaults to `{Type}Factories`).
///
/// ```ignore
/// #[methods]
/// impl Greeter {
///     #[init]
///     async fn new(config: Arc<Config>) -> Result<Self> { Ok(Self { config }) }
/// }
/// ```
#[proc_macro_attribute]
pub fn methods(attr: TokenStream, item: TokenStream) -> TokenStream {
    let args = parse_macro_input!(attr as attr::MethodsArgs);
    let item = parse_macro_input!(item as ItemImpl);

    methods::expand(args, item)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Marks a method inside a `#[handlers]` impl as an RPC.
///
/// This is a **marker** consumed (and stripped) by `#[handlers]`; it performs no
/// expansion of its own when nested. Used on its own — outside a `#[handlers]`
/// block — it emits a `compile_error!`, since there is no `Self` context to tie
/// the RPC to a service.
///
/// See `#[handlers]` for the rules a `#[rpc]` method must satisfy (async,
/// `Result` return, extractor parameters, optional `&self`) and the accepted
/// arguments.
#[proc_macro_attribute]
pub fn rpc(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let item = parse_macro_input!(item as ItemFn);

    rpc::expand_standalone(item)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Assembles a daemon and validates it from one declaration.
///
/// ```ignore
/// // Built earlier in `main`, so their values can also configure the transport.
/// let config = ConfigManager::<Toml>::load_in(&dirs.dir(), &[])?;
///
/// let daemon = daemon! {
///     name: "example-daemon",
///     services: [Notifications, Echo],
///     configs: [ DbConfig => "app.db.reader", DbConfig => "app.db.writer" ],
///     managers: {
///         config: config,        // hand in a pre-built `ConfigManager`
///         directories: dirs,     // hand in a pre-built `DirectoriesManager`
///     },
/// }
/// .build()
/// .await?;
/// ```
///
/// Expands to a `DaemonBuilder` — `Daemon::builder(name).auto_discover()`, a
/// `with_component(..)` for each listed instance, a `config::<T>(path)` for each
/// `configs` entry (`Type => "property.path"`), and `config_source`/`directories`
/// for any `managers` entries — so it is what you use in `main`.
///
/// - `configs` binds the same config type at several property paths; a type with a
///   baked-in `#[config(path = "..")]` auto-registers and needs no entry.
/// - `managers` hands in instances built earlier in `main`: `config: <binding>` a
///   [`ConfigManager`](overseerd_core::ConfigManager), `directories: <binding>` a
///   [`DirectoriesManager`](overseerd_core::DirectoriesManager). Both are optional —
///   omitted, the builder constructs defaults (config loaded from the `Dir<Config>`
///   directory, directories derived from the daemon name).
///
/// The listed `services` are additionally required to be
/// [`Wired`](overseerd_core::Wired) under the `di-check` feature, asserting their
/// whole dependency graph (including trait-object and `#[service]` field
/// dependencies, across crates) at compile time. The same declaration that wires the
/// daemon validates it — there is no separate list to maintain.
#[proc_macro]
pub fn daemon(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as daemon::DaemonInput);

    daemon::expand(input).into()
}

/// Marks a trait as injectable as `Arc<dyn Trait>` (providers register with
/// `#[component(provide = dyn Trait)]`).
///
/// Under the `di-check` feature it emits `impl Provide<dyn Trait> for Wiring` so
/// a single `Arc<dyn Trait>` dependency type-checks; the trait must be `Send +
/// Sync` (state it as a supertrait) and object-safe. Without `di-check` the trait
/// passes through unchanged.
#[proc_macro_attribute]
pub fn injectable(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let item = parse_macro_input!(item as syn::ItemTrait);

    injectable::expand(item).into()
}
