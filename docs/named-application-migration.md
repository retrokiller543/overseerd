# Migrating To Named Applications

Expression-form `app!` and the `daemon!` alias have been removed. `app!` now declares a reusable
named host with compile-time lifecycle state, an optional generated CLI runner, and an optional
target-local tooling entry.

The [`app!` Rustdoc](https://docs.rs/upwell/latest/upwell/macro.app.html) is authoritative for
the full grammar, generated types and methods, lifecycle contracts, exact Clap defaults and
precedence, commands, plugins, tooling, feature behavior, and errors. This guide covers migration
choices and common source changes without duplicating that contract.

## Minimal Migration

Old expression-oriented assembly created a builder inside `main`. That syntax no longer parses.
Move the definition to item scope, give the generated host a Rust type name, declare serving, and
delegate the normal process entry to `run`:

```rust
use upwell::{daemon::prelude::*, prelude::*};

app! {
    app NotifyApplication {
        name: "notifyd",
        protocol: Rpc,
        serve(_context, app) {
            let transport = TcpTransport::bind("127.0.0.1:7000").await?;

            app.serve(transport).await
        },
    }
}

#[tokio::main]
async fn main() -> Result<(), upwell::CliError> {
    NotifyApplication::run().await
}
```

The declaration envelope is `#[doc]` attributes, Rust visibility, `app`, and the generated type
name. `app NotifyApplication` is private; `pub app NotifyApplication` generates a public host and
public generated CLI types. Only outer documentation attributes are accepted on the generated app.

`name` and `protocol` are mandatory. With the default `cli` feature, or with `tooling`, `name` must
be a string literal. With both features absent it may be an item-scope expression evaluated on every
`NotifyApplication::builder()` call, but it still cannot capture a local variable from `main`.

## Dependency Change

Every crate that expands a named app while `cli` is enabled needs a direct Clap dependency, even if
the app has no `args`, `commands`, or `serve` declaration. Generated bootstrap always derives and
names `::clap` types.

```toml
clap = { version = "4", features = ["derive"] }
```

The `upwell` facade enables `cli` by default. Disable that feature if the crate only needs direct
builder and typestate lifecycle APIs and should not generate a parser or runner.

## Move Assembly To Item Scope

The named body keeps protocol-neutral assembly keys for component instances, config bindings,
managers, plugins, lifecycle, and generated CLI declarations. `middleware`, `guards`, and
`error_handler` call extension methods supplied by the selected protocol builder; other
protocol-specific customization belongs in `configure` rather than in a nested protocol grammar.

Item-scope expressions are evaluated each time the generated `builder()` is called. They cannot
capture locals from `main`. If assembly depends on a runtime-local value, use `configure` when the
value can come from `BootstrapContext`, or use the direct builder escape hatch described below.

`services: [Type, ...]` is assertion-only: under `di-check` it requires each listed type to be
`Wired`. It does not register runtime services. Runtime service/controller registration comes from
the selected protocol's auto-discovery.

Manager entries now mean either a complete instance expression or a manager-construction block:

```rust
app! {
    app ConfiguredApplication {
        name: "configured",
        protocol: Rpc,
        managers: {
            directories: { root: std::env::temp_dir() },
            config: {
                profiles: &[String::from("development")],
                sighup: true,
                watch: true,
                debounce: std::time::Duration::from_millis(500),
            },
        },
    }
}
```

A source-less config block loads from the explicitly declared directories manager. `profiles` is
used for that load. `source` instead supplies a config-manager expression. Directory blocks accept
exactly one of `app` or `root`; combining them is rejected, as is combining config `source` with
`profiles`. Explicit manager entries are declaration-owned, so generated bootstrap does not
overwrite them. Omitted managers preserve generated-bootstrap/default builder ownership. See the
Rustdoc for exact defaults, errors, and `watch`-feature behavior.

## Lifecycle Migration

Each phase accepts either `phase = async_function` or an inline body:

```rust
app! {
    app NotifyApplication {
        name: "notifyd",
        protocol: Rpc,
        setup = setup,
        configure(context, builder) {
            Ok::<_, std::convert::Infallible>(builder)
        },
        before_build = before_build,
        after_build(context, app) {
            Ok::<_, std::convert::Infallible>(app)
        },
        serve(_context, app, server: Cfg<ServerConfig>) {
            let server = server.snapshot();
            let transport = TcpTransport::bind((server.bind.as_str(), server.port)).await?;

            app.serve(transport).await
        },
    }
}
```

The contracts are `BootstrapContext -> BootstrapContext` for setup, builder-in/builder-out for
configure and before-build, app-in/app-out for after-build, and context-plus-app to `()` for serve;
all callbacks are awaited and return `Result`. Additional typed inline-serve parameters are resolved
through built DI before the body runs.

The host defaults to `NotifyApplication<Initial>` and consumes itself through `Setup`, `PreBuild`,
and `Built`. Direct calls such as `new(ExecutionMode::Run).build().await` still execute every
intermediate phase. `PreBuild` owns a validated `PreparedApp` without ordinary components or a
protocol runtime; `Built` owns the constructed `App` after `after_build`, before serving. The
Rustdoc's generated-method table lists every stage accessor, transition, and state escape hatch.

## Generated CLI Migration

The generated CLI owns framework bootstrap. Keep `main` thin unless the process has a concrete
reason to own runtime construction or error policy:

```rust
#[tokio::main]
async fn main() -> Result<(), upwell::CliError> {
    NotifyApplication::run().await
}
```

`run()` uses Clap's normal print-and-exit behavior for help, version, and usage errors.
`run_with(args)` never prints or exits and returns `CliError::Clap`, which makes it suitable for
tests and embedding.

Omitting `cli` in the declaration preserves framework slot defaults. A declared serve phase adds a
`serve` command and selects it when no command is given. The configurable canonical slots are
`config`, `profile`, `log`, `log_format`, `color`, and `serve`; Clap help and version remain
framework-owned. `false` disables only the parser source, while `true` explicitly preserves normal
slot behavior.

Use exact Clap default forms. Scalar slots accept `default_value` or `default_value_t`; repeated
profiles accept `default_values` or `default_values_t`. Typed values must meet Clap's `Display` and
parser round-trip requirements. `LogFormat` implements `Display`, but `PathBuf` and the current
`ColorChoice` do not, so config paths and color normally use literal `default_value`:

```rust
cli: {
    config: { default_value: "config" },
    profile: { default_values_t: [String::from("development")] },
    log_format: { default_value_t: LogFormat::Compact },
    color: { default_value: "never" },
    serve: { name: "run", default_command: true },
}
```

Generated bootstrap retains Clap `ValueSource`, so parser defaults remain lower precedence than
explicit CLI, environment, and loaded logging configuration. Refer to the Rustdoc's per-setting
precedence table rather than treating every slot as one generic precedence chain.

## Commands And Global Arguments

Move command-local options into a `clap::Args` type implementing `CliCommand<Host>`. Shared groups
go in `args` and are flattened into the root parser; mark individual fields `#[arg(global = true)]`
when they must parse after a subcommand. Nested `commands` blocks create nested native subcommand
enums while preserving one generated subcommand field at each parser level.

Command names normalize from Rust snake case to lowercase kebab case. Generated command enums have
the host's visibility, so public hosts require publicly usable argument and command types. The DSL
accepts only documented, non-structural `#[command(...)]` settings; options that rename, flatten,
replace, skip, or externally extend generated variants are rejected. The complete allowlist and
phase-specific `CommandContext` APIs are linked from the authoritative Rustdoc.

## Static Plugins

Parser-visible plugins must be selected statically:

```text
plugins: [
    OperationsPlugin,
    replace PROTOCOL_OWNED_REPLACEABLE_SLOT => ReplacementPlugin,
    suppress PROTOCOL_OWNED_OPTIONAL_SLOT,
]
```

Installed and replacement plugin types are synchronously `Default`-constructed. Replacement and
suppression require a slot actually exposed by the selected protocol with the matching policy. The
built-in RPC and Axum protocols currently expose no default plugin slots, so they have no built-in
slot constants to use in those directives; ordinary plugin installation remains supported.

The early boundary lets the effective plugin set contribute typed global args or commands before
parsing, then transfers the same retained instances into preparation. Plugins added from lifecycle
builder callbacks or direct builder methods are runtime-only and cannot alter the parser or resolved
default slots.

## Tooling Migration

The `tooling` feature is independent of `cli`. It generates a target-local callable probe; with
`cli`, `run()` also recognizes the private dedicated-process probe argument before ordinary parsing.
The probe executes setup through preparation in `ExecutionMode::Tooling`, projects the exact
immutable prepared plan, and never constructs ordinary components, the root container, or protocol
runtime. It does not run after-build or serve.

Static plugin construction and application callbacks still run because they define the parser and
prepared plan. Guard callback side effects explicitly:

```rust
setup(context) {
    if context.mode().is_run() {
        register_external_process_state()?;
    }

    Ok::<_, SetupError>(context)
}
```

CLI metadata, when present, is extracted from the final composed Clap parser, including plugin
augmentation, ownership, aliases, defaults, cardinality, and nesting. Probe responses are published
atomically without replacing a raced destination. Cargo package/binary target selection and private
response-directory orchestration remain the external #152 tooling boundary for isolation and stale
temporary cleanup; named apps do not register process-global callbacks and tooling does not parse
source.

## Custom Main And Direct Builder

Use direct typestate lifecycle entry when only process/runtime ownership is custom:

```rust
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    runtime.block_on(async {
        NotifyApplication::new(ExecutionMode::Run).serve().await
    })?;

    Ok(())
}
```

Direct lifecycle entry starts with an empty `BootstrapContext`; it does not parse generated CLI
bootstrap options. Use public bootstrap/host functions explicitly if a custom runner needs those
semantics.

Use `App::<Protocol>::builder(name).auto_discover()` when assembly must capture local values, supply
plugin options/instances, or mutate the builder outside lifecycle callbacks. That escape hatch does
not generate a named host, callbacks, command tree, runner, parser-visible static-plugin boundary,
or private tooling entry.

Complete current examples:

- [`examples/daemon`](../examples/daemon/src/main.rs): reserved CLI customization, application and
  plugin commands, lifecycle hooks, generated runner, and inline serve DI.
- [`examples/http`](../examples/http/src/main.rs): protocol-specific builder customization in
  `configure` and generated serving.
- [`tests/app_definition.rs`](../tests/app_definition.rs): manager forms, lifecycle transitions,
  errors, and tooling no-construction checks.
- [`tests/app_commands.rs`](../tests/app_commands.rs): generated CLI defaults, command nesting,
  collisions, plugin CLI, and parser provenance.
