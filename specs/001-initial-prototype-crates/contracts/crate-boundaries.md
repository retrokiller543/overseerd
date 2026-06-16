# Crate Boundary Contract

## Purpose

Defines the planned crate responsibilities for the first rough prototype. A crate must do one thing and must not own behavior outside its responsibility.

## Workspace Location Rule

- All implementation crates introduced by this feature must live under `crates/`.
- The repository root package may remain only as a facade or workspace entrypoint.

## Planned Crates

### `overseer` Root Facade

**Path**: `.`

**Owns**:
- Public re-exports that make the prototype discoverable from the root package.
- Minimal facade documentation pointing users to core and demo behavior.

**Does not own**:
- Descriptor definitions.
- Registration validation.
- Demonstration domain logic.
- Runtime, transport, macro, or SDK generation behavior.

### `overseer-core`

**Path**: `crates/overseer-core`

**Owns**:
- Component descriptors.
- Service descriptors.
- RPC operation descriptors.
- Dependency relationships.
- Daemon definition assembly.
- Explicit registration validation.
- Inspection models.

**Does not own**:
- Prototype example domain logic.
- Procedural macros.
- Automatic discovery.
- Unix socket or other production transports.
- Generated client SDKs.
- Health, metrics, authentication, configuration, deployment tooling, or background jobs.

### `overseer-demo`

**Path**: `crates/overseer-demo`

**Owns**:
- The prototype demonstration fixture.
- A minimal component-backed service operation used to validate the core model.
- Human-readable demonstration output.

**Does not own**:
- Core descriptor behavior.
- Reusable framework registration logic.
- Production daemon hosting.
- Networking, persistence, credentials, or external integrations.

## Review Rule

If a task cannot be assigned to exactly one planned crate responsibility, the task must be split, deferred, or explicitly called out in the implementation plan before coding begins.
