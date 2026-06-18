# Implementation Plan: Response Status Codes

**Branch**: `003-response-status-codes` | **Date**: 2026-06-18 | **Spec**: [spec.md](./spec.md)

**Input**: Feature specification from `/specs/003-response-status-codes/spec.md`

## Summary

Give RPC **error responses** a machine-readable status code and an arbitrary
serializable body, replacing today's lossy `WireOutcome::Err(String)`. The status
code is a packed `u32` partitioned into three non-overlapping sections — a
framework-defined **flags** byte (combinable bitflags, e.g. `RETRYABLE`), a
framework-owned **predefined code** byte (`Internal`, `BadInput`, `NotFound`,
`Unauthorized`), and a 16-bit **custom** section owned by the application. The
handler-facing `IntoErrorResponse` trait is refactored to the Actix
`ResponseError` shape: an error type declares both its status code and its body.
The error arm of the dispatch path changes from `crate::Error` to a structured
`ErrorResponse { code, body }`, and `crate::Error` gains an `IntoErrorResponse`
impl so existing handlers and internal failures map onto predefined categories
with zero source changes. Success responses are untouched.

## Technical Context

**Language/Version**: Rust (edition 2024, workspace toolchain as pinned in repo)

**Primary Dependencies**: `serde` + `postcard` (wire encoding), `tokio` +
`futures` + `tokio-stream` + `tokio-util` (async/streaming), `thiserror` (error
types). **No new runtime dependency**: the flags section is hand-rolled with
associated consts + bitwise ops rather than pulling in `bitflags` (justified
below; `bitflags` recorded as the rejected alternative).

**Storage**: N/A (no persistence)

**Testing**: `cargo test` — unit tests per crate plus the in-memory transport
(`MemoryTransport`) for fast, deterministic round-trip tests of the error path;
the existing `src/bin/example.rs` demonstrates the feature end-to-end.

**Target Platform**: Linux/macOS daemons over TCP and Unix-socket transports
(and the in-memory transport for tests).

**Project Type**: Rust library/framework workspace (`crates/core`,
`crates/transport`, `crates/macros`) with a root example crate (`src/`).

**Performance Goals**: No regression on the unary success path (unchanged). The
error path adds a `u32` field and reuses the existing postcard body encoding —
negligible overhead. Flag checks are single bitwise ops.

**Constraints**: Must not add `rsa`/`openssl` to the dependency tree (project
rule). Must not break the success path. Breaking wire change to the error arm is
acceptable (pre-1.0 prototype, no deployed peers) but must be applied across all
in-tree transports in lockstep.

**Scale/Scope**: Touches the error path in two crates plus the example. ~8 files
modified, 1–2 new modules. Estimated small-to-medium change; no story requires
the others to land first beyond the P1 foundation.

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

- **Spec traceability**: PASS. Three prioritized user stories (typed code+body;
  custom codes; flags), acceptance scenarios, edge cases, 13 functional
  requirements, and measurable success criteria are all present in spec.md. Both
  open design forks were resolved with the user before drafting.
- **Minimal scope**: PASS. The change is confined to the error path
  (`WireOutcome::Err`, `CallResult::Err`, `StreamError`, `ResponseSink::error`,
  `IntoErrorResponse`, dispatch error arm) plus the example. It necessarily spans
  `transport` and `core` because the wire contract and the handler ergonomics are
  two halves of the same path; this is feature-intrinsic, not unrelated
  refactoring. The success path is explicitly out of scope.
- **Testable increments**: PASS. US1/US2/US3 each have an independent verification
  method (round-trip a code+body; assert custom-section isolation; assert flag
  detection). Automated tests use `MemoryTransport`; the example provides manual
  end-to-end validation.
- **Explicit contracts**: PASS. The wire-format change (`WireOutcome::Err` and
  `StreamError` gaining `{ code, body }`) and the trait change are documented in
  `contracts/` and `data-model.md`, with the breaking-change/compatibility note.
- **Operational safety**: PASS. Body-serialization failure degrades to a fallback
  body while preserving the code and logs the failure (FR-011). Default error
  bodies derive from the existing error `Display`, not internal state, so no new
  secret-leakage surface is introduced.

**Result**: All gates pass. No entries required in Complexity Tracking.

## Project Structure

### Documentation (this feature)

```text
specs/003-response-status-codes/
├── plan.md              # This file (/speckit-plan output)
├── research.md          # Phase 0 output
├── data-model.md        # Phase 1 output
├── quickstart.md        # Phase 1 output
├── contracts/           # Phase 1 output
│   ├── wire-protocol.md     # WireOutcome::Err / StreamError / StatusCode layout
│   └── handler-api.md       # IntoErrorResponse / ErrorResponse / StatusCode API
├── checklists/
│   └── requirements.md  # Spec quality checklist (from /speckit-specify)
└── tasks.md             # Phase 2 output (/speckit-tasks — NOT created here)
```

### Source Code (repository root)

```text
crates/
├── transport/
│   └── src/
│       ├── status.rs          # NEW: StatusCode (u32 newtype), Flags, PredefinedCode catalog
│       ├── frame.rs           # CHANGE: CallResult::Err { code, body } (was Err(String))
│       ├── protocol/mod.rs    # CHANGE: WireOutcome::Err { code, body }; StreamError { id, code, body }; WireResponse::new
│       ├── transport.rs       # CHANGE: ResponseSink::error(code, body) signature
│       ├── transports/
│       │   ├── stream.rs      # CHANGE: StreamResponder/StreamSink error frame construction
│       │   └── memory.rs      # CHANGE: ServerEvent::Error { code, body }; response()/error() mapping
│       └── lib.rs             # CHANGE: export StatusCode, Flags, PredefinedCode
├── core/
│   └── src/
│       ├── extract.rs         # CHANGE: refactor IntoErrorResponse -> status_code()+body; add ErrorResponse; FallibleHandler/Handler error arm
│       ├── error.rs           # CHANGE: impl IntoErrorResponse for Error (variant -> predefined code)
│       ├── descriptors/service/rpc.rs # CHANGE: RpcHandler / RpcOutcome::Stream error type -> ErrorResponse
│       ├── router.rs          # CHANGE: dispatch error arm; RouteNotFound -> NotFound code
│       ├── daemon.rs          # CHANGE: drive_call Err arms build CallResult::Err{code,body} / sink.error(code,body); FR-011 fallback+log
│       └── lib.rs             # CHANGE: re-export ErrorResponse, IntoErrorResponse, StatusCode et al.
└── macros/
    └── src/handlers.rs        # REVIEW: dispatch selection unchanged; verify trait-shape change compiles

src/
└── bin/example.rs             # CHANGE: custom error type w/ code+body+RETRYABLE; unpack() reads new Err shape
```

**Structure Decision**: Existing Rust workspace. Wire-contract types
(`StatusCode`, `Flags`, `PredefinedCode`) live in `crates/transport` because they
*are* the wire contract and transport already owns `WireOutcome`/`CallResult`.
Handler-facing ergonomics (`ErrorResponse`, refactored `IntoErrorResponse`) live
in `crates/core` and re-export `StatusCode`. The `macros` crate is expected to
need no logic change — only verification that the refactored trait still binds.

## Complexity Tracking

> No Constitution Check violations. Section intentionally empty.
