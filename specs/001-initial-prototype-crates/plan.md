# Implementation Plan: Initial Prototype Crates

**Branch**: `001-initial-prototype-crates` | **Date**: 2026-06-16 | **Spec**: [spec.md](./spec.md)

**Input**: Feature specification from `specs/001-initial-prototype-crates/spec.md`

## Summary

Build the first rough Overseer prototype as a narrow Rust workspace vertical slice based on the README vision. The implementation will move project-relevant framework behavior out of the placeholder root crate and into bounded crates under `crates/`: `overseer-core` for descriptor-first metadata, explicit registration, validation, daemon definitions, and inspection; `overseer-demo` for the runnable demonstration fixture. The root `overseer` package remains only as a thin facade so the public entrypoint stays ergonomic while crate responsibilities remain clear.

## Technical Context

**Language/Version**: Rust edition 2024; current local toolchain reported `rustc 1.96.0-nightly (cf7da0b72 2026-03-30)` and `cargo 1.96.0-nightly (e84cb639e 2026-03-21)`

**Primary Dependencies**: Rust standard library only for the first prototype; no new third-party runtime, macro, transport, serialization, or observability dependencies planned

**Storage**: N/A; no persistence, migrations, generated runtime files, or stored daemon state

**Testing**: `cargo test --workspace`; targeted crate checks with `cargo test -p overseer-core`, `cargo test -p overseer-demo`, and `cargo test -p overseer`

**Target Platform**: Rust library workspace with a local runnable demonstration; no production daemon hosting or network transport in this feature

**Project Type**: Rust library/framework prototype with a root facade crate and bounded implementation crates under `crates/`

**Performance Goals**: Prototype registration, validation, inspection, and demo execution complete within normal local development feedback time; demo should complete in under 5 seconds on a developer machine

**Constraints**: One crate owns one responsibility; all implementation crates live under `crates/`; explicit registration is required; generated or assembled behavior must be inspectable; no production transport, procedural macros, generated SDKs, persistence, credentials, or network exposure in this feature

**Scale/Scope**: One minimal daemon definition, at least one component descriptor, at least one service descriptor, at least one RPC-style operation descriptor, at least one dependency relationship, one runnable demonstration, and crate-boundary documentation for the prototype crates

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

- Spec traceability: PASS. The specification includes prioritized user stories, acceptance scenarios, edge cases, requirements, assumptions, success criteria, and key entities.
- Minimal scope: PASS. Planned changes are limited to prototype crates, facade wiring, documentation, and validation artifacts required for the first README-derived vertical slice. Deferred features are explicitly out of scope.
- Testable increments: PASS. User Story 1 is verified through a runnable demo and automated registration/inspection tests; User Story 2 is verified through workspace metadata and crate-boundary review; User Story 3 is verified through contracts and future extension seams.
- Explicit contracts: PASS. Data model and contract artifacts define descriptors, registration behavior, inspection output, and crate boundaries. No persistence or migration impact exists.
- Operational safety: PASS. No credentials, persistence, or network exposure are planned. Error behavior is scoped to clear developer-facing validation failures.

## Project Structure

### Documentation (this feature)

```text
specs/001-initial-prototype-crates/
├── plan.md
├── research.md
├── data-model.md
├── quickstart.md
├── contracts/
│   ├── README.md
│   ├── core-registration.md
│   ├── inspection-output.md
│   └── crate-boundaries.md
└── tasks.md             # Created by /speckit-tasks, not /speckit-plan
```

### Source Code (repository root)

```text
Cargo.toml
src/
└── lib.rs               # Root `overseer` facade only

crates/
├── overseer-core/
│   ├── Cargo.toml
│   └── src/
│       └── lib.rs       # Descriptor model, explicit registration, validation, inspection
└── overseer-demo/
    ├── Cargo.toml
    └── src/
        ├── lib.rs       # Reusable demo fixture for tests
        └── main.rs      # Runnable human-readable prototype demonstration
```

**Structure Decision**: Use a Rust workspace with a root facade package plus two implementation crates under `crates/`. `overseer-core` owns the reusable framework prototype model. `overseer-demo` owns only the demonstration fixture and runnable output. The root `overseer` package re-exports the public prototype surface without absorbing implementation behavior.

## Phase 0: Research Summary

See [research.md](./research.md).

Key decisions:

- Use a small Rust workspace with implementation crates under `crates/`.
- Keep the root `overseer` package as a facade only.
- Start with two implementation crates: `overseer-core` and `overseer-demo`.
- Build explicit registration before auto-discovery or procedural macros.
- Model RPC operations as descriptors before executable production transport.
- Validate with `cargo test --workspace` and `cargo run -p overseer-demo`.
- Avoid persistence, credentials, and network exposure.

No unresolved `NEEDS CLARIFICATION` items remain after research.

## Phase 1: Design Summary

See [data-model.md](./data-model.md) for descriptor entities, validation rules, and relationships.

See [contracts/](./contracts/) for public behavior contracts:

- [Core registration contract](./contracts/core-registration.md)
- [Inspection output contract](./contracts/inspection-output.md)
- [Crate boundary contract](./contracts/crate-boundaries.md)

See [quickstart.md](./quickstart.md) for runnable validation scenarios.

## Post-Design Constitution Check

- Spec traceability: PASS. Design artifacts map directly to FR-001 through FR-010 and the three user stories.
- Minimal scope: PASS. The plan adds only the crates and docs required for the prototype slice. Future macros, production transports, SDK generation, health/metrics/auth/config/deployment, and background jobs remain out of scope.
- Testable increments: PASS. Quickstart includes workspace metadata validation, automated tests, runnable demo validation, facade usability checks, and scope review.
- Explicit contracts: PASS. Contracts define core registration, inspection output, and crate ownership. Data model defines all relevant descriptor entities and validation rules.
- Operational safety: PASS. No persistence, migrations, network listeners, credentials, or sensitive data handling are introduced. Planned errors are developer-facing validation messages only.

## Complexity Tracking

No constitution violations require justification.
