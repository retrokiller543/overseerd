# 1.0 App And Tooling Roadmap

## Objective

Coordinate epics `#141` and `#149` without implementing them as one change. Land one reviewable issue PR at a time behind a shared integration branch, with a new focused plan written before each wave begins.

## Branch Model

1. Create `feat/141-149-app-cli-tooling` from `release/1.0.0` before implementation.
2. Create one branch per issue from the latest integration branch, for example `feat/142-app-definition`.
3. Open each issue PR into `feat/141-149-app-cli-tooling`.
4. Merge only after that issue's focused tests and workspace checks pass, then rebase/update the next issue branch from the integration branch.
5. Open one final PR from the integration branch into `release/1.0.0` after both epics are complete.
6. If an issue cannot fit in a reviewable PR, stop and split the GitHub issue before coding. Do not silently turn it into a large mixed PR.
7. Prefix every commit that changes/removes public syntax or API with exactly `BREAKING CHANGE: `. Use `mise exec -- git ...` for commits.

## Fixed Decisions

- Replace expression-oriented `app!`; low-level users migrate to `App::<P>::builder(...)`.
- New syntax starts with `app! { pub app Name { ... } }`.
- Lifecycle entries support:
  - declarative settings: `setup: { ... }`;
  - external function: `setup = setup_fn`;
  - inline function: `setup(options) { ... }`.
- Framework setup handles directories, config/profiles, CLI/environment overrides, and tracing.
- Precedence is `CLI > environment > profile config > base config > defaults`.
- Generated CLI is Clap-based, behind a default-on `cli` feature. No CLI abstraction layer.
- One app supports one `ProtocolPlugin`.
- `cargo overseerd init` initially uses `()` as a compiling protocol-neutral placeholder; it does not guess protocol dependencies.
- The generated host runner handles normal CLI and private tooling invocation.
- Tooling must stop before component construction, hooks, watchers, protocol serving, or listener binding.
- `#141` and `#149` share the integration branch because `#147`, `#150`, `#151`, and `#152` form one tooling contract.

## Delivery Waves

Only Wave 1 is implementation-ready in this plan. Before beginning each later wave, inspect the merged code and write a separate focused plan for that wave.

### Wave 1: App Definition Foundation

PR 1, issue `#142`: named app parser and generated host shell.

PR 2, issue `#143`: lifecycle runner and non-constructing prepare boundary.

Checkpoint: named apps can be defined, built through explicit phases, and prepared/validated without constructing components. No generated CLI or cargo tool yet.

### Wave 2: Generated Application CLI

Issues in order: `#144`, `#145`, `#146`.

Checkpoint: default serve dispatch, bootstrap options, typed phase-aware commands, and Clap-native plugin extensions work through `run()`/`run_with()`.

### Wave 3: Tooling Vertical Slice

Issues in order: `#150`, `#151`, `#147`, `#152`.

Checkpoint: `cargo overseerd` can select a package/target, run its generated host in private tooling mode, and receive a deterministic protocol-neutral document without constructing or serving the app.

### Wave 4: Cargo Tool Commands

Issues in order: `#153`, `#154`, `#155`, `#157`, `#156`.

Checkpoint: doctor/check, inspect, graph/explain, protocol facets, and protocol-neutral project initialization are complete. Re-evaluate ordering of `#157` before this wave; move it earlier only if the Wave 3 document cannot represent unknown facets without it.

### Wave 5: Migration And Release Polish

Issue `#148` plus final cross-epic compatibility and documentation work.

Checkpoint: examples use the named app, old expression-macro call sites are migrated, escape hatches are documented, and the integration branch is ready for the release PR.

## Wave 1 Implementation Plan

### PR 1: `#142` Named App Definition

Branch: `feat/142-app-definition`

1. Add parser tests first in the macro-core sibling test layout for:
   - minimal named app;
   - visibility and host name;
   - declarative, external-path, and inline phase forms;
   - duplicate/unknown keys;
   - missing protocol/name and malformed phase signatures.
2. Refactor `crates/macros-core/src/app.rs` into focused parse/model/expand modules only as needed; avoid unrelated macro cleanup.
3. Generate a named host type and thin phase adapter functions. Keep orchestration in runtime traits rather than generating a large state machine directly in tokens.
4. Expose a builder escape hatch from the host so custom startup remains possible.
5. Remove expression-oriented `app!` parsing and update its macro documentation. This is the breaking commit.
6. Migrate only call sites needed to keep the workspace compiling:
   - use direct `App::builder` in low-level tests;
   - use a minimal named app where host reuse is actually exercised.
   Full example/migration documentation remains `#148`.
7. Do not add Clap, commands, tooling JSON, or cargo tooling in this PR.

PR 1 validation:

```text
cargo fmt --all -- --check
cargo test -p overseerd-macros-core
cargo test -p overseerd-macros
cargo check --workspace --all-features
```

Acceptance:

- Invalid syntax reports span-accurate compile errors.
- A named host deterministically creates the same protocol-specific builder on repeated calls.
- The workspace has no expression-style `app!` calls.
- Direct `App::builder` remains the documented low-level path.

### PR 2: `#143` Lifecycle And Prepare Boundary

Branch: `feat/143-app-lifecycle`

1. Add public, documented runtime types in `overseerd-app`:
   - execution mode (`Run` or `Tooling`);
   - lifecycle phase;
   - typed phase error preserving the source;
   - bootstrap, configured, prepared, and built states;
   - host traits implemented by generated adapters.
2. Split current `AppBuilder::build` into two consuming operations:
   - `prepare`: registration, config binding/default preparation, registry/scope validation, and protocol pre-build validation;
   - `PreparedApp::build`: component construction, runtime/hook attachment, and protocol construction.
3. Add a mutable protocol pre-build context for registering components, prebuilt instances, and config paths before app validation, followed by a read-only finalized validation context. Keep router/runtime construction in the build phase.
4. Implement `Plugin`, `ProtocolPlugin`, and `Protocol` for `()` for host tests and the later init scaffold.
5. Wire the generated host through `setup -> configure -> before_build -> prepare -> build -> after_build -> serve`, but implement only builder/phase APIs in this wave; CLI dispatch waits for Wave 2.
6. Add tests with counters/panicking factories proving:
   - exact phase order;
   - setup/configured commands can stop early through the host API;
   - tooling/prepare does not invoke factories, hooks, watchers, protocol build, bind, or serve;
   - phase errors identify the failed phase and retain their typed source.
7. Keep arbitrary user setup side effects explicitly outside the framework's safety guarantee; execution mode is supplied so compliant setup code can avoid run-only behavior.

PR 2 validation:

```text
cargo fmt --all -- --check
cargo test -p overseerd-app
cargo test -p overseerd-rpc
cargo test -p overseerd-axum
cargo clippy --workspace --all-targets --all-features
cargo test --workspace --all-features
```

Acceptance:

- Normal build behavior remains unchanged after `prepare().build()`.
- Tooling preparation validates the real configured registry without constructing components.
- RPC and Axum validate against finalized preparation state without building their runtime protocol objects.
- Later CLI and tooling work can consume the host traits without changing `App` to depend on Clap or process arguments.

## Later-Wave Contract Boundaries

These are constraints, not detailed implementation tasks:

- Wave 2 owns Clap dependencies, generated `Cli`, bootstrap flags, default serve, commands, and plugin CLI extensions.
- Wave 3 owns the shared versioned document, generic inspection conversion, private runner protocol, and cargo target execution.
- Wave 4 owns user-facing cargo commands/renderers/facets/scaffolding.
- Wave 5 owns comprehensive examples and migration guidance.
- Do not pull later-wave features into an earlier PR merely to demonstrate future architecture; use narrow traits and test fakes.

## Global Guardrails

- Keep the epic roadmap and active wave plan committed on the epic branch until its release PR merges. Preserve unrelated `.kilo/` content and `docs/plans/data-validation.md`.
- Use the existing registration-backend abstraction; do not add a new callback registry or another registration dependency.
- Mark new externally extensible public structs/enums `#[non_exhaustive]` where appropriate and provide constructors/builders.
- Keep tests out of implementation files according to repository test layout rules.
- Do not add `openssl` or `rsa` to the dependency tree.
- Before every issue PR merge, run formatting and the CI clippy invocation from `AGENTS.md`.

## Replanning Checkpoints

After each wave:

1. Compare merged behavior to the next issues' acceptance criteria.
2. Inspect current public APIs and dependency graph.
3. Split oversized issues before implementation.
4. Write a new focused plan containing only the next wave's file-level changes, tests, migration impact, and PR order.
