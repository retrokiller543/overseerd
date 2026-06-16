# Research: Initial Prototype Crates

## Decision: Use a small Rust workspace with implementation crates under `crates/`

**Rationale**: The repository is already a Rust workspace using edition 2024, and the feature explicitly requires all crates to live under `crates/`. Keeping each implementation crate under `crates/` makes crate ownership inspectable and prevents the root package from becoming an unbounded implementation bucket.

**Alternatives considered**:
- Keep all code in the root crate: rejected because it violates the requested crate-boundary model and makes later macro/runtime/transport separation harder.
- Create many future-facing crates immediately: rejected because the first prototype must stay rough and bounded; only crates needed for the vertical slice should be introduced.

## Decision: Keep the root `overseer` package as a facade only

**Rationale**: Existing consumers will naturally look for the `overseer` crate, but the user's constraint requires implementation crates under `crates/`. A thin facade allows the repository root to remain the ergonomic entrypoint while keeping implementation responsibility in bounded crates.

**Alternatives considered**:
- Remove the root package and make only path crates public: rejected because it reduces the discoverability of the framework entrypoint and adds churn unrelated to proving the prototype.
- Put core implementation in the root package: rejected because it weakens the `crates/` boundary requirement.

## Decision: Start with two implementation crates: `overseer-core` and `overseer-demo`

**Rationale**: `overseer-core` can own descriptors, explicit registration, daemon definitions, dependency relationships, and introspection models. `overseer-demo` can own the independently verifiable demonstration without polluting core with example-specific domain logic. This is the smallest split that satisfies "one crate does one thing" while proving the README concepts.

**Alternatives considered**:
- Add `overseer-runtime`, `overseer-ipc`, `overseer-macros`, and `overseer-client` now: rejected because production runtime, transport, macros, and SDK generation are out of scope for the first prototype.
- Put the demonstration in examples under the root crate: rejected because the demo has its own responsibility and should not encourage implementation outside `crates/`.

## Decision: Explicit registration first; auto-discovery and macros deferred

**Rationale**: The README states convention-assisted discovery should be optional and built on inspectable metadata. Explicit registration validates the model without introducing procedural macro or linker/discovery complexity before the descriptor contracts are stable.

**Alternatives considered**:
- Implement procedural macros first: rejected because macro behavior would hide model problems and violates the metadata-first development strategy.
- Implement auto-discovery first: rejected because discovery should be a convenience over explicit registration, not the foundation.

## Decision: Model RPC operations as descriptors before executable transport

**Rationale**: The first prototype needs to show typed RPC-style operations and inspectable contracts, but production Unix socket routing is explicitly out of scope. Operation descriptors can capture names, input/output summaries, owning service, and dependencies without committing to transport behavior.

**Alternatives considered**:
- Implement a Unix socket transport immediately: rejected because it expands scope into networking and operational behavior before the metadata model is proven.
- Skip RPC concepts entirely: rejected because typed RPC handlers are part of the README's initial goals and required by the spec.

## Decision: Validate with `cargo test` and a runnable demo command

**Rationale**: The feature requires an independently verifiable demonstration. Tests should validate descriptor and registration behavior programmatically, while a demo command should let a maintainer see the registered component, service, operation, dependency, and daemon metadata in one run.

**Alternatives considered**:
- Manual review only: rejected because the constitution requires testable increments and automated tests where behavior can be checked programmatically.
- Integration with a daemon process supervisor: rejected because process lifecycle supervision is not needed to validate this first slice.

## Decision: No persistence, credentials, or network exposure

**Rationale**: The first prototype is metadata and registration oriented. Avoiding persistence and network exposure keeps operational risk low and matches the spec's no-migration assumption.

**Alternatives considered**:
- Store generated contracts on disk at runtime: rejected because static documentation and test assertions are enough for this prototype.
- Bind a local socket for demonstration: rejected because production transport is out of scope.
