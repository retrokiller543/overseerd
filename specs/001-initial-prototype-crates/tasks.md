# Tasks: Initial Prototype Crates

**Input**: Design documents from `specs/001-initial-prototype-crates/`

**Prerequisites**: plan.md, spec.md, research.md, data-model.md, contracts/, quickstart.md

**Verification**: Every user story includes automated verification where behavior can be checked programmatically, plus manual or command validation where needed.

**Organization**: Tasks are grouped by user story to enable independent implementation and testing.

## Format: `[ID] [P?] [Story] Description`

- **[P]**: Can run in parallel with other tasks that touch different files and do not depend on incomplete work
- **[Story]**: Which user story this task belongs to: [US1], [US2], [US3]
- Every task includes exact file paths

## Phase 1: Setup (Shared Infrastructure)

**Purpose**: Create the bounded workspace skeleton required by the implementation plan.

- [ ] T001 Update workspace members and path dependencies for `overseer-core` and `overseer-demo` in Cargo.toml
- [ ] T002 Create `overseer-core` crate manifest with responsibility metadata in crates/overseer-core/Cargo.toml
- [ ] T003 Create initial `overseer-core` library module with crate-level responsibility documentation in crates/overseer-core/src/lib.rs
- [ ] T004 Create `overseer-demo` crate manifest with dependency on `overseer-core` in crates/overseer-demo/Cargo.toml
- [ ] T005 Create initial `overseer-demo` library module with crate-level responsibility documentation in crates/overseer-demo/src/lib.rs
- [ ] T006 Create initial runnable demo entrypoint with application-owned startup comments in crates/overseer-demo/src/main.rs
- [ ] T007 Replace placeholder root crate behavior with facade-only documentation and re-export scaffold in src/lib.rs

---

## Phase 2: Foundational (Blocking Prerequisites)

**Purpose**: Define the shared core vocabulary and validation surface that all user stories depend on.

**⚠️ CRITICAL**: No user story work can begin until this phase is complete.

- [ ] T008 Define public error type and result alias for descriptor validation failures in crates/overseer-core/src/lib.rs
- [ ] T009 Define identifier validation helper for non-empty descriptor IDs and names in crates/overseer-core/src/lib.rs
- [ ] T010 Define `ComponentDescriptor` with documented fields and constructors in crates/overseer-core/src/lib.rs
- [ ] T011 Define `RpcOperationDescriptor` with documented fields, contract summaries, and dependencies in crates/overseer-core/src/lib.rs
- [ ] T012 Define `ServiceDescriptor` with operation ownership and builder helpers in crates/overseer-core/src/lib.rs
- [ ] T013 Define `DependencyRelationship` for consumer/provider/reason metadata in crates/overseer-core/src/lib.rs
- [ ] T014 Define `DaemonDefinition` read-only descriptor container in crates/overseer-core/src/lib.rs
- [ ] T015 Define `DaemonBuilder` explicit registration API skeleton in crates/overseer-core/src/lib.rs
- [ ] T016 Add crate-boundary documentation comments for root facade behavior in src/lib.rs

**Checkpoint**: Foundation vocabulary exists and user story implementation can begin.

---

## Phase 3: User Story 1 - Build a first daemon prototype from README concepts (Priority: P1) 🎯 MVP

**Goal**: Demonstrate a component-backed service operation from explicit registration through inspectable daemon metadata while preserving user-owned startup.

**Independent Test**: `cargo test -p overseer-core`, `cargo test -p overseer-demo`, and `cargo run -p overseer-demo` show a registered component, service, RPC-style operation, dependency, and daemon definition.

### Verification for User Story 1 (REQUIRED) ⚠️

- [ ] T017 [P] [US1] Add failing core registration success test for component/service/operation/dependency assembly in crates/overseer-core/src/lib.rs
- [ ] T018 [P] [US1] Add failing core validation tests for duplicate IDs, empty fields, and missing dependency providers in crates/overseer-core/src/lib.rs
- [ ] T019 [P] [US1] Add failing demo fixture test asserting required inspection categories in crates/overseer-demo/src/lib.rs
- [ ] T020 [P] [US1] Add failing runnable demo output test or validation helper for daemon/component/service/operation/dependency text in crates/overseer-demo/src/lib.rs

### Implementation for User Story 1

- [ ] T021 [US1] Implement `DaemonBuilder` component registration and duplicate component validation in crates/overseer-core/src/lib.rs
- [ ] T022 [US1] Implement `ServiceDescriptor` operation attachment and duplicate operation validation in crates/overseer-core/src/lib.rs
- [ ] T023 [US1] Implement `DaemonBuilder` service registration and duplicate service validation in crates/overseer-core/src/lib.rs
- [ ] T024 [US1] Implement dependency relationship registration and unresolved provider validation in crates/overseer-core/src/lib.rs
- [ ] T025 [US1] Implement `DaemonBuilder::build` to produce immutable `DaemonDefinition` in crates/overseer-core/src/lib.rs
- [ ] T026 [US1] Implement read-only inspection view for components, services, operations, dependencies, and daemon metadata in crates/overseer-core/src/lib.rs
- [ ] T027 [US1] Implement reusable demo daemon fixture with one component-backed service operation in crates/overseer-demo/src/lib.rs
- [ ] T028 [US1] Implement human-readable demo output in crates/overseer-demo/src/main.rs
- [ ] T029 [US1] Wire root facade re-exports for core prototype concepts in src/lib.rs
- [ ] T030 [US1] Run and fix `cargo test -p overseer-core` until US1 core tests pass using crates/overseer-core/src/lib.rs
- [ ] T031 [US1] Run and fix `cargo test -p overseer-demo` until US1 demo tests pass using crates/overseer-demo/src/lib.rs
- [ ] T032 [US1] Run and fix `cargo run -p overseer-demo` until output satisfies quickstart validation in crates/overseer-demo/src/main.rs

**Checkpoint**: User Story 1 is a complete MVP and can be demonstrated independently.

---

## Phase 4: User Story 2 - Maintain strict crate responsibility boundaries (Priority: P2)

**Goal**: Make crate ownership explicit and verify all implementation crates are under `crates/` with one responsibility each.

**Independent Test**: `cargo metadata --no-deps` lists the root facade plus the two implementation crates under `crates/`, and crate documentation states ownership and non-ownership clearly.

### Verification for User Story 2 (REQUIRED) ⚠️

- [ ] T033 [P] [US2] Add workspace boundary validation test for package paths and crate names in crates/overseer-demo/src/lib.rs
- [ ] T034 [P] [US2] Add facade compile test proving root `overseer` exposes core concepts without owning implementation in src/lib.rs
- [ ] T035 [P] [US2] Add manual validation notes for `cargo metadata --no-deps` expected crate paths in specs/001-initial-prototype-crates/quickstart.md

### Implementation for User Story 2

- [ ] T036 [US2] Add explicit responsibility and excluded-responsibility crate docs to crates/overseer-core/src/lib.rs
- [ ] T037 [US2] Add explicit responsibility and excluded-responsibility crate docs to crates/overseer-demo/src/lib.rs
- [ ] T038 [US2] Add explicit facade-only responsibility and excluded-responsibility docs to src/lib.rs
- [ ] T039 [US2] Verify Cargo workspace metadata includes only planned prototype implementation crates under crates/ by running `cargo metadata --no-deps` against Cargo.toml
- [ ] T040 [US2] Run and fix `cargo test -p overseer` until facade boundary checks pass using src/lib.rs

**Checkpoint**: User Story 2 is independently verifiable through workspace metadata and crate documentation.

---

## Phase 5: User Story 3 - Provide a foundation for future runtime, macro, and transport work (Priority: P3)

**Goal**: Ensure the prototype exposes stable seams for future macros, runtime assembly, transports, and client generation without implementing those out-of-scope features.

**Independent Test**: Contracts and tests show future features can consume descriptor metadata and explicit registration without requiring hidden discovery or production transport behavior.

### Verification for User Story 3 (REQUIRED) ⚠️

- [ ] T041 [P] [US3] Add test proving operation input/output contract summaries are inspectable for future client-generation consumers in crates/overseer-core/src/lib.rs
- [ ] T042 [P] [US3] Add test proving explicit registration can be inspected without auto-discovery or macro hooks in crates/overseer-core/src/lib.rs
- [ ] T043 [P] [US3] Add scope guard checklist for deferred runtime, macro, transport, and SDK behavior in specs/001-initial-prototype-crates/contracts/crate-boundaries.md

### Implementation for User Story 3

- [ ] T044 [US3] Add public descriptor accessors needed by future runtime, macro, transport, and client-generation crates in crates/overseer-core/src/lib.rs
- [ ] T045 [US3] Add inspection summary text that names descriptor categories without promising production transport behavior in crates/overseer-core/src/lib.rs
- [ ] T046 [US3] Update core registration contract with future-consumer expectations from implemented descriptors in specs/001-initial-prototype-crates/contracts/core-registration.md
- [ ] T047 [US3] Update inspection output contract with final implemented category names in specs/001-initial-prototype-crates/contracts/inspection-output.md
- [ ] T048 [US3] Run and fix `cargo test --workspace` until all future-seam tests pass across Cargo.toml

**Checkpoint**: User Story 3 is independently verifiable through descriptor accessors, contracts, and scope guards.

---

## Phase 6: Polish & Cross-Cutting Concerns

**Purpose**: Final verification, documentation cleanup, and scope safety across all stories.

- [ ] T049 [P] Update README prototype status and crate layout summary in README.md
- [ ] T050 [P] Update docs/vision.md only if implemented prototype boundaries differ from the current possible architecture section in docs/vision.md
- [ ] T051 Run `cargo fmt --all -- --check` and fix formatting in Cargo.toml and Rust source files
- [ ] T052 Run `cargo test --workspace` and fix remaining failures across Cargo.toml
- [ ] T053 Run `cargo run -p overseer-demo` and confirm quickstart output expectations in specs/001-initial-prototype-crates/quickstart.md
- [ ] T054 Run final scope review against contracts/crate-boundaries.md to confirm no production transport, macros, SDK generation, persistence, credentials, or network behavior was added in specs/001-initial-prototype-crates/contracts/crate-boundaries.md

---

## Dependencies & Execution Order

### Phase Dependencies

- **Setup (Phase 1)**: No dependencies; start immediately.
- **Foundational (Phase 2)**: Depends on Setup completion; blocks all user stories.
- **User Story 1 (Phase 3)**: Depends on Foundational completion; delivers MVP.
- **User Story 2 (Phase 4)**: Depends on Foundational completion; can start after or alongside US1 once workspace skeleton exists, but final checks depend on actual crate files.
- **User Story 3 (Phase 5)**: Depends on Foundational completion and should follow US1 descriptor implementation for accurate future-facing accessors.
- **Polish (Phase 6)**: Depends on all desired user stories being complete.

### User Story Dependencies

- **User Story 1 (P1)**: Starts after Phase 2; no dependency on US2 or US3.
- **User Story 2 (P2)**: Starts after Phase 2; independent of US1 behavior except final workspace metadata reflects implemented crates.
- **User Story 3 (P3)**: Starts after Phase 2, but tasks T044-T048 are most useful after US1 descriptors exist.

### Within Each User Story

- Verification tasks come before implementation tasks.
- Descriptor data structures come before registration behavior.
- Registration behavior comes before inspection output.
- Demo fixture comes before runnable demo output.
- Story-level command validation completes each story checkpoint.

### Parallel Opportunities

- Setup tasks T002 and T004 can be prepared in parallel after T001 if editing different crate manifests.
- Foundational descriptor definitions T010, T011, T012, and T013 can be drafted in parallel before integration in T014 and T015.
- US1 verification tasks T017, T018, T019, and T020 can be written in parallel.
- US2 documentation and verification tasks T033, T034, T035, T036, T037, and T038 touch different files and can be parallelized.
- US3 verification and contract tasks T041, T042, T043, T046, and T047 can be parallelized when their dependent descriptor names are known.
- Polish documentation tasks T049 and T050 can be parallelized.

---

## Parallel Example: User Story 1

```bash
# Launch US1 verification work in parallel before implementation:
Task: "Add failing core registration success test for component/service/operation/dependency assembly in crates/overseer-core/src/lib.rs"
Task: "Add failing core validation tests for duplicate IDs, empty fields, and missing dependency providers in crates/overseer-core/src/lib.rs"
Task: "Add failing demo fixture test asserting required inspection categories in crates/overseer-demo/src/lib.rs"
Task: "Add failing runnable demo output test or validation helper for daemon/component/service/operation/dependency text in crates/overseer-demo/src/lib.rs"

# Then implement core behavior before demo behavior:
Task: "Implement `DaemonBuilder` component registration and duplicate component validation in crates/overseer-core/src/lib.rs"
Task: "Implement `ServiceDescriptor` operation attachment and duplicate operation validation in crates/overseer-core/src/lib.rs"
Task: "Implement reusable demo daemon fixture with one component-backed service operation in crates/overseer-demo/src/lib.rs"
```

## Parallel Example: User Story 2

```bash
Task: "Add explicit responsibility and excluded-responsibility crate docs to crates/overseer-core/src/lib.rs"
Task: "Add explicit responsibility and excluded-responsibility crate docs to crates/overseer-demo/src/lib.rs"
Task: "Add explicit facade-only responsibility and excluded-responsibility docs to src/lib.rs"
```

## Parallel Example: User Story 3

```bash
Task: "Add test proving operation input/output contract summaries are inspectable for future client-generation consumers in crates/overseer-core/src/lib.rs"
Task: "Add scope guard checklist for deferred runtime, macro, transport, and SDK behavior in specs/001-initial-prototype-crates/contracts/crate-boundaries.md"
Task: "Update inspection output contract with final implemented category names in specs/001-initial-prototype-crates/contracts/inspection-output.md"
```

---

## Implementation Strategy

### MVP First (User Story 1 Only)

1. Complete Phase 1: Setup.
2. Complete Phase 2: Foundational descriptor vocabulary and builder skeleton.
3. Complete Phase 3: User Story 1.
4. Stop and validate with `cargo test -p overseer-core`, `cargo test -p overseer-demo`, and `cargo run -p overseer-demo`.
5. Demo if ready; do not proceed into broader runtime, macro, transport, or SDK work.

### Incremental Delivery

1. Setup + Foundational creates the bounded workspace and descriptor vocabulary.
2. US1 proves the README-derived vertical slice and is the MVP.
3. US2 hardens crate-boundary documentation and workspace validation.
4. US3 confirms future extension seams without implementing out-of-scope features.
5. Polish validates quickstart, formatting, tests, and scope guard.

### Parallel Team Strategy

With multiple developers:

1. One developer performs Phase 1 workspace setup.
2. Core descriptor definitions in Phase 2 can be split by descriptor type, then integrated through `DaemonBuilder`.
3. After Phase 2, one developer can complete US1 core/demo behavior, another can prepare US2 boundary docs/tests, and another can update US3 contract/scope guard tasks.
4. Final validation must be serialized through `cargo fmt --all -- --check`, `cargo test --workspace`, and `cargo run -p overseer-demo`.

---

## Notes

- Tests are included because the constitution requires automated tests when behavior is programmatically checkable.
- [P] tasks are parallelizable only when assigned to different files or independent sections without unresolved dependencies.
- [US1], [US2], and [US3] labels map directly to the prioritized user stories in spec.md.
- Keep changes scoped to files named in tasks unless an implementation task proves another file is strictly necessary.
- Do not add third-party dependencies unless the plan is amended with rationale and simpler alternatives considered.
