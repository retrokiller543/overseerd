# Phase 1 Data Model: Response Status Codes

Entities and their fields, relationships, and validation rules. Wire-level types
live in `crates/transport`; handler-facing types live in `crates/core`.

## StatusCode  *(transport)*

A newtype over the packed `u32` wire value. The single source of truth for the
three-section layout.

| Aspect | Detail |
| --- | --- |
| Representation | `struct StatusCode(u32)` — `#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]` |
| Section: flags | bits 24–31 (`u8`), combinable bitflags, framework-defined |
| Section: predefined | bits 16–23 (`u8`), discrete framework code |
| Section: custom | bits 0–15 (`u16`), application-owned opaque value |

**Accessors / constructors** (no panics; pure bit ops):
- `predefined(self) -> PredefinedCode` — masks bits 16–23.
- `custom(self) -> u16` — masks bits 0–15.
- `flags(self) -> u8` (raw) and `contains(self, flag) -> bool`.
- `with_predefined(self, PredefinedCode) -> Self`, `with_custom(self, u16) -> Self`,
  `with_flag(self, flag) -> Self` — builder-style, each writes only its section.
- `raw(self) -> u32` / `from_raw(u32) -> Self` — wire conversion.

**Validation / invariants**:
- The predefined section is **only** writable via `with_predefined` (framework
  API); `with_custom` masks to 16 bits and can never touch bits ≥16 (FR-003).
- Decoding any `u32` is total — an unrecognized predefined byte yields
  `PredefinedCode::Unknown(u8)`, never an error (FR-009).

## PredefinedCode  *(transport)*

Framework-owned category occupying the predefined byte.

| Variant | Byte value | Notes |
| --- | --- | --- |
| `Internal` | `0` | Default for unset/unmapped; catch-all server error |
| `BadInput` | `1` | Malformed/invalid request |
| `NotFound` | `2` | No such route/resource |
| `Unauthorized` | `3` | Caller not permitted |
| `Unknown(u8)` | other | Forward-compat catch-all for codes a peer doesn't know |

**Relationships**: produced from `crate::Error` variants by the `IntoErrorResponse`
impl in `core` (see research Decision 5). `Internal` is the default when no code is
set.

## Flags  *(transport)*

Constants on the flags byte; combinable with bitwise OR.

| Flag | Bit | Meaning |
| --- | --- | --- |
| `RETRYABLE` | `0` (bit 24 of the `u32`) | Caller may safely retry |

**Validation**: multiple flags may be set simultaneously; each is independently
testable via `StatusCode::contains` (FR-012). Room for 7 more flags without a width
change.

## ErrorResponse  *(core)*

The handler-side error currency and the value carried to the wire error arm.

| Field | Type | Notes |
| --- | --- | --- |
| `code` | `StatusCode` | The full packed status |
| `body` | `Vec<u8>` | Serialized error body (postcard); may be empty |

**Construction**:
- `ErrorResponse::new(code, body)`.
- Produced by `IntoErrorResponse::into_error_response` (default impl serializes the
  error's `Display` string as the body and uses the trait's `status_code`).

**Validation rule (FR-011)**: if body serialization fails while building an
`ErrorResponse`, construction falls back to a body derived from the error message
(or empty), **preserves `code`**, and the failure is logged. Never panics, never
drops the code.

## IntoErrorResponse trait  *(core, refactored)*

The Actix `ResponseError`-style contract replacing the current
`fn into_error_response(self) -> Error`.

```text
trait IntoErrorResponse {
    fn status_code(&self) -> StatusCode { StatusCode::from(PredefinedCode::Internal) }   // default
    fn error_response(self) -> ErrorResponse { /* default: serialize Display -> body, attach status_code */ }
}
```

- **Default `status_code`**: `Internal`.
- **Default `error_response`**: uses `status_code()` + a body derived from the
  error (e.g. its `Display`).
- **Blanket impl** for `E: Into<Error>` is retained so existing
  `Result<T, E>` handlers keep compiling, mapping to the default/Internal code
  (FR-006). `crate::Error` overrides `status_code` to map variants to the catalog.

## Relationship to existing wire types  *(transport, changed)*

| Type | Before | After |
| --- | --- | --- |
| `WireOutcome` | `Ok(Vec<u8>)` \| `Err(String)` | `Ok(Vec<u8>)` *(unchanged)* \| `Err { code: StatusCode, body: Vec<u8> }` |
| `CallResult` | `Ok(Vec<u8>)` \| `Err(String)` | `Ok(Vec<u8>)` *(unchanged)* \| `Err { code: StatusCode, body: Vec<u8> }` |
| `WireMessage::StreamError` | `{ id, message: String }` | `{ id, code: StatusCode, body: Vec<u8> }` |
| `ResponseSink::error` | `error(self, message: String)` | `error(self, code: StatusCode, body: Vec<u8>)` |
| `ServerEvent::Error` (memory) | `Error(String)` | `Error { code, body }` |
| `RpcOutcome::Stream` item | `Result<Vec<u8>, crate::Error>` | `Result<Vec<u8>, ErrorResponse>` |
| dispatch result error arm | `crate::Error` | `ErrorResponse` |

**Success path (`WireOutcome::Ok` / `CallResult::Ok`) is unchanged** (FR-013).

## State / lifecycle

No stateful entities. The status code is computed at the moment a handler (or the
framework) produces an error and is immutable thereafter. No migrations (no
persistence). The wire-format change is breaking but requires no data migration
(pre-1.0, no deployed peers).
