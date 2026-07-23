# Wave 2: Generated CLI, Commands, And Extensions

## Outcome

Land three issue-sized PRs on the shared `feat/141-149-app-cli-tooling` integration branch:

1. `#144` generates a default-on, feature-gated Clap CLI and framework bootstrap flow.
2. `#145` adds typed app commands with minimum lifecycle phases.
3. `#146` adds a public Clap-native plugin/protocol extension seam.

Only `#144` is implementation-ready below. Replan `#145` and `#146` from the merged `#144` API before coding them.

## Delivery Sequence

- Active branch: `feat/app-cli/144/generated-cli`.
- Active draft PR: `#163`, targeting `feat/141-149-app-cli-tooling`.
- After owner merge of `#144`, create `feat/app-cli/145/typed-commands` with an empty Conventional Commit and immediate draft PR.
- After owner merge of `#145`, create `feat/app-cli/146/plugin-extensions` with an empty Conventional Commit and immediate draft PR.
- The assistant never merges PRs.
- Resolve every addressed review conversation.
- An issue is complete only when CI passes and automated review has no unaddressed issues. Expected, explicitly approved 1.0 breaking changes may be documented and resolved rather than reverted.
- Commit messages follow Conventional Commits 1.0.0. Breaking commits use `!` plus a `BREAKING CHANGE:` footer.
- Run test suites with cargo-nextest.

## Settled CLI Boundaries

- Clap is the CLI API. Do not introduce a protocol-neutral argument schema.
- Add a default-on `cli` feature to the facade, `overseerd-app`, `overseerd-macros`, and `overseerd-macros-core`.
- Clap references and generated runner APIs exist only under `cli`; named app definitions, lifecycle APIs, direct `App::builder`, and tooling preparation compile without it.
- Re-export Clap from the framework for shared runtime types, but require applications using generated CLI derives to declare a direct Clap dependency. This keeps generated `Parser`/`Args`/`Subcommand` types native and directly extensible by application code.
- Finite CLI choices use enums: `LogFormat` and `ColorChoice`. Open filter grammars remain strings.
- No subcommand means `serve`.
- A named app gets generated `run()` and `run_with(args)` only when it declares a `serve` lifecycle phase. The framework cannot infer a protocol endpoint without protocol-specific semantics. Apps without `serve` keep builder/setup/prepare/build and can provide a custom main.
- `run()` is the process-owned convenience that reads `std::env::args_os()` and renders Clap/lifecycle errors.
- `run_with(args)` accepts an `IntoIterator<Item = Into<OsString> + Clone>` and returns a typed `Result`, without exiting the process. Tests and embedding use this API.
- `run_with` parses normal CLI arguments only. Private tooling invocation remains `#147`.
- `#144` owns only the built-in `serve` command. App-defined commands belong to `#145`; plugin arguments/commands belong to `#146`.
- Core global options are protocol-neutral. Do not add host, port, socket, transport, TLS, jobs, RPC, or Axum options.

## Bootstrap Model

### Generated Types

Under `cli`, each named app with a serve phase generates or aliases:

```rust
#[derive(clap::ValueEnum)]
pub enum ColorChoice {
    Auto,
    Always,
    Never,
}

#[derive(clap::Args)]
pub struct BootstrapOptions {
    pub config: Option<PathBuf>,
    pub profiles: Vec<String>,
    pub log: Option<String>,
    pub log_format: Option<LogFormat>,
    pub color: Option<ColorChoice>,
}

#[derive(clap::Subcommand)]
pub enum Command {
    Serve,
}

#[derive(clap::Parser)]
pub struct Cli {
    pub bootstrap: BootstrapOptions,
    pub command: Option<Command>,
}
```

Use host-prefixed generated identifiers internally where collisions are possible, while exposing stable associated aliases or documented host methods. Two named apps in one module must compile without generated-name collisions.

### Runtime Bootstrap State

Extend `BootstrapContext` with typed framework-owned state rather than requiring callers to know extension-map keys:

- parsed `BootstrapOptions` under `cli`;
- resolved `DirectoriesManager`;
- merged `ConfigManager<Dynamic>`;
- active profiles in precedence order;
- effective `LoggingConfig`;
- effective `ColorChoice`;
- whether tracing was installed by generated bootstrap.

Keep constructors and accessors public. Keep fields private so later bootstrap additions do not force another struct-literal break.

### Source Precedence

Implement and test this precedence for framework-owned bootstrap values:

```text
CLI > environment > profile config > base config > type defaults
```

- Config location:
  - CLI `--config`.
  - `OVERSEERD_CONFIG`.
  - generated platform application config directory.
- Profiles:
  - CLI repeatable `--profile`.
  - existing `OVERSEERD_PROFILES` behavior when CLI profiles are absent.
  - profile files override base files in declared order.
- Log filter:
  - CLI `--log`.
  - `RUST_LOG`.
  - `logging.level`.
  - `LoggingConfig::default()`.
- Log format:
  - CLI `--log-format`.
  - `OVERSEERD_LOG_FORMAT`.
  - `logging.format`.
  - `LogFormat::default()`.
- Color:
  - CLI `--color`.
  - `NO_COLOR` forces `Never`; otherwise `CLICOLOR_FORCE` forces `Always` when CLI is absent.
  - `Auto` default.

Do not invent arbitrary environment-to-config-field mapping. Existing `${ENV}` placeholder resolution remains the config system's general environment seam.

### Config Location Semantics

- If config path is a directory, load `application.<ext>` and selected profile overlays using existing `ConfigManager::load_in` behavior.
- If config path is a file, load that exact file as the base source, then locate profile files beside it using `<stem>-<profile>.<ext>`.
- Preserve format dispatch by extension and enabled features.
- Add a narrow public config-manager constructor/API for exact-file plus profile loading; do not expose raw tree mutation solely for CLI overrides.
- CLI log/color overrides are bootstrap values and do not mutate the general config tree in `#144`. They are inserted into `BootstrapContext` and used for tracing. General typed config overlays may be designed later if command/plugin needs establish a concrete requirement.
- Generated default setup passes the resolved directories and config manager into `Host::builder` configuration. Existing declaration-provided `managers` remain authoritative only when custom setup/configure paths explicitly use them; avoid loading two competing managers.

## PR `#144` Implementation Increments

### 1. Feature And Public Types

- Add workspace Clap dependency with only required features (`derive`, `std`, `help`, `usage`, `error-context`, `suggestions`, `color`; add `env` only if generated fields directly use Clap env parsing). Document the matching direct downstream dependency required by generated derives.
- Add and forward default-on `cli` features through facade/app/macro crates.
- Keep Wasm builds free of native app/CLI dependencies through existing target gates.
- Add public documented `ColorChoice`, typed `CliError`, and bootstrap accessors under `cli`.
- Implement Clap `ValueEnum` for framework enums through the gated re-export or generated/manual trait implementations.
- Add feature-matrix checks proving `--no-default-features` emits no Clap references.

### 2. Config Source Completion

- Add exact-file plus ordered-profile loading to `overseerd-config`.
- Preserve retained source order so reload maintains profile precedence.
- Test TOML, YAML when enabled, absent profiles, malformed files, duplicate profile order, and file-versus-directory selection.
- Keep environment profile fallback deterministic and prevent CLI profile values from being appended twice.

### 3. Default Bootstrap

- Add a generated default setup implementation that:
  1. resolves directories from the app name;
  2. resolves config location and profiles;
  3. loads and auto-discovers config;
  4. extracts `LoggingConfig` from `logging`, using defaults when absent;
  5. computes effective filter/format/color precedence;
  6. installs tracing only in `ExecutionMode::Run` and only when the tracing-subscriber feature is enabled;
  7. stores resolved bootstrap state in `BootstrapContext`.
- When a custom `setup = path` or inline setup is declared, pass parsed `BootstrapOptions` as framework context rather than silently bypassing them. The custom setup owns whether to call default bootstrap helpers.
- Add a public lower-level bootstrap helper so custom setup/custom main can reuse the generated defaults without process argument parsing.
- Use typed errors for directory resolution, config loading, tracing setup, and missing required bootstrap state.

### 4. Builder Integration

- Ensure generated configure uses the bootstrap-resolved directories/config manager when the declaration does not supply custom manager expressions.
- Keep explicit declaration manager expressions working for custom/advanced assembly and document precedence against default bootstrap managers.
- Avoid cloning non-clone config managers: move bootstrap-owned managers exactly once into builder configuration while retaining inspectable metadata separately.
- Add tests proving the configured builder receives the same source/profile state used by bootstrap extraction.

### 5. CLI And Runner Generation

- Generate `BootstrapOptions`, `Command`, and `Cli` for named serve-capable apps.
- Use app runtime name as command name and package metadata for version/about where available.
- Parse global options before and after `serve` where Clap global semantics permit.
- Dispatch `None | Some(Command::Serve)` through `build(ExecutionMode::Run)` and the app's declared `serve_with` phase.
- `run_with` returns `Result<(), CliError>` and never calls `std::process::exit`.
- `run` may render the error and return it; generated binaries decide whether to return the error from `main`. Do not terminate inside framework library code.
- Keep direct `Host::builder`, `setup`, `prepare`, `build`, and `serve_with` available as escape hatches.

### 6. Documentation And Example Scope

- Update macro docs with a thin main:

  ```rust
  #[tokio::main]
  async fn main() -> Result<(), AppCliError> {
      Homeledger::run().await
  }
  ```

- Add one focused serve-capable named app test/example. Full Homeledger migration remains `#148`.
- Document feature disabling, custom main, `run_with`, precedence, and why protocol options are absent from core.

## `#144` Test Matrix

Parser/expansion tests:

- CLI code emitted only with `cli`.
- two named apps do not collide;
- serve-capable app generates runner; app without serve does not;
- package/app metadata appear in command output;
- generated types use native Clap derives and compile in a downstream-style target with a direct Clap dependency.

CLI behavior tests using `run_with` or Clap parsing without process exits:

- no subcommand dispatches serve;
- explicit `serve` dispatches the same path;
- `--help`, `serve --help`, and `--version` output;
- invalid format/color/profile/config arguments are typed Clap errors;
- repeatable ordered profiles;
- options accepted in documented global positions;
- custom argv program name does not change app identity.

Precedence tests:

- CLI log beats `RUST_LOG`, which beats config/default;
- CLI profile selection prevents environment profiles from being appended unexpectedly;
- later profile files override earlier profiles and base config;
- CLI config file/directory beats `OVERSEERD_CONFIG` and platform default;
- CLI color beats `NO_COLOR`/`CLICOLOR_FORCE` with documented behavior;
- absent options preserve existing defaults.

Lifecycle tests:

- parse/help/version do not execute setup or build;
- serve runs setup/configure/before-build/prepare/build/after-build/serve once in order;
- bootstrap/config/tracing failures identify the correct phase/source;
- tooling mode never enters generated normal CLI construction;
- custom `run_with` arguments never read `std::env::args_os()`.

Feature tests:

- `cargo check --workspace --no-default-features`;
- app/macro crates with `cli` disabled;
- facade default features include CLI but no protocol;
- Wasm client checks remain green;
- tracing-subscriber disabled compiles and leaves tracing installation to the application.

## `#144` Validation Gate

```text
cargo fmt --all -- --check
cargo nextest run --workspace --all-features
cargo clippy --workspace --all-targets --all-features
cargo check --workspace --no-default-features
```

Also run focused feature combinations introduced by `cli`, then let PR `#163` CI and automated review complete. Resolve all addressed conversations. Leave merge to the project owner.

## Later Issue Boundaries

### Settled Command Execution Model

- Unify application and plugin commands under one `CliCommand<H>` trait implemented by the fully
  parsed command node rather than a separate handler function plus associated `Args` type.
- `CliCommand::run` takes `&self`; the parsed value owns its local arguments and nested command
  tree. This supports arbitrary native Clap nesting such as `app api users list`.
- Make a nested `commands` DSL the default application-facing API. The macro generates every
  intermediate native Clap `Subcommand` enum and recursively delegates `phase()` and `run()` to the
  selected leaf. A generated command tree may contain leaves with different lifecycle requirements.
- A leaf value implements both native Clap `Args` and `CliCommand<H>`; it stores all of its parsed
  command-local values and executes through `run(&self, context)`.
- Support explicit nested and flattened inclusion of reusable command-set enums for plugins,
  generated clients, and command packages. Hand-written nesting is therefore optional rather than
  required.
- The runner asks the selected parsed command for its `CommandPhase`, advances the host only to
  that phase, then supplies `CommandContext<'_, H>`.
- `CommandContext` exposes bootstrap/global CLI state at every phase and, when available, prepared
  or built app state plus typed resolution from the app's global/root scope. Command-local Clap
  values remain on `self`; framework/DI values come from the context.
- Keep exactly one generated top-level `#[command(subcommand)]` field. Compose command sets through
  native nested command values or flattened wrapper variants inside that one generated command
  enum.
- Use a separate `args` declaration only for flattened global argument groups. Move each parsed
  global args value into `BootstrapContext` by type before lifecycle setup, making it available to
  command execution and configuration.
- Do not use untyped external subcommands or runtime mutation as the primary extension model.

Conceptual trait shape:

```rust
pub trait CliCommand<H: AppHost> {
    type Error: std::error::Error + Send + Sync + 'static;

    fn phase(&self) -> CommandPhase;

    fn run<'a>(
        &'a self,
        context: CommandContext<'a, H>,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send + 'a;
}
```

Conceptual app declaration:

```rust
commands: {
    migrate: MigrateCommand,
    api: {
        users: {
            list: ListUsersCommand,
            get: GetUserCommand,
        },
        groups: {
            list: ListGroupsCommand,
        },
    },
    rpc: nested overseerd::daemon::RpcCommands,
    tooling: flatten ToolingCommands,
}
```

`api` and `users` become generated namespace subcommands. `nested` mounts a reusable command set
under the declared name; `flatten` contributes its commands as siblings at the current level.

### `#145` Typed Commands

- Extend the generated command enum with parsed command-node types from app declarations.
- Introduce dynamic selected-leaf phase requirements (`setup`, `configured`, `built`) and the
  lifecycle-aware `CommandContext`.
- Dispatch setup-only commands without preparing/building, configured commands before component
  construction, and built commands without serving.
- Generate `CliCommand` delegation for the app-specific top-level enum while allowing user-owned
  command nodes to implement it directly and recursively.
- Add reserved-name and duplicate validation.
- Replan exact configured-state ownership after `#144` merges.

### `#146` Plugin Extensions

- Add Clap-native feature-gated extension traits.
- Keep exactly one `#[command(subcommand)]` field on the generated parser. Compose app/plugin
  command enums inside the single generated command enum through
  `#[command(flatten)] Extension(ExtensionCommands)` variants.
- Support multiple ordinary `#[command(flatten)]` extension `Args` fields on the parser, typed
  extraction, phase requirements, and collision diagnostics.
- Use the same `CliCommand` contract for application and plugin-owned command trees; do not add a
  second plugin-specific command handler abstraction.
- Let protocol extensions interpret bootstrap values during configure and provide serve endpoint semantics.
- Demonstrate one in-repo implementation and one third-party-style test crate.
- Keep non-`cli` plugin builds Clap-free.
