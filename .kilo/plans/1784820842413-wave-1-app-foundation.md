# Wave 1: Named App And Lifecycle Foundation

## Outcome

Land two independently reviewable PRs:

1. `#142` introduces a reusable named app definition that deterministically creates the existing protocol-specific builder.
2. `#143` adds executable lifecycle phases and splits app preparation/validation from component construction.

Wave 1 deliberately excludes Clap, commands, tooling JSON/probes, and comprehensive migration docs.

## Branch And Delivery Sequence

Implementation begins with branch setup, before source edits:

```text
git switch release/1.0.0
git pull --ff-only
git switch -c feat/141-149-app-cli-tooling
git push -u origin feat/141-149-app-cli-tooling
git switch -c feat/142-app-definition
```

- Commit this plan and the epic roadmap on the epic branch until its release PR merges. Preserve unrelated `.kilo/` content and `docs/plans/data-validation.md`.
- PR `#142` targets `feat/141-149-app-cli-tooling`.
- After `#142` merges, update the local integration branch, then create `feat/143-app-lifecycle` from it.
- PR `#143` also targets `feat/141-149-app-cli-tooling`.
- Do not merge issue branches directly into `release/1.0.0`.
- Use `mise exec -- git ...` for commits.
- Prefix any commit that changes an existing public API with exactly `BREAKING CHANGE: `. Wave 1 adds APIs but does not remove expression-style `app!`, so its normal implementation commits need no breaking prefix unless implementation discovers an unavoidable break.

## Scope Decisions

- Expression-oriented `app! { name: ..., protocol: ... }` remains temporarily available inside the integration branch to avoid migrating roughly 25 examples/tests in parser PR `#142`.
- Issue `#148` removes expression mode and migrates all call sites before the integration branch reaches `release/1.0.0`. That removal commit must use `BREAKING CHANGE: `.
- PR `#142` does not parse lifecycle callbacks before they have runtime semantics. PR `#143` adds callback grammar and execution together.
- One app has exactly one `ProtocolPlugin`.
- `App` remains independent of process arguments and Clap.
- Preparation may create framework-owned planning/config objects, but must not invoke user component factories, construct the root container, attach/run hooks, build protocol routers, spawn reload tasks, bind listeners, or serve.
- Arbitrary Rust inside user lifecycle callbacks cannot be sandboxed. The framework supplies `ExecutionMode`; user callbacks are responsible for avoiding run-only side effects in tooling mode.

## PR 1: `#142` Named Definition And Builder Host

### Stable Syntax Added In This PR

Use a named wrapper around the existing declarative assembly fields:

```rust
overseerd::app! {
    pub app Homeledger {
        name: "homeledger",
        protocol: overseerd::daemon::RpcPlugin,
        services: [Notifications],
        components: [clock],
        configs: [DbConfig => "app.db"],
        managers: {
            directories: dirs,
            config: config,
        },
        middleware: [trace_layer],
        guards: [auth_guard],
        error_handler: handle_error,
    }
}
```

The generated surface is intentionally small:

```rust
pub struct Homeledger;

impl Homeledger {
    pub fn builder() -> overseerd::AppBuilder<RpcPlugin>;
}
```

- `builder()` reevaluates declaration expressions on each call; it does not cache state or create components.
- Visibility on `app` is applied to the host type.
- Runtime app `name` and `protocol` remain required.
- Existing `overseerd = ...` and `crate = ...` path overrides remain accepted in the declaration body.
- No `run`, `serve`, callback, command, CLI, or tooling methods are generated yet.

### Parser And Expansion Work

1. Change the macro entry model from a single legacy `AppInput` to an enum that dispatches by leading tokens:
   - named definition when input starts with optional visibility followed by `app`;
   - existing expression form otherwise.
2. Reuse one assembly model and one builder-expression generator for both forms. Do not fork the current manager/config/component expansion.
3. Separate only where it improves reviewability:
   - `crates/macros-core/src/app.rs`: entry enum, shared expansion entry, module declarations;
   - `crates/macros-core/src/app/parse.rs`: named/legacy parsing and duplicate-key helpers;
   - `crates/macros-core/src/app/expand.rs`: shared builder expression plus named host wrapper;
   - `crates/macros-core/src/app/tests.rs`: parser and token-expansion tests.
4. Detect duplicate top-level keys for the named form. The current legacy parser silently overwrites several repeated keys; do not expand this PR into changing legacy diagnostics.
5. Preserve span-specific errors for unknown keys, missing name/protocol, malformed manager blocks, and duplicate named keys.
6. Update `crates/macros/src/lib.rs` macro documentation with one named example and mark expression mode as temporary/deprecated for 1.0 migration. Do not remove the `daemon!` alias in this PR.
7. Re-export no new runtime trait for PR 1; the generated inherent `builder()` is sufficient and avoids coupling parser work to `#143`.

### PR 1 Tests

Parser tests:

- private, `pub`, and restricted visibility hosts;
- required `app` keyword and host identifier;
- complete and minimal assembly bodies;
- both crate path overrides;
- duplicate `name`, `protocol`, list, manager, and singleton keys;
- missing runtime name or protocol;
- unknown key and malformed nested manager settings;
- legacy expression input still dispatches through the shared builder generator.

Expansion tests:

- generated host has the requested visibility/name;
- `builder()` returns `AppBuilder<DeclaredProtocol>`;
- builder body includes each declared registration/configuration exactly once;
- generated identifiers are scoped inside the method and two named apps in one module do not collide.

Workspace integration test:

- add one small named test protocol app in an existing integration-test target;
- call `Host::builder()` twice and prove both builders are independent without building components;
- leave existing expression app call sites unchanged.

### PR 1 Validation

```text
cargo fmt --all -- --check
cargo test -p overseerd-macros-core
cargo test -p overseerd-macros
cargo check --workspace --all-features
cargo clippy --workspace --all-targets --all-features
```

### PR 1 Completion Gate

- Named definitions compile and produce reusable builders.
- Existing expression users still compile during integration development.
- No runtime lifecycle types, Clap dependency, or tooling model has been introduced.
- Open PR `#142` into the integration branch and stop until it is merged.

## PR 2: `#143` Lifecycle Runner And Prepare Boundary

### Lifecycle Callback Contract

Extend only named definitions with executable callback forms:

```rust
app! {
    pub app Homeledger {
        // Existing declaration fields...

        configure = configure_app,

        before_build(context, builder) {
            Ok(builder)
        },

        after_build(context, app) {
            Ok(app)
        },

        serve = serve_app,
    }
}
```

Wave 1 forms:

- `phase = path`: call an external async function with the phase signature.
- `phase(arguments...) { ... }`: generate an inline async function; argument names are user-selected but types are fixed by the phase.
- Reject multiple forms for one phase and unknown declarative settings at macro expansion.

The settled `setup: { ... }` declarative form is added in Wave 2 alongside its real config,
profile, logging, and color settings. Wave 1 must not accept a placeholder settings grammar that
would immediately change in the next wave.

Wave 1 signatures:

```rust
setup(mode: ExecutionMode) -> Result<BootstrapContext, E>
configure(context: &mut BootstrapContext, builder: AppBuilder<P>)
    -> Result<AppBuilder<P>, E>
before_build(context: &mut BootstrapContext, builder: AppBuilder<P>)
    -> Result<AppBuilder<P>, E>
after_build(context: &mut BootstrapContext, app: App<P>)
    -> Result<App<P>, E>
serve(context: BootstrapContext, app: App<P>) -> Result<(), E>
```

- External lifecycle functions are async in Wave 1. Supporting sync function paths would require a
  separate adapter contract because a macro cannot inspect a path's return type; do not add that
  abstraction speculatively.
- All phase errors must implement `std::error::Error + Send + Sync + 'static`.
- Generated adapters map them into `PhaseError` while preserving the source chain.
- Omitted setup creates `BootstrapContext::new(mode)`.
- Omitted configure/before-build/after-build are identity operations.
- Omitted serve is not callable in Wave 1; the generated host exposes build/prepare APIs, and default protocol serving arrives with Wave 2 CLI. If `serve` is declared, expose `serve_with(context, app)` for direct testing/custom main.

### Runtime Types

Add a focused `crates/app/src/host.rs` module with sibling `host/tests.rs`:

- `#[non_exhaustive] ExecutionMode { Run, Tooling }` with query methods.
- `#[non_exhaustive] LifecyclePhase { Setup, Configure, BeforeBuild, Prepare, Build, AfterBuild, Serve }`.
- `BootstrapContext`, constructed through methods rather than public field literals. In Wave 1 it stores execution mode and a typed extension map for setup-produced shared values; Wave 2 adds standard CLI/config bootstrap values without changing callback signatures.
- `PhaseError` containing `LifecyclePhase` and boxed typed source, with `thiserror` display/source implementation.
- Public `AppHost` trait containing associated `Protocol` and generated async lifecycle entry points needed by tests, embedding, and later tooling. Use stable Rust async trait patterns already accepted by the workspace; do not add `async-trait` unless object safety is concretely required.
- Inherent generated methods delegate to the trait so normal users need not import it.

Generated methods after PR 2:

```text
Host::builder()
Host::setup(mode)
Host::configure(context)
Host::prepare(mode)
Host::build(mode)
Host::serve_with(context, app)   // only when serve is declared
```

`prepare(mode)` runs through setup/configure/before-build and returns both the context and `PreparedApp<P>`. `build(mode)` continues through construction and after-build, returning context plus `App<P>`. Neither method parses process arguments.

### Split `AppBuilder::build`

Introduce `PreparedApp<P>` in `crates/app/src/app.rs` (or a focused sibling module if extraction makes review easier). Keep one ownership path so normal build and tooling cannot drift.

`AppBuilder::prepare()` performs, in current build order:

1. Collect and merge explicit/auto-discovered descriptors.
2. Let the protocol plugin register descriptor/config seeds.
3. Resolve directories and create framework-owned seed instances without invoking component factories.
4. Finalize the config manager, bindings, defaults, and reload metadata without spawning triggers.
5. Validate the app registry and resolve the effective component/factory set.
6. Collect hook descriptors without attaching or running hooks.
7. Compute scope plans/construction orders.
8. Invoke protocol validation over the finalized registry and config store.
9. Return `PreparedApp<P>` owning every value needed for construction.

`PreparedApp::build()` performs:

1. Build config store/reloader framework state if not already prepared.
2. Invoke `ScopeContainer::build_root`, the first point at which user factories may execute.
3. Attach hook manager and root resolver without running startup hooks.
4. Create `AppRuntime`.
5. Invoke `ProtocolPlugin::build` to construct the protocol/router.
6. Return the existing `App<P>`.

Keep `AppBuilder::build()` as a convenience delegating to `self.prepare().and_then(build)` so existing direct-builder behavior remains source-compatible in Wave 1.

### Protocol Validation Seam

Extend `ProtocolPlugin` with two preparation-safe methods: mutable pre-build contribution before finalization, then read-only validation over finalized registry/config state. Pre-build supports registering component descriptors, prebuilt instances, and config paths; neither context exposes runtime containers.

- RPC moves service/path validation from `RpcPlugin::build` into finalized validation; router construction stays in `build`.
- Axum moves temporary configuration/path checks that do not require `AppRuntime` into finalized validation; router/controller instance capture and WebSocket endpoint construction stay in `build`. A future generic extraction-time validation epic replaces protocol-specific config guards.
- A default implementation preserves third-party protocol source compatibility for this additive Wave 1 change.

### Protocol-Neutral Placeholder

Implement the protocol traits directly for `()` in `overseerd-app`:

- no scopes, descriptors, runtime resources, or serving behavior;
- protocol `build` succeeds without touching components;
- intended for host tests and the later `cargo overseerd init` placeholder;
- do not add a fake long-running `Serve` implementation in Wave 1.

### PR 2 Tests

Host/parser tests:

- omitted phases use defaults;
- external-path and inline forms parse and execute;
- duplicate phase forms and malformed argument counts fail with useful spans;
- async external callbacks;
- inline callbacks can use `?` and shared typed bootstrap extensions;
- two hosts keep generated adapter names isolated.

Lifecycle order tests:

- record setup, configure, before-build, prepare, build, after-build, and explicit serve order;
- failure at every phase prevents all later phases;
- `PhaseError::phase()` and `source()` expose the correct failure.

Preparation safety tests use panic/counters to prove `prepare(Tooling)` does not:

- call a component factory;
- build root/scoped containers;
- attach or run startup/shutdown hooks;
- spawn config watcher/SIGHUP tasks;
- call `ProtocolPlugin::build`;
- construct RPC/Axum routers or WebSocket endpoints;
- bind or serve a listener.

Behavior preservation tests:

- `AppBuilder::build()` and `AppBuilder::prepare().await?.build().await` produce equivalent registries/protocol behavior;
- existing daemon, HTTP, config-trigger, middleware, and WebSocket tests remain green;
- RPC duplicate service/path errors now arise during prepare;
- Axum preparation catches preparation-safe config errors before runtime construction.

### PR 2 Validation

```text
cargo fmt --all -- --check
cargo test -p overseerd-app
cargo test -p overseerd-macros-core
cargo test -p overseerd-rpc
cargo test -p overseerd-axum --all-features
cargo test --workspace --all-features
cargo clippy --workspace --all-targets --all-features
cargo check --workspace --no-default-features
```

Also verify dependency policy:

```text
cargo tree --workspace --all-features -i openssl
cargo tree --workspace --all-features -i rsa
```

Both commands must show no newly introduced prohibited dependency path.

### PR 2 Completion Gate

- Named hosts execute lifecycle phases without process/Clap coupling.
- `prepare(Tooling)` validates the real configured application without user component construction or runtime side effects.
- Existing `AppBuilder::build()` behavior remains available.
- Protocol validation needed by tooling no longer depends on `AppRuntime` construction.
- Open PR `#143` into the integration branch and stop after merge; Wave 2 receives a new plan based on the merged API.

## File Boundary Summary

Expected PR 1 files:

```text
crates/macros-core/src/app.rs
crates/macros-core/src/app/parse.rs
crates/macros-core/src/app/expand.rs
crates/macros-core/src/app/tests.rs
crates/macros/src/lib.rs
one focused integration test target
```

Expected PR 2 files:

```text
crates/app/src/app.rs
crates/app/src/host.rs
crates/app/src/host/tests.rs
crates/app/src/protocol.rs
crates/app/src/lib.rs
crates/macros-core/src/app/{parse,expand,tests}.rs
crates/rpc/src/plugin.rs
crates/axum/src/plugin.rs
focused RPC/Axum/app integration tests
facade re-exports in src/lib.rs
```

Avoid unrelated formatting/refactors and do not modify cargo-tooling or schema crates in Wave 1.
