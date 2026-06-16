# Quickstart: Initial Prototype Crates

This guide describes how to validate the first rough prototype after implementation tasks are completed.

## Prerequisites

- Rust toolchain compatible with the workspace edition.
- Cargo available on PATH.
- Repository checked out on branch `001-initial-prototype-crates`.

## Expected Workspace Shape

After implementation, the prototype should include:

```text
Cargo.toml
src/lib.rs
crates/
├── overseer-core/
│   ├── Cargo.toml
│   └── src/lib.rs
└── overseer-demo/
    ├── Cargo.toml
    └── src/
        ├── lib.rs
        └── main.rs
```

Refer to `contracts/crate-boundaries.md` for exact crate responsibilities.

## Validation Scenario 1: Workspace and Crate Boundaries

Run:

```sh
cargo metadata --no-deps
```

Expected outcome:

- Workspace metadata lists the root `overseer` package.
- Workspace metadata lists `overseer-core` under `crates/overseer-core`.
- Workspace metadata lists `overseer-demo` under `crates/overseer-demo`.
- No implementation crate introduced by the prototype lives outside `crates/`.

## Validation Scenario 2: Automated Behavior Checks

Run:

```sh
cargo test --workspace
```

Expected outcome:

- Core descriptor and registration tests pass.
- Demo validation tests pass.
- Tests prove at least one component-backed service operation can be registered and inspected.
- Tests prove invalid duplicate identifiers or unresolved dependencies produce clear errors.

## Validation Scenario 3: Runnable Prototype Demonstration

Run:

```sh
cargo run -p overseer-demo
```

Expected outcome:

- The command completes successfully.
- Output shows the daemon name.
- Output shows at least one registered component.
- Output shows at least one registered service.
- Output shows at least one RPC-style operation with input and output contract summaries.
- Output shows at least one dependency relationship.
- Output makes it clear that the demo assembles the daemon after application-owned setup, rather than Overseer owning process startup.

## Validation Scenario 4: Facade Usability

Run:

```sh
cargo test -p overseer
```

Expected outcome:

- The root facade compiles.
- Root-level tests or compile checks prove consumers can access the prototype's public core concepts without moving implementation responsibility into the root crate.

## Validation Scenario 5: Scope Guard Review

Review:

- `contracts/core-registration.md`
- `contracts/inspection-output.md`
- `contracts/crate-boundaries.md`
- `data-model.md`

Expected outcome:

- Implemented behavior maps to the README-derived metadata and explicit registration model.
- No production transport, procedural macro, generated SDK, health/metrics/auth/config/deployment, persistence, credential, or network behavior was introduced as part of this first prototype.
