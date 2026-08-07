# App CLI And Tooling Finalization

Issues: #150, #151, #147, #186, #148
Parent epics: #149, #141
Tracking PR: #161

## Goal

Finish the generated application CLI surface and establish the complete target-local tooling boundary
in one staged pull request. The change preserves current generated behavior by default, makes
framework-reserved CLI elements explicitly customizable, exposes deterministic prepared application
metadata, generates the private tooling entry consumed by later Cargo probe execution, and completes
the application migration and documentation.

Cargo package and binary selection and external probe execution remain in #152. User-facing
`cargo upwell` commands remain in #153-#157 and #156.

## Dependency Boundary

The generated tooling entry in #147 cannot produce its required versioned inspection document until
#150 defines that document and #151 projects prepared state into it. This PR therefore delivers the
narrow vertical contract in dependency order:

1. #186 finalizes framework-reserved bootstrap and `serve` policy and its static metadata.
2. #150 defines the versioned protocol-neutral schema without depending on application runtime,
   Clap, Cargo metadata, RPC, or Axum.
3. #151 projects immutable prepared application state into the schema without serializing executable
   payloads or process-local identities.
4. #147 generates a target-local private entry that runs the real application preparation path in
   tooling mode and returns the projected document.
5. #148 migrates examples and documentation against the finalized API and removes expression-form
   `app!`.

## Framework CLI Customization

The `app!` declaration gains an optional `cli` policy block. Omission preserves the current parser:

- `--config`, `--profile`, `--log`, `--log-format`, and `--color` remain enabled with their current
  spelling and metadata;
- a declared serve phase exposes `serve` and selects it when no subcommand is supplied;
- help and version remain framework-owned.

Each configurable element has a canonical stable identity independent of its display name.
Bootstrap arguments use generated Clap field IDs (`config`, `profiles`, `log`, `log_format`, and
`color`) directly, while the generated framework command uses canonical ID `serve`. An
application can disable a bootstrap option, customize its supported Clap metadata, and provide one
exact Clap string or typed default setting. The generated parser remains typed and contains exactly
one top-level subcommand field. No API accepts arbitrary mutation of the completed Clap tree.

Values declared as `default_value`, `default_values`, `default_value_t`, or `default_values_t` are
emitted unchanged as real Clap defaults and displayed in generated help. Clap's typed forms
stringify through `Display` or `ValueEnum` and parse through the field's value parser; they are typed
expressions at declaration, not an unparsed bypass. Generated bootstrap captures each field's
`clap::parser::ValueSource` before typed extraction, so a `DefaultValue` remains distinguishable from
an explicit `CommandLine` value.
Effective precedence is:

```text
explicit CLI
  -> environment
  -> profile configuration
  -> base configuration
  -> parser default
  -> platform/framework default
```

For config location and profile selection, which are needed to load configuration, the application
default is evaluated after the corresponding environment source and before platform/framework
fallback. Disabling an option removes only that CLI source; it does not disable environment,
platform, or generated config loading.

Generated `serve` is independent from other commands. It may be disabled or renamed and may opt out
of no-subcommand dispatch. A serve slot cannot be enabled unless the application has a serve phase.
Disabling the CLI slot does not remove direct typestate lifecycle APIs.

## Tooling Schema And Projection

The versioned document uses stable string identities and deterministic ordering. It contains:

- schema/framework version and application/package/binary/source identity;
- selected protocol and prepared scope topology;
- generic component, provider, config-binding, hook, and lifecycle resources;
- effective plugins, replacement/suppression decisions, attributed contributions, and CLI provider
  provenance;
- effective typed parser metadata with framework/application/plugin ownership and CLI provider
  provenance;
- typed diagnostics and namespaced opaque facets.

The generated framework-slot definition exists only while the parser is composed. It is neither
public application state nor a standalone tooling resource. Projection is read-only over the exact
effective typed parser metadata retained in the prepared plugin plan. It must not
rerun discovery or contribution lowering. The document excludes raw config values, secrets,
`TypeId`, function addresses, closures, factories, and other executable or process-local state.

The schema has an explicit major version and additive compatibility rule. Canonical JSON fixtures,
round trips, and declaration-permutation tests cover deterministic output.

## Generated Tooling Entry

Each named application compiled with tooling support receives one private target-local entry. The
normal generated host runner recognizes only the private probe invocation contract and delegates to
that entry before normal Clap parsing. The entry:

1. resolves the same retained pre-parse plugin catalog as normal execution;
2. runs setup, configure, before-build, config resolution, validation, and preparation with
   `ExecutionMode::Tooling`;
3. projects the resulting immutable prepared application into the versioned document;
4. serializes a structured success or typed failure envelope for #152.

It does not construct ordinary components or protocol runtime state, run startup hooks, bind
listeners, start transports or config watchers, or serve. Application callbacks receive tooling mode
and remain responsible for avoiding their own unrelated side effects.

Application discovery remains Cargo metadata package and binary-target selection. No link-time app
callback registry is added. #152 owns building and invoking the selected target, ambiguity errors,
feature/target selection, artifact isolation, and process stderr handling.

## Migration

The daemon example becomes the Homeledger-shaped generated-runner example and demonstrates:

- reserved bootstrap and serve customization;
- setup, pre-build, built, and after-build lifecycle placement;
- static plugin selection and optional plugin CLI providers;
- config/profile/logging overrides and typed command contexts;
- generated `run`, `run_with`, and direct builder/user-owned-main escape hatches.

Focused tests and examples that need direct builder mutation migrate to
`App::<Protocol>::builder(name)`. Named applications are used where generated lifecycle or CLI APIs
are the subject. The temporary expression form and deprecated expression alias are removed rather
than retained through compatibility parsing.

The migration guide documents named syntax, lifecycle typestate, framework CLI customization, plugin CLI
composition, static plugin directives, tooling mode, and manual-main boundaries. Generated help has
stable normalized coverage.

## Implementation Stages

### Stage 1: Framework CLI customization

- Add parser/model diagnostics and default-preserving policy values.
- Generate app-specific typed bootstrap arguments and semantic conversion.
- Separate serve availability, visibility, and default-command selection.
- Apply application defaults at the correct bootstrap precedence points.
- Emit a minimal hidden framework-slot definition directly into parser composition.
- Extend ownership/collision validation so renamed framework slots and `serve` retain framework
  provenance.

### Stage 2: Schema and projection

- Add the protocol-neutral schema crate and JSON compatibility tests.
- Add read-only metadata accessors only where the immutable prepared plan does not already expose
  safe information.
- Retain only effective typed parser and pre-parse CLI provider metadata through preparation.
- Implement deterministic prepared-state projection and redaction.
- Add direct and facade exports behind a tooling feature independent of CLI.

### Stage 3: Private entry

- Generate source/package/target identity beside each named application.
- Add the private probe envelope and target-local runner dispatch.
- Prove tooling preparation starts no runtime resources and constructs no ordinary components.
- Cover CLI-disabled plus tooling-enabled builds.

### Stage 4: Migration and docs

- Migrate all expression-form call sites.
- Remove legacy parser/codegen/tests and deprecated expression alias behavior.
- Complete the realistic daemon example and normalized generated-help coverage.
- Update crate/root documentation and add the migration guide.

## Validation

- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets --all-features`
- `cargo nextest run --workspace --all-features`
- `cargo test --doc --workspace --all-features`
- `cargo check --workspace --no-default-features`
- direct CLI-disabled/tooling-enabled app, macro, and facade checks
- daemon, HTTP, Jobs, RPC, Axum, third-party protocol/plugin, Windows, and relevant Wasm checks
- CI and automated Kilo review with no unresolved findings

## Exclusions

- Cargo metadata discovery and external probe process execution (#152);
- doctor/check, inspect rendering, graph/explain, init, and protocol renderer commands (#153-#157,
  #156);
- a global callback registry or source-code parsing for application discovery;
- async plugin construction or mutable post-freeze registrations;
- raw mutation of a completed Clap command tree;
- source-compatibility wrappers for expression-form `app!`.
