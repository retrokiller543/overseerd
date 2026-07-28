# Cargo Overseerd Probe Discovery And Execution

Issue: #152
Parent epic: #149
Tracking PR: #161

## Goal

Establish the Cargo-side foundation shared by every `cargo overseerd` command and by editor
integrations. The slice discovers an explicitly selected application binary through Cargo metadata,
builds that target in a tooling-owned artifact directory, invokes its generated private probe, and
returns one validated versioned probe envelope without serving the application.

This slice does not add `check`, `doctor`, `inspect`, `export`, `graph`, `explain`, renderer, or
scaffolding behavior. Those commands consume the API introduced here in later issue-sized slices.

## Consumer Boundary

The Cargo orchestration is a library API first. The future terminal command, VS Code extension, and
RustRover plugin must share the same selection, build, execution, and validation behavior rather
than scrape terminal text or reimplement Cargo discovery.

The core API therefore:

- returns typed selection, Cargo, artifact, process, and envelope failures;
- preserves Cargo diagnostics and target-local stdout/stderr as structured evidence;
- retains stable package, manifest, binary, application, and source identities;
- never applies color, paging, terminal grouping, or human-only formatting;
- never reinterprets producer-native source or manifest paths on another platform;
- validates every received envelope through the versioned tooling schema;
- keeps results deterministic where Cargo input and prepared application state are equivalent;
- permits callers to cancel by dropping or terminating an invocation without consuming a partial
  response as valid output.

Later machine-facing commands may serialize these results and diagnostics, but no IDE-specific
transport or long-running language server belongs in this slice.

## Selection Contract

Discovery uses `cargo metadata --format-version 1 --no-deps`. Only workspace packages and ordinary
binary targets are candidates. Dependencies, examples, tests, benches, build scripts, and source
code inspection are not application discovery mechanisms.

Package selection uses this deterministic order:

1. an explicit package name;
2. the sole eligible workspace default member;
3. the sole eligible workspace member;
4. otherwise an actionable ambiguity error with sorted candidates.

Binary selection uses this deterministic order:

1. an explicit binary target name;
2. an eligible package `default-run` target;
3. the sole eligible binary target;
4. otherwise an actionable ambiguity error with sorted candidates.

A binary is eligible only when all of its `required-features` are enabled by the selected feature
configuration. Explicit selections receive distinct not-found and disabled-required-feature errors.
Package names and binary names remain separate Cargo identities.

## Cargo Contract

The invoker uses the `CARGO` environment variable when present and otherwise executes `cargo`. User
selection supports manifest path, package, binary target, target triple, default-feature policy,
all-features, and explicit features.

The selected target is built with Cargo JSON messages and an absolute tooling-owned target directory
beneath Cargo metadata's effective target directory:

```text
<cargo-target-directory>/overseerd/build
```

The executable path comes only from the matching Cargo `compiler-artifact` message. The invoker does
not construct target paths, append platform suffixes, or infer artifacts from package names.
Build output and normal project artifacts remain isolated.

## Probe Contract

Each invocation creates a private unique directory beneath:

```text
<cargo-target-directory>/overseerd/probes
```

The selected executable receives exactly the generated versioned hidden probe argument. The invoker
inherits the application environment after removing inherited reserved probe variables, then sets
the response path and selected package, package version, absolute manifest, and binary identities.
The application response file is separate from stdout and stderr and must not exist before launch.

After the process exits, the invoker bounds response size, reads the response file, and calls
`ProbeEnvelope::from_json`. A valid failure envelope remains an authoritative framework result even
when the generated process exits with status 1. Missing, malformed, oversized, or schema-invalid
responses remain probe protocol failures with captured process evidence.

The generated tooling mode remains responsible for stopping after setup, configuration, validation,
and preparation. This Cargo-side slice does not construct ordinary components or protocol runtime,
run startup hooks, bind transports, start watchers, or serve.

## Implementation Stages

### Stage 1: Metadata And Selection

- Add the `cargo-overseerd` workspace crate as a reusable library.
- Define feature, Cargo, and target-selection inputs without terminal concerns.
- Load Cargo metadata and select one package and eligible binary deterministically.
- Add pure selection tests covering defaults, ambiguity, required features, and stable candidate
  ordering.

### Stage 2: Build And Probe

- Build the selected target under the isolated tooling target directory.
- Parse Cargo messages and retain the exact matching executable artifact.
- Invoke the private probe with a private response directory and reserved environment contract.
- Decode and validate the response envelope while preserving process output and status.
- Add fixture and Homeledger-shaped end-to-end coverage proving no runtime construction or serving.

### Stage 3: Public Orchestration

- Expose one narrow discover/select/build/probe entry point over the typed lower-level stages.
- Document which returned data is stable schema identity and which evidence is local process context.
- Keep terminal rendering and command exit-code policy out of the library.

## Validation

- `cargo fmt --all -- --check`
- focused `cargo nextest run -p cargo-overseerd`
- `cargo clippy --workspace --all-targets --all-features`
- `cargo nextest run --workspace --all-features`
- `cargo test --doc --workspace --all-features`
- `cargo check --workspace --no-default-features`
- process coverage for successful, failed, missing, malformed, and identity-invalid probe responses
- workspace/package/binary ambiguity and feature-gated target coverage
- native Windows artifact-path and process-environment behavior through CI

## Exclusions

- terminal and JSON command rendering (#153-#155);
- stable command exit-code policy (#153);
- optional protocol/plugin display renderers (#157);
- VS Code or RustRover transport, UI, caching, and lifecycle implementation;
- source parsing, macro-expansion parsing, or a global application callback registry;
- target runners, emulators, or remote cross-target execution;
- construction or execution of application runtime state;
- cleanup of ordinary Cargo target outputs.
