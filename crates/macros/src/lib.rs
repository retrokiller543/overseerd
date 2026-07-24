//! Procedural macros for the Overseerd framework.
//!
//! These are re-exported from the `overseerd` facade crate; depend on that rather
//! than this crate directly. They span three subsystems — components (dependency
//! injection), services (RPC), and configuration.
//!
//! | Macro                       | Applies to | Produces |
//! |-----------------------------|------------|----------|
//! | `#[component]`              | struct     | `Component` impl + a factory (field-injection default, `factory = path`, or `default_factory = false` for a manual instance) |
//! | `#[service]`                | struct     | `Component` + `ServiceComponent` impls + a service header + a factory |
//! | `#[handlers]`               | impl block | RPC handlers + RPC group |
//! | `#[methods]`                | impl block | lifecycle methods — an `#[init]` constructor (an explicit factory) |
//! | `#[rpc]` / `#[init]`        | method     | markers consumed by `#[handlers]` / `#[methods]` |
//! | `#[injectable]`             | trait      | `Provide<dyn Trait>` impl (under `di-check`) |
//! | `#[config]`                 | struct/enum | `ConfigProperties` impl (with field `#[default = ".."]` templated defaults); auto-registers a binding when given `#[config(path = "..")]` |
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
//!    `AppBuilder::with_component`. Annotate the type
//!    `#[component(default_factory = false)]`, which emits the `Component` metadata
//!    with no factory.
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
//! Each `#[proc_macro_*]` entry point here is a thin shim: it forwards its token streams to
//! the matching `expand` function in [`overseerd_macros_core`], the ordinary library that
//! holds all the parsing and codegen (a proc-macro crate can only export proc-macros, so the
//! reusable machinery lives there). Errors are surfaced as `compile_error!` by the core, not
//! by panicking.

extern crate proc_macro;

use proc_macro::TokenStream;

/// Declares a **system-constructed singleton component** on a struct.
///
/// The container builds the instance during startup by *field injection*: each
/// field is resolved from the container as a dependency (its type is an injectable
/// handle — `Arc<T>`, `Cfg<T>` for config, a trait-object collection, …), unless it
/// carries `#[default]`, which makes it owned state built with `Default::default()`.
/// Use this for dependencies the system can assemble itself (pools, clients composed
/// from other components, …). For an instance you must build yourself, use
/// `#[component(default_factory = false)]` and provide it via
/// `AppBuilder::with_component`.
///
/// # Arguments
///
/// All optional:
/// - `id` — unique component id. Defaults to the lowercased type name.
/// - `name` — display name. Defaults to the type name.
/// - `factory = path` — register `path` (an async `Factory`) as the constructor
///   instead of field injection; its parameters are its dependencies.
/// - `default_factory = false` — emit no factory (a **manual** instance, provided
///   via `AppBuilder::with_component`).
/// - `factory_slice = Ident` — override the generated `{Type}Factories` slice name.
/// - `priority = <const i64 expression>` — collection-provider order among providers whose
///   `before` / `after` constraints are satisfied; lower values run first. Constants and
///   associated constants are accepted.
/// - `before` / `after` — relative ordering constraints for traits shared by both providers;
///   use `as dyn Trait` to require and restrict the relationship to a specific trait.
///
/// ```ignore
/// #[component]                          // id = "dbpool", name = "DbPool"
/// #[component(id = "db", name = "Db")]  // explicit
/// #[component(factory = Db::connect)]   // explicit async factory
/// #[component(default_factory = false)] // manual, via with_component
/// ```
///
/// # What it generates
///
/// - `impl Component for T` (carrying `ID`/`NAME`);
/// - a `ComponentDescriptor` registered into the `COMPONENTS` slice (picked up by
///   `auto_discover`), pointing at the type's `{Type}Factories` slice — which holds
///   the field-injection default (unless suppressed) plus any `factory =` / `#[init]`
///   entry.
///
/// # Example
///
/// ```ignore
/// use overseerd::prelude::*;
/// use std::sync::Arc;
///
/// #[component(default_factory = false)]
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
/// A component the system should *build* uses field injection by default; for one
/// you construct yourself, `#[component(default_factory = false)]` emits the
/// metadata with no factory (provide it via `AppBuilder::with_component`), and
/// `#[component(factory = path)]` registers an explicit async factory.
///
/// # See also
///
/// `#[service]` (a component that also exposes RPCs and a version) and `#[methods]`
/// (an `#[init]` constructor for any component).
#[proc_macro_attribute]
pub fn component(attr: TokenStream, item: TokenStream) -> TokenStream {
    overseerd_macros_core::component(attr.into(), item.into()).into()
}

/// Implements the `ConfigProperties` trait for a config `struct` or `enum`, making it
/// injectable as `Cfg<T>` from a property path.
///
/// The type must also derive `Deserialize`, and `#[config]` must sit *above* the derive so
/// it strips any field `#[default]` before the derive runs. With `#[config(path = "..")]`
/// the binding is auto-registered (picked up by `auto_discover`); without a path, bind it
/// explicitly with `AppBuilder::config::<T>(path)` — needed when the same type is
/// bound at several paths. `#[config(name = "..")]` overrides the display name.
///
/// A named field may carry `#[default = ".."]`: the literal is a template string merged
/// under the config before deserialization, so a missing field falls back to it and
/// resolves through the normal `${..}` pipeline (env vars, `${other.path}` refs, and the
/// `${@runtime}` directory namespace). Unlike a hand-written `Default`, the default keeps
/// full templating power. On an enum, defaults may sit on variant fields and apply only to
/// the variant present in the config, and a variant may be marked with a bare `#[default]`
/// to select it when the config names no variant.
///
/// Distinct from the **field-level** `#[config("path")]` inside a `#[component]` /
/// `#[service]` struct, which marks a `Cfg<T>` injection site (consumed by that
/// macro's expansion); this struct-level form declares the config type itself.
///
/// ```ignore
/// #[config(path = "app.server")]
/// #[derive(Deserialize)]
/// struct ServerConfig {
///     #[default = "${tcp.ip}:8080"]
///     addr: SocketAddr,
/// }
/// ```
#[proc_macro_attribute]
pub fn config(attr: TokenStream, item: TokenStream) -> TokenStream {
    overseerd_macros_core::config(attr.into(), item.into()).into()
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
    overseerd_macros_core::methods(attr.into(), item.into()).into()
}

/// Defines a reusable application host and its configured builder.
///
/// ```ignore
/// app! {
///     pub app Example {
///         name: "example-daemon",
///         protocol: overseerd::daemon::RpcPlugin,
///         services: [Notifications, Echo],
///         configs: [DbConfig => "app.db.reader", DbConfig => "app.db.writer"],
///     }
/// }
///
/// let built = Example::new(ExecutionMode::Run).build().await?;
/// let app = built.app();
/// ```
///
/// Named applications are compile-time lifecycle state machines. `Example` defaults to
/// `Example<Initial>`; consuming transitions produce `Example<Setup>`, `Example<PreBuild>`, and
/// `Example<Built>`. Each stage exposes only valid operations. Initial and intermediate stages
/// also provide explicit fast-forward transitions that still execute every skipped phase in order:
///
/// - `Initial` stores only `ExecutionMode`; no CLI state, directories, config, builder, DI
///   components, runtime, or protocol have been created.
/// - `Setup` stores `BootstrapContext`; the setup hook and tracing finalization have completed, but
///   no builder has been prepared or validated.
/// - `PreBuild` stores `BootstrapContext` and `PreparedApp`; bootstrap managers were applied,
///   configure hooks ran, directories and config were resolved, protocol/framework descriptors
///   were registered, the graph and scopes were validated, and construction plans were computed.
///   Ordinary components, the root container, runtime, and protocol have not been constructed.
/// - `Built` stores `BootstrapContext` and `App`; singleton components and the root container were
///   constructed, hooks and the root resolver were attached, the runtime and protocol were
///   finalized, and `after_build` ran. Serving and startup hooks have not started.
///
/// ```ignore
/// let setup = Example::new(ExecutionMode::Run).setup().await?;
/// let prepared = setup.prepare().await?;
/// let built = prepared.build().await?;
/// built.serve().await?;
///
/// // Explicitly runs setup, prepare, build, then serve.
/// Example::new(ExecutionMode::Run).serve().await?;
/// ```
///
/// The generated initial stage implements `AppHost`, so custom runtimes can drive the public
/// generic `setup_host`, `prepare_host`, and `build_host` utilities or consume the stage-owned
/// `BootstrapContext`, `PreparedApp`, and built `App` through `into_parts()`.
///
/// With the default `cli` feature, a named app declaring `serve` or application commands also
/// generates native Clap `ExampleCli`/`ExampleCommand` types and
/// `Example::run()`/`run_with(args)`. The application must declare a direct `clap` dependency with
/// its `derive` feature so command types can derive `clap::Args`:
///
/// ```toml
/// clap = { version = "4", features = ["derive"] }
/// ```
///
/// A thin binary can then delegate to the generated host:
///
/// ```ignore
/// #[tokio::main]
/// async fn main() -> Result<(), overseerd::CliError> {
///     Example::run().await
/// }
/// ```
///
/// Command-local arguments live on command types implementing `CliCommand<Example>`. Nested
/// command blocks generate native intermediate Clap `Subcommand` enums, while a separate `args`
/// block flattens shared argument groups into the generated parser and stores them by type in the
/// command's `BootstrapContext`. Fields that must also parse after a subcommand use Clap's
/// `#[arg(global = true)]` setting:
///
/// ```ignore
/// #[derive(clap::Args)]
/// struct MigrateCommand {
///     #[arg(long)]
///     dry_run: bool,
/// }
///
/// #[derive(clap::Args)]
/// struct OutputArgs {
///     #[arg(long, global = true)]
///     format: Option<String>,
/// }
///
/// impl CliCommand<Example> for MigrateCommand {
///     type Error = MigrationError;
///
///     fn phase(&self) -> CommandPhase {
///         CommandPhase::Built
///     }
///
///     async fn run(
///         &self,
///         context: CommandContext<Example>,
///     ) -> Result<(), Self::Error> {
///         let app = context.app().expect("built command context");
///         // Resolve migration dependencies from `app.container()`.
///         Ok(())
///     }
/// }
///
/// app! {
///     app Example {
///         name: "example-daemon",
///         protocol: RpcPlugin,
///         args: { output: OutputArgs },
///         commands: {
///             #[command(alias = "db", display_order = 10)]
///             migrate: MigrateCommand,
///             api: { users: { list: ListUsersCommand } },
///         },
///     }
/// }
/// ```
///
/// Inline `serve` phases may request typed DI dependencies after the explicit context and app
/// parameters. Each additional typed parameter is resolved from the built root container before
/// the phase body runs:
///
/// ```ignore
/// serve(context, app, server: Cfg<ServerConfig>) {
///     let transport = TcpTransport::bind((server.bind.as_str(), server.port)).await?;
///     app.serve(transport).await
/// }
/// ```
///
/// Command entries accept native non-structural `#[command(...)]` metadata such as aliases,
/// visibility, display order, help text, usage customization, and local parser behavior. Settings
/// that replace or flatten generated variants are rejected because they would bypass typed
/// dispatch. When the generated host is public, its command and argument types must be at least as
/// visible as the host because they appear in the generated public parser enums.
///
/// The generated host's fallible `builder()` creates an `AppBuilder` from the declaration:
/// `App::builder(name).auto_discover()`, a `with_component(..)` for each listed
/// instance, a `config::<T>(path)` for each `configs` entry (`Type =>
/// "property.path"`), and `config_source`/`directories` for any `managers` entries. It returns
/// a `Result` because loading a directory-backed configuration manager can fail.
///
/// - `configs` binds the same config type at several property paths; a type with a
///   baked-in `#[config(path = "..")]` auto-registers and needs no entry.
/// - `managers` hands in instances built earlier in `main`: `config: <binding>` a
///   `ConfigManager`, `directories: <binding>` a `DirectoriesManager`. Both are optional —
///   omitted, the builder constructs defaults (config loaded from the `Dir<Config>`
///   directory, directories derived from the app name).
///
/// The listed `services` are additionally required to be
/// `Wired` under the `di-check` feature, asserting their
/// whole dependency graph (including trait-object and `#[service]` field
/// dependencies, across crates) at compile time. The same declaration that wires the
/// app validates it — there is no separate list to maintain.
///
/// The expression-oriented form remains temporarily available during the 1.0 migration.
/// New applications should use a named definition; custom/local-value assembly should call
/// `App::<P>::builder(..)` directly.
#[proc_macro]
pub fn app(input: TokenStream) -> TokenStream {
    overseerd_macros_core::app(input.into()).into()
}

/// Deprecated alias for [`app!`](macro@app). Renamed in 0.7.0; removed in 1.0.0.
#[deprecated(
    since = "0.7.0",
    note = "renamed to `app!`; the `daemon!` alias is removed in 1.0.0"
)]
#[proc_macro]
pub fn daemon(input: TokenStream) -> TokenStream {
    overseerd_macros_core::app(input.into()).into()
}

/// Marks a trait as injectable as `Arc<dyn Trait>` (providers register with
/// `#[component(provide = dyn Trait)]`).
///
/// On native targets the trait also extends
/// `RuntimeDescriptor<ComponentDescriptor>`, allowing a provider's component
/// descriptor to be read through the trait object. Wasm targets retain the
/// original trait because the DI descriptor types are native-only.
///
/// Under the `di-check` feature it emits `impl Provide<dyn Trait> for Wiring` so
/// a single `Arc<dyn Trait>` dependency type-checks; the trait must be `Send +
/// Sync` (state it as a supertrait) and object-safe.
#[proc_macro_attribute]
pub fn injectable(attr: TokenStream, item: TokenStream) -> TokenStream {
    overseerd_macros_core::injectable(attr.into(), item.into()).into()
}
