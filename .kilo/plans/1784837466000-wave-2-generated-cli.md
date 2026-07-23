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
- Re-export Clap from the framework under `cli`, and use the re-export in generated derive attributes so downstream apps do not need a duplicate direct Clap dependency.
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

- Add workspace Clap dependency with only required features (`derive`, `std`, `help`, `usage`, `error-context`, `suggestions`, `color`; add `env` only if generated fields directly use Clap env parsing).
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
- generated derive paths use framework Clap re-export.

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

### `#145` Typed Commands

- Extend the generated command enum with app declarations.
- Introduce typed phase requirements (`setup`, `configured`, `built`) and async adapters.
- Dispatch setup-only commands without preparing/building, configured commands before component construction, and built commands without serving.
- Add reserved-name and duplicate validation.
- Replan exact configured-state ownership after `#144` merges.

### `#146` Plugin Extensions

- Add Clap-native feature-gated extension traits.
- Support flattened args, subcommands, typed extraction, phase requirements, and collision diagnostics.
- Let protocol extensions interpret bootstrap values during configure and provide serve endpoint semantics.
- Demonstrate one in-repo implementation and one third-party-style test crate.
- Keep non-`cli` plugin builds Clap-free.
