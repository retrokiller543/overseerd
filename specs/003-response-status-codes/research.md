# Phase 0 Research: Response Status Codes

This document resolves the design questions the spec deferred to planning and
records the rationale for each decision. The two product-level forks (three-section
code; errors-only) were already settled with the user during `/speckit-specify`;
the items below are the implementation-level unknowns.

## Decision 1 — Status code width and section layout

**Decision**: A packed `u32` with three non-overlapping sections:

```text
 31      24 23      16 15                   0
+----------+----------+----------------------+
|  flags   | predef.  |       custom         |
|  (u8)    |  (u8)    |       (u16)          |
+----------+----------+----------------------+
  bitflags   discrete       app-owned value
```

- **flags** (high byte, bits 24–31): framework-defined, combinable bitflags for
  control flow (`RETRYABLE`, room for 7 more).
- **predefined** (bits 16–23): framework-owned discrete category (256 values).
- **custom** (low 16 bits): application-owned opaque value (65 536 values).

**Rationale**: The user described the code growing from "two u8" to "three or
four u8" with the three named parts (flags / our codes / custom). `u32` is the
natural Rust width, gives the custom section the most room (16 bits), keeps each
section byte/half-word aligned for trivial mask-and-shift accessors, and serializes
compactly under postcard (varint). Flags get a full byte so future control-flow
signals don't require a width change.

**Alternatives considered**:
- *`u16`, two sections* (the original sketch): no room for a separate flags
  section once predefined + custom both need space. Rejected — the user expanded
  the model to three sections.
- *Three packed bytes (24-bit)*: no native Rust 24-bit integer; would need a
  custom (de)serialize and awkward alignment. Rejected for `u32`.
- *Struct of three fields instead of a packed int*: clearer in source but heavier
  on the wire and loses the "single integer the client can match/mask" ergonomic
  the user asked for. Rejected; the packed `u32` is exposed through a `StatusCode`
  newtype with named accessors, so source stays readable without paying the wire
  cost.

## Decision 2 — Where the wire-contract types live

**Decision**: `StatusCode` (the `u32` newtype + section accessors), the `Flags`
bitflag constants, and the `PredefinedCode` catalog live in **`crates/transport`**
(`status.rs`). `ErrorResponse { code, body }` and the refactored
`IntoErrorResponse` trait live in **`crates/core`** (`extract.rs`). `core`
re-exports `StatusCode` so handler authors import everything from `overseer_core`.

**Rationale**: The status code is the on-the-wire contract; transport already owns
`WireOutcome` and `CallResult`, so the canonical definition belongs beside them.
This keeps the rich semantics available to any wire peer — including a future
generated client SDK (TODO #4) that may not depend on `core`'s handler machinery.
`IntoErrorResponse` is purely a server-side handler ergonomic, so it stays in
`core`.

**Alternatives considered**:
- *Everything in `core`*: transport would carry a bare `u32`/`Vec<u8>` and clients
  would need `core` to interpret codes. Rejected — couples wire interpretation to
  the handler crate and complicates the future SDK.
- *Everything in `transport`*: would drag the handler-facing `IntoErrorResponse`
  (which references `Serialize` bodies and the dispatch path) down into the
  transport layer. Rejected — violates the layering; transport stays
  semantics-free except for the code catalog it must define as the contract.

## Decision 3 — Flags representation (dependency question)

**Decision**: Hand-roll the flags as associated `const` values on `StatusCode`
(or a small `Flags(u8)` newtype) with bitwise `set`/`contains` helpers. **No new
dependency.**

**Rationale**: Project rule is to add dependencies sparingly and ask first; a
single flag (`RETRYABLE`) plus a couple of bitwise helpers does not justify a
crate. The flags byte is small and fully under our control.

**Alternatives considered**:
- *`bitflags` crate*: ergonomic and well-known, but an unnecessary dependency for
  one byte of flags. Recorded as the rejected alternative; revisit only if the
  flag set grows large enough to want the macro ergonomics.

## Decision 4 — The error currency on the dispatch path

**Decision**: Change the erased dispatch error arm from `crate::Error` to
`ErrorResponse { code: StatusCode, body: Vec<u8> }`. The handler traits resolve as:

- `FallibleHandler::call` maps `Err(e)` through `e.into_error_response()` →
  `ErrorResponse` (preserving custom code + body).
- Extractor failures (`FromContext` returning `crate::Result`) convert at the `?`
  boundary via `crate::Error: IntoErrorResponse` → `ErrorResponse`.
- `crate::Error` gains an `IntoErrorResponse` impl mapping each variant to a
  predefined code (see Decision 5), so internal/extractor failures get sensible
  categories for free and existing `Result<T, E>` handlers need no change.

**Rationale**: To carry a *custom body* (FR-004), the error value must survive to
the wire instead of collapsing to `e.to_string()` in `drive_call`. Making
`ErrorResponse` the error type of the dispatch result is the single change that
preserves both code and body across unary and streaming paths while keeping the
blanket `E: Into<Error>` ergonomics working (those just map to the default code).

**Alternatives considered**:
- *Add `code` + `body` to `crate::Error`*: pollutes the framework's internal error
  enum with per-call response bodies and conflates "framework error" with "handler
  response". Rejected.
- *Fold the error into `RpcOutcome` (a third variant)*: makes every success-path
  match handle an error case it never produces. Rejected; `Result<RpcOutcome,
  ErrorResponse>` keeps success and failure cleanly separated.

## Decision 5 — Predefined catalog and mapping of existing errors

**Decision**: Initial predefined catalog (the framework-owned byte):

| Code | Meaning | Maps from `crate::Error` variants |
| --- | --- | --- |
| `Internal` (default) | Unclassified server error | `Serialization`, `MissingExtension`, `MissingComponent`, `Transport`, and any unmapped variant |
| `BadInput` | Malformed/invalid request | `InvalidPayload`, `NotStreaming` |
| `NotFound` | No such route/resource | `RouteNotFound` |
| `Unauthorized` | Caller not permitted | (none yet; reserved for handler use) |

The default (unset predefined byte / `0`) maps to `Internal`. Registry-validation
variants (`DuplicateComponentId`, `DependencyCycle`, …) occur at build time, not
call time, so they need no runtime mapping but fall through to `Internal` if ever
surfaced.

**Rationale**: Satisfies FR-007 (catalog with at least invalid input, internal,
not found, unauthorized) and SC-002 (every existing call-time error variant maps
to a category). Keeping the set minimal avoids speculative categories; the byte has
room to grow.

**Alternatives considered**:
- *Mirror the full gRPC status set (16 codes)*: more complete but speculative for a
  prototype; most would be unused. Rejected in favor of a minimal, extensible set.

## Decision 6 — Streaming error parity

**Decision**: `StreamError { id, message: String }` becomes
`StreamError { id, code, body }`; `ResponseSink::error` takes the code + body
instead of a `String`; `RpcOutcome::Stream`'s item error type becomes
`ErrorResponse`. The in-memory transport's `ServerEvent::Error` carries the same.

**Rationale**: FR-010/SC-006 require identical error shape across unary and
streaming. A handler stream yielding `Err` flows through the same
`IntoErrorResponse` path as a unary error.

**Alternatives considered**:
- *Leave `StreamError` as a string*: would force consumers to handle two different
  error shapes. Rejected.

## Decision 7 — Forward compatibility for unknown predefined codes

**Decision**: `PredefinedCode` is represented as a `u8` behind the `StatusCode`
newtype, with a `predefined()` accessor returning a small enum that includes an
`Unknown(u8)` catch-all (or returns the raw `u8` with named constants). Decoding
never fails on an unrecognized predefined byte.

**Rationale**: FR-009 — a client built against an older framework version must not
fail to parse a response carrying a newer predefined code. Representing the wire
value as a `u8` (not a closed enum that postcard could reject) guarantees this.

**Alternatives considered**:
- *Closed `enum` (de)serialized directly*: postcard would error on an out-of-range
  discriminant, breaking forward compat. Rejected — keep the wire value a `u8`.
