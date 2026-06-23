---
description: "Task list for Response Status Codes"
---

# Tasks: Response Status Codes

**Input**: Design documents from `/specs/003-response-status-codes/`

**Prerequisites**: plan.md, spec.md, research.md, data-model.md, contracts/

**Verification**: Each user story has automated verification via `MemoryTransport`
round-trips (root `tests/`, matching `tests/streaming.rs`) plus inline unit tests
for the bit-level `StatusCode` API. Manual end-to-end validation runs through the
extended `src/bin/example.rs` (daemon started by the user, per project convention).

**Organization**: Tasks are grouped by user story. The wire/dispatch rewiring is
**Foundational** — until it lands the workspace will not compile, so it blocks all
stories. Each story then adds an independently testable slice on top.

## Format: `[ID] [P?] [Story] Description`

- **[P]**: Can run in parallel (different files, no dependencies on incomplete tasks)
- **[Story]**: US1 / US2 / US3 (Setup/Foundational/Polish carry no story label)
- File paths are exact and relative to the repository root.

## Path Conventions

Rust workspace: `crates/transport`, `crates/core`, `crates/macros`; umbrella crate
+ example in `src/`; integration tests in root `tests/`.

---

## Phase 1: Setup (Shared Infrastructure)

**Purpose**: Land the wire-contract type skeleton everything else builds on.

- [X] T001 Create `crates/transport/src/status.rs` with the `StatusCode(u32)` newtype — `#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]`, plus `raw(self) -> u32` and `from_raw(u32) -> Self` — and register `pub mod status;` in `crates/transport/src/lib.rs`.

---

## Phase 2: Foundational (Blocking Prerequisites)

**Purpose**: Rewire the error path end-to-end so error responses carry
`{ code, body }` (default `Internal`, body = error `Display`), the success path is
untouched, and the whole workspace compiles and `cargo test` is green again.

**⚠️ CRITICAL**: No user story work can begin until this phase is complete — the
wire-type changes (T003–T005) break compilation across every consumer until
T006–T017 restore it.

- [X] T002 Add `PredefinedCode` to `crates/transport/src/status.rs`: variants `Internal=0`, `BadInput=1`, `NotFound=2`, `Unauthorized=3`, and `Unknown(u8)` for any other byte; add `StatusCode::predefined(self) -> PredefinedCode`, `impl From<PredefinedCode> for StatusCode`, and `impl Default for StatusCode` (→ `Internal`). Decoding any byte MUST be total (FR-009).
- [X] T003 Change `WireOutcome::Err(String)` to `Err { code: StatusCode, body: Vec<u8> }` and update `WireResponse::new` to map from the new `CallResult::Err` in `crates/transport/src/protocol/mod.rs` (leave `Ok` arm unchanged — FR-013).
- [X] T004 Change `CallResult::Err(String)` to `Err { code: StatusCode, body: Vec<u8> }` in `crates/transport/src/frame.rs`.
- [X] T005 Change `WireMessage::StreamError { id, message: String }` to `{ id, code: StatusCode, body: Vec<u8> }` in `crates/transport/src/protocol/mod.rs`, and change `ResponseSink::error(self, message: String)` to `error(self, code: StatusCode, body: Vec<u8>)` in `crates/transport/src/transport.rs`.
- [X] T006 Update `crates/transport/src/transports/stream.rs`: `StreamResponder::respond` (new `CallResult::Err`), `StreamSink::error` (build `StreamError { id, code, body }`), and the read-loop's `StreamError` match arm.
- [X] T007 Update `crates/transport/src/transports/memory.rs`: change `ServerEvent::Error(String)` to `Error { code, body }`, and update `response()`, the `error()` sink impl, and `respond()` accordingly.
- [X] T008 Re-export `StatusCode` and `PredefinedCode` from `crates/transport/src/lib.rs`.
- [X] T009 Add `ErrorResponse { code: StatusCode, body: Vec<u8> }` with `ErrorResponse::new(code, body)` in `crates/core/src/extract.rs`.
- [X] T010 Refactor the `IntoErrorResponse` trait in `crates/core/src/extract.rs` to the Actix-`ResponseError` shape: `fn status_code(&self) -> StatusCode` (default `Internal`) and `fn error_response(self) -> ErrorResponse` (default: serialize `Display` to `body`, attach `status_code()`); retain the blanket `impl<E: Into<Error>>` so existing `Result<T, E>` handlers still satisfy it (FR-005, FR-006).
- [X] T011 Change the dispatch error currency to `ErrorResponse` in `crates/core/src/extract.rs` and `crates/core/src/descriptors/service/rpc.rs`: `Handler`/`FallibleHandler::call` and `dispatch_with`/`dispatch_fallible` resolve to `Result<RpcOutcome, ErrorResponse>`; `FromContext` `?`-failures convert via `Error: IntoErrorResponse`; `RpcOutcome::Stream`'s item error type becomes `ErrorResponse`; update `ResponseStream::respond` and `Streaming<T>` error conversion to match.
- [X] T012 Add a baseline `impl IntoErrorResponse for crate::Error` (all variants → `Internal` for now; refined in US1) in `crates/core/src/error.rs` so extractor `?` and dispatch compile.
- [X] T013 Update `RpcRouter::dispatch` in `crates/core/src/router.rs` for the new error type, mapping `RouteNotFound` through the error path (refined category mapping lands in US1).
- [X] T014 Update `drive_call` in `crates/core/src/daemon.rs`: the unary `Err` arm builds `CallResult::Err { code, body }` from the `ErrorResponse`; the stream `Some(Err(e))` arm calls `sink.error(code, body)`.
- [X] T015 Re-export `ErrorResponse`, `IntoErrorResponse`, `StatusCode`, and `PredefinedCode` from `crates/core/src/lib.rs` (and surface via `src/lib.rs`).
- [X] T016 Update `unpack()` in `src/bin/example.rs` to match `WireOutcome::Err { code, body }` (restores the example to compiling; richer reporting added in US1).
- [X] T017 Build the whole workspace and run the existing suite to confirm the rewiring is green: `cargo build && cargo test` (verify `crates/macros/src/handlers.rs` still compiles against the refactored trait; adjust only doc/path references if needed).

**Checkpoint**: Workspace compiles, existing tests pass, every error response carries `Internal` + a Display-derived body, success path unchanged.

---

## Phase 3: User Story 1 - Typed status code and structured body on error responses (Priority: P1) 🎯 MVP

**Goal**: A handler error carries a predefined category + an arbitrary serializable body; the consumer reads both. Existing `Result<T, E>` handlers keep working, mapped to a sensible category.

**Independent Test**: Return a custom error mapping to a predefined code with a structured body; assert the client reads the exact code and deserializes the exact body. Separately assert an unchanged framework-error handler still works.

### Verification for User Story 1 ⚠️

- [X] T018 [P] [US1] Integration test in `tests/status_codes.rs`: a handler returning a custom error type produces `WireOutcome::Err { code, body }`; the client decodes the exact `PredefinedCode` and deserializes the exact body (drive via `MemoryTransport`, mirroring `tests/streaming.rs`). (SC-001)
- [X] T019 [P] [US1] Integration test in `tests/status_codes.rs`: a handler returning `Result<T, overseerd_core::Error>` still compiles and maps to its mapped predefined category (regression). (SC-003)
- [X] T020 [P] [US1] Unit test in `crates/core/src/extract.rs` (`#[cfg(test)]`): when the body fails to serialize, `error_response`/`drive_call` still yields the intended `code` with a fallback body and logs the failure. (FR-011)
- [X] T021 [P] [US1] Unit test in `crates/transport/src/status.rs` (`#[cfg(test)]`): decoding a `StatusCode` whose predefined byte is unrecognized yields `PredefinedCode::Unknown(u8)`, never a parse error. (FR-009)

### Implementation for User Story 1

- [X] T022 [US1] Refine `impl IntoErrorResponse for crate::Error` in `crates/core/src/error.rs` to map variants to categories: `InvalidPayload`/`NotStreaming` → `BadInput`, `RouteNotFound` → `NotFound`, all others → `Internal` (SC-002).
- [X] T023 [US1] Implement the FR-011 fallback-and-log path in the default `IntoErrorResponse::error_response` (and/or the body build in `crates/core/src/daemon.rs`), preserving `code` and emitting a `warn!` on serialization failure.
- [X] T024 [US1] Extend `src/bin/example.rs`: add a custom error type (e.g. `GreetError`) implementing `IntoErrorResponse` with a predefined code + structured body, a handler that returns it, and update the client path to print the decoded `PredefinedCode` and deserialized body instead of `panic!("RPC error: {e}")`.

**Checkpoint**: US1 fully functional and independently testable — errors are classified and carry structured bodies; existing handlers untouched.

---

## Phase 4: User Story 2 - Custom application error codes that cannot collide with framework codes (Priority: P2)

**Goal**: Applications set a value in the custom section; it round-trips intact and provably cannot touch the predefined or flags sections.

**Independent Test**: Set `with_custom(x)`; assert the consumer reads `x` back and the predefined + flags bytes are unchanged; assert there is no API to write the predefined byte via the custom path.

### Verification for User Story 2 ⚠️

- [X] T025 [P] [US2] Unit test in `crates/transport/src/status.rs` (`#[cfg(test)]`): `with_custom(x).custom() == x`; `with_custom` leaves `predefined()` and the flags byte unchanged; values > `u16::MAX` are impossible by type and the low 16 bits never overflow into bits ≥16 (FR-003).
- [X] T026 [P] [US2] Integration test in `tests/status_codes.rs`: an error carrying both a predefined code and a custom subcode round-trips both sections intact to the client (SC-005).

### Implementation for User Story 2

- [X] T027 [US2] Add `StatusCode::with_custom(self, u16) -> Self` (writes only bits 0–15) and `StatusCode::custom(self) -> u16` (masks bits 0–15) in `crates/transport/src/status.rs`; confirm no public API can set the predefined byte except via `PredefinedCode`.
- [X] T028 [US2] Set a custom subcode on the example error type in `src/bin/example.rs` and print it on the client side.

**Checkpoint**: US1 and US2 both work independently — custom codes coexist with framework categories without collision.

---

## Phase 5: User Story 3 - Control-flow flags carried alongside the code (Priority: P3)

**Goal**: Errors carry combinable flags (e.g. `RETRYABLE`) in the flags byte; a consumer branches on a single bit test without deserializing the body.

**Independent Test**: Return an error marked `RETRYABLE`; assert the consumer detects it via `contains` with no body deserialization, and that two flags coexist independently.

### Verification for User Story 3 ⚠️

- [X] T029 [P] [US3] Unit test in `crates/transport/src/status.rs` (`#[cfg(test)]`): `with_flag(RETRYABLE).contains(RETRYABLE)` is true; two flags set together both read as set; setting flags leaves `predefined()` and `custom()` unchanged (FR-012).
- [X] T030 [P] [US3] Integration test in `tests/status_codes.rs`: a retryable error round-trips so the client detects `RETRYABLE` from the code alone (SC-004).

### Implementation for User Story 3

- [X] T031 [US3] Add the flags API to `crates/transport/src/status.rs`: `RETRYABLE` constant (flags-byte bit 0), `StatusCode::with_flag(self, flag) -> Self`, `StatusCode::contains(self, flag) -> bool`, and `StatusCode::flags(self) -> u8`; re-export the flag constant from `crates/transport/src/lib.rs`.
- [X] T032 [US3] Mark the example error `RETRYABLE` and have the client branch on it (e.g. print "retryable") in `src/bin/example.rs`.

**Checkpoint**: All three stories independently functional.

---

## Phase 6: Polish & Cross-Cutting Concerns

- [X] T033 [P] Add brief class-level doc comments (per project style) to `StatusCode`, `PredefinedCode`, `ErrorResponse`, and the `IntoErrorResponse` trait across `crates/transport/src/status.rs` and `crates/core/src/extract.rs`; ensure no stray inline comments.
- [X] T034 [P] Update `crates/transport/src/protocol/mod.rs` and `frame.rs` doc comments to describe the new `{ code, body }` error shape.
- [X] T035 Mark TODO item 3 complete in `TODO.md`.
- [X] T036 Run `specs/003-response-status-codes/quickstart.md` validation: `cargo test` green, then ask the user to run the example daemon + client and confirm the client prints a classified error (code + flag + body).

---

## Dependencies & Execution Order

### Phase Dependencies

- **Setup (Phase 1)**: No dependencies — start immediately.
- **Foundational (Phase 2)**: Depends on Setup. **Blocks all user stories** — the workspace does not compile until T017 passes.
- **User Stories (Phase 3–5)**: All depend on Foundational. Once it is green they can proceed in parallel or in priority order (P1 → P2 → P3).
- **Polish (Phase 6)**: Depends on the desired user stories being complete.

### User Story Dependencies

- **US1 (P1)**: Depends only on Foundational. Delivers the MVP (classified errors + structured bodies).
- **US2 (P2)**: Depends only on Foundational (adds `with_custom`/`custom`). Independently testable; does not require US1.
- **US3 (P3)**: Depends only on Foundational (adds the flags API). Independently testable; does not require US1/US2.

> US2 and US3 each add isolated methods to `status.rs` and isolated lines to the
> example; they touch the same two files, so run their *implementation* tasks
> sequentially if worked in parallel to avoid edit conflicts, but their logic is
> independent.

### Within Each User Story

- Verification tasks are written before implementation; automated tests should fail first.
- `status.rs` API additions precede the example wiring that uses them.

### Parallel Opportunities

- Transport-layer foundational edits across distinct files can overlap, but T003 and T005 both touch `protocol/mod.rs` (sequence them).
- All `[P]` verification tasks within a story run in parallel (distinct test files / distinct `#[cfg(test)]` modules).
- With multiple developers, US1/US2/US3 can be staffed concurrently once Foundational is green.

---

## Parallel Example: User Story 1

```bash
# Launch all US1 verification together (distinct files):
Task: "Integration test: custom error code+body round-trip in tests/status_codes.rs"          # T018
Task: "Integration test: existing Result<T, Error> handler regression in tests/status_codes.rs" # T019
Task: "Unit test: body-serialization fallback in crates/core/src/extract.rs"                   # T020
Task: "Unit test: unknown predefined byte -> Unknown(u8) in crates/transport/src/status.rs"     # T021
```

---

## Implementation Strategy

### MVP First (User Story 1 only)

1. Phase 1: Setup (T001).
2. Phase 2: Foundational (T002–T017) — **critical, blocks everything**; ends with a green `cargo test`.
3. Phase 3: User Story 1 (T018–T024).
4. **STOP and VALIDATE**: classified errors with structured bodies, existing handlers unchanged.

### Incremental Delivery

1. Setup + Foundational → workspace green, errors carry code+body (Internal default).
2. US1 → classified categories + structured bodies + example (MVP).
3. US2 → custom codes with provable section isolation.
4. US3 → control-flow flags (retryable).
5. Polish → docs, TODO update, quickstart validation.

---

## Notes

- `[P]` tasks = different files, no incomplete-task dependencies.
- The Foundational phase is deliberately large because the wire-type change is a
  hard compile boundary — splitting it across stories would leave the tree
  uncompilable between stories, violating independent testability.
- Per project convention, new behavior is demonstrated in the existing
  `src/bin/example.rs`, and the long-running daemon is started by the user, not by
  the implementing agent.
- Commit after each task or logical group; checkpoint after each story.
