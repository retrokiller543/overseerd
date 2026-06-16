# Feature Specification: Initial Prototype Crates

**Feature Branch**: `001-initial-prototype-crates`

**Created**: 2026-06-16

**Status**: Draft

**Input**: User description: "we need to start with the very first rough prototype of this project now, base it off the README.md file, we need to keep a clear bounds for the crates, so one crate does one thing and nothing outside of its responsibility, all crates are stored in crates/."

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Build a first daemon prototype from README concepts (Priority: P1)

As an Overseer framework developer, I want a rough but coherent prototype that demonstrates components, services, typed RPC metadata, daemon assembly, and user-owned startup so that the project moves from a placeholder library toward the README vision without over-scoping the first implementation.

**Why this priority**: This is the smallest valuable slice of the project: it proves the core product idea before investing in automation, transports, SDKs, or broader operational features.

**Independent Test**: Can be tested by exercising one minimal daemon example or test fixture that registers a component, registers a service with an RPC-style operation, assembles a daemon definition, and exposes the resulting metadata for inspection.

**Acceptance Scenarios**:

1. **Given** a new developer reading the repository, **When** they run the prototype demonstration, **Then** they can see a component, service, RPC operation, and daemon definition represented in the system.
2. **Given** an application owner who wants control over startup, **When** they inspect the prototype entrypoint or example flow, **Then** the application remains responsible for startup decisions before handing assembled definitions to Overseer.
3. **Given** the README promise that generated or discovered behavior must be inspectable, **When** the prototype registers its example daemon elements, **Then** registered services, operations, dependencies, and contracts can be listed in a human-readable form.

---

### User Story 2 - Maintain strict crate responsibility boundaries (Priority: P2)

As a project maintainer, I want each crate to have one clearly documented responsibility and all crates stored under `crates/` so that early prototype work does not create tangled ownership boundaries that will be hard to undo later.

**Why this priority**: The user explicitly identified crate bounds as a foundational constraint, and early boundaries will shape all later implementation and planning.

**Independent Test**: Can be tested by reviewing the workspace layout and crate-level descriptions to confirm every crate lives under `crates/`, has a single stated responsibility, and avoids owning behavior assigned to another crate.

**Acceptance Scenarios**:

1. **Given** the repository after prototype setup, **When** a maintainer lists workspace crates, **Then** each non-root implementation crate is located under `crates/`.
2. **Given** a crate in the prototype, **When** a maintainer reads its package description or top-level documentation, **Then** the crate's responsibility is clear and does not overlap with another crate's responsibility.
3. **Given** a proposed change to a crate, **When** the change belongs to another responsibility area, **Then** the crate boundary guidance makes that mismatch visible before implementation proceeds.

---

### User Story 3 - Provide a foundation for future runtime, macro, and transport work (Priority: P3)

As a future contributor, I want the prototype to expose stable concepts and boundaries for metadata, runtime assembly, macros, and transports so that later features can build on the same model instead of introducing competing abstractions.

**Why this priority**: The README identifies future growth areas, but the first prototype should only prepare the seams needed for later work rather than implementing everything at once.

**Independent Test**: Can be tested by comparing the prototype's documented boundaries against README goals and confirming that future macro generation, Unix socket transport, task supervision, and SDK generation can attach to the model without requiring a full rewrite of the core concepts.

**Acceptance Scenarios**:

1. **Given** the README's metadata-first principle, **When** future macro work is considered, **Then** macros can target descriptors rather than embedding hidden runtime behavior.
2. **Given** the README's convention-assisted discovery principle, **When** future auto-discovery is considered, **Then** it can build on explicit registration instead of replacing it.
3. **Given** future transport and SDK goals, **When** contributors inspect the prototype contracts, **Then** they can identify where transport routing and client generation would consume the same metadata.

---

### Edge Cases

- If a README goal is too broad for the first prototype, it must be documented as intentionally out of scope rather than partially hidden inside an unrelated crate.
- If a crate appears to need two responsibilities, the boundary must be split or one responsibility explicitly deferred.
- If automatic discovery is not ready, the prototype must still support explicit registration and must not require hidden convention behavior.
- If generated behavior is introduced in any form, the generated or derived metadata must remain inspectable.
- If an example daemon needs startup setup, the example must keep startup ownership outside Overseer's core abstractions.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: The prototype MUST establish a repository structure where implementation crates are stored under `crates/`.
- **FR-002**: Each prototype crate MUST have one documented responsibility and MUST NOT own behavior outside that responsibility.
- **FR-003**: The prototype MUST provide a minimal metadata model for components, services, RPC-style operations, dependencies, and daemon-level registration.
- **FR-004**: Developers MUST be able to register components and services explicitly without relying on automatic discovery.
- **FR-005**: The prototype MUST demonstrate user-owned startup by allowing the application to perform setup before invoking Overseer daemon assembly or execution behavior.
- **FR-006**: The prototype MUST expose an inspection path for registered components, services, RPC-style operations, dependencies, and daemon contracts.
- **FR-007**: The prototype MUST include at least one independently verifiable demonstration of a component-backed service operation from registration through metadata inspection.
- **FR-008**: The prototype MUST keep convention-assisted discovery, macro-generated registration, production transport behavior, generated client SDKs, health checks, metrics, authentication, configuration management, deployment tooling, and background job systems out of scope unless they are represented only as documented future extension points.
- **FR-009**: The prototype MUST make future runtime, macro, transport, and client-generation responsibilities separable so later crates can consume the metadata model without circular ownership.
- **FR-010**: The prototype MUST replace placeholder behavior with project-relevant behavior derived from the README vision.

### Constitution Alignment *(mandatory)*

- **Scope Control**: In scope: a first rough prototype based on README concepts, bounded crate layout under `crates/`, explicit registration, metadata inspection, and a minimal demonstrable daemon slice. Out of scope: production-ready discovery, procedural macro implementation, Unix socket production transport, generated Rust client SDKs, health/metrics/auth/config/deployment features, and broad refactors unrelated to the prototype.
- **Independent Verification**: User Story 1 is verified through a minimal daemon demonstration; User Story 2 is verified through workspace and crate-boundary review; User Story 3 is verified by checking future extension seams against README goals.
- **Interfaces & Data Contracts**: The prototype introduces conceptual contracts for component descriptors, service descriptors, RPC operation descriptors, dependency relationships, daemon registration, and inspection output. It has no persistence or migration impact.
- **Operational Safety**: The prototype must avoid secrets, credentials, persistence, or network exposure. Any errors in the demonstration must be understandable to developers and must identify the failed registration, dependency, or inspection step without hiding context.

### Key Entities *(include if feature involves data)*

- **Component Descriptor**: Represents a reusable dependency available to services, including its identity and inspectable metadata.
- **Service Descriptor**: Represents daemon functionality and the operations it exposes.
- **RPC Operation Descriptor**: Represents a typed callable operation, its owning service, input/output contract summary, and dependencies needed to execute it.
- **Dependency Relationship**: Represents how services or operations depend on components or other framework-provided context.
- **Daemon Definition**: Represents the assembled set of components, services, operations, dependencies, and lifecycle-relevant metadata for one daemon.
- **Crate Boundary**: Represents the responsibility assigned to one crate and the behaviors explicitly excluded from that crate.
- **Prototype Demonstration**: Represents the minimal end-to-end example or test fixture proving the README concepts are present and inspectable.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: A developer can identify every prototype crate's responsibility and excluded responsibilities within 5 minutes of reading the repository's crate-level documentation.
- **SC-002**: The primary prototype demonstration completes successfully and shows a registered component, service, operation, and daemon definition in one run.
- **SC-003**: 100% of implementation crates introduced by the prototype are located under `crates/`.
- **SC-004**: 100% of prototype crates have a single stated responsibility with no overlapping ownership identified during review.
- **SC-005**: The prototype exposes at least five inspectable metadata categories: components, services, operations, dependencies, and daemon-level registration.
- **SC-006**: A maintainer can map each implemented prototype capability back to a README initial goal or documented future extension point without finding unrelated behavior.

## Assumptions

- The first prototype is intentionally rough and foundational; it should prove shape and boundaries before production completeness.
- README.md and docs/vision.md are the source of product intent for this specification.
- Explicit registration is the default for the prototype; automatic discovery and procedural macros may be added later on top of the explicit model.
- The root package may remain as a convenience facade or workspace entrypoint only if its responsibility stays clearly documented and does not absorb implementation responsibilities that belong under `crates/`.
- No existing persisted data or external consumers need migration for this first prototype.
