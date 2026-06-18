# Feature Specification: Response Status Codes

**Feature Branch**: `003-response-status-codes`

**Created**: 2026-06-18

**Status**: Draft

**Input**: User description: "We are starting to iron out item nmr 3 in TODO.md, adding status codes to response values, i would like the status code to be, this also refactors IntoErrorResponse to be more similar to the Actix format of ResponseError allowing for any body to be sent as a error with a status code (u16 over the wire where we specify common error codes and allow for custom ones, might be worth to make it bitflags to allow for combination of errors). we can split the u16 into two sections, first half is predefined and cant be changed by custom errors, second half is only for custom errors, meaning we send two u8 next to eachother"

## Overview

Today an Overseer RPC error is a bare string: a handler error collapses to its
display text and travels back as an undifferentiated `Err(String)`. A client
cannot tell *what kind* of failure occurred, cannot branch on it (retry vs.
give up vs. surface to the user), and cannot receive any structured detail
beyond the message. This feature gives error responses a machine-readable
**status code** and an **arbitrary serializable body**, modelled on Actix's
`ResponseError` trait, so that the type returned by a handler decides both the
code and the payload that reaches the caller.

The "users" of this feature are the developers building services on top of
Overseer (defining error types and handlers) and the developers consuming those
services (handling the responses). Scope is **error responses only** for this
increment; successful responses keep their current shape.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Typed status code and structured body on error responses (Priority: P1)

A service developer returns an error from a handler. Instead of losing all
structure to a string, the error carries a **predefined status code** drawn from
a framework catalog of common failure categories (e.g. invalid input, internal
error, not found, unauthorized) and an **arbitrary serializable body** the
developer chooses. The consuming developer receives both: they can match on the
code for control flow and deserialize the body for detail. Existing handlers
that return `Result<T, E>` keep compiling and behave sensibly without any code
change, defaulting to a generic "internal error" category.

**Why this priority**: This is the core of the feature and the smallest slice
that delivers value. Every other story builds on the new error shape. It also
discharges the explicit ask: refactor the error-response trait to the
Actix-`ResponseError` style (a status code plus a free-form body).

**Independent Test**: Define an error type that maps to a predefined code and a
custom body; return it from a handler; assert the client observes the exact code
and can deserialize the exact body. Separately, take an existing handler that
returns a plain error and assert it still works, mapped to the default code.

**Acceptance Scenarios**:

1. **Given** a handler that returns an error carrying a predefined code and a
   structured body, **When** the client receives the response, **Then** the
   client can read the same predefined code and deserialize the same body.
2. **Given** an existing handler returning a plain framework error with no
   explicit code, **When** it fails, **Then** the response carries the default
   predefined code (generic internal error) and a body derived from the error's
   message, and no source change to that handler was required.
3. **Given** an error whose body cannot be serialized, **When** the response is
   built, **Then** the framework still returns a well-formed error response with
   the intended code and a fallback body rather than panicking or dropping the
   code.

---

### User Story 2 - Custom application error codes that cannot collide with framework codes (Priority: P2)

A service developer needs error codes specific to their domain (e.g. "tenant
suspended", "quota exhausted") that the framework does not define. They set a
value in the **custom section** of the status code. The framework guarantees this
value can never overwrite or be confused with the **predefined section**: the two
live in separate, non-overlapping parts of the same code, so framework upgrades
that add predefined codes never collide with an application's custom codes, and
an application can never accidentally claim a framework-reserved code.

**Why this priority**: Custom codes are a primary motivation ("specify common
error codes and allow for custom ones"), but they depend on the predefined
catalog and code structure from US1 existing first.

**Independent Test**: Set a custom code on an error; assert the predefined
section is unaffected and the client reads back the custom value intact. Attempt
(in the type system / API) to set a framework-reserved value via the custom path
and assert it is impossible or has no effect on the predefined section.

**Acceptance Scenarios**:

1. **Given** an error with a custom code set, **When** the client receives it,
   **Then** the custom section carries the exact value the application set and
   the predefined section reflects only the framework category.
2. **Given** the framework later adds a new predefined code, **When** an existing
   application that uses a custom code is recompiled against it, **Then** the
   application's custom code is unchanged and does not collide.
3. **Given** application code, **When** a developer tries to set a value in the
   predefined section through the custom-error path, **Then** the API does not
   permit it (the predefined section is not writable by custom errors).

---

### User Story 3 - Control-flow flags carried alongside the code (Priority: P3)

A consuming developer (or a generic middleware/client layer) wants to make a
decision — most commonly "is this retryable?" — without deserializing the body
or string-matching the message. Error responses carry a small set of
framework-defined **flags** (e.g. retryable) in a dedicated, combinable section
of the code. A caller tests a single bit to branch; multiple flags can be set at
once.

**Why this priority**: Valuable for ergonomics and generic client behavior, but
the system is fully usable with codes and bodies (US1/US2) before flags exist.
Flags are the most speculative part of the request ("might be worth ...").

**Independent Test**: Return an error marked retryable; assert the client can
detect the retryable flag with a single check and that combining it with other
flags preserves each flag independently.

**Acceptance Scenarios**:

1. **Given** an error marked with the retryable flag, **When** the client
   inspects the code, **Then** it detects retryable without reading the body.
2. **Given** an error with two flags set, **When** the client inspects the code,
   **Then** both flags read as set and the predefined and custom sections are
   unaffected.

---

### Edge Cases

- **No explicit code**: a handler error that does not opt into the new trait
  must still produce a valid response with the default predefined code.
- **Unknown predefined code at the consumer**: a client built against an older
  framework version receives a predefined code it does not recognize; it must
  treat it as an opaque/unknown category rather than failing to parse the
  response.
- **Body serialization failure**: must degrade to a fallback body while keeping
  the status code intact (never silently drop the error).
- **Streaming errors**: a mid-stream failure today carries only a string
  message; it must carry the same status code + body shape as a unary error so
  consumers handle both paths uniformly.
- **Reserved/zero code**: behavior when the predefined section is the default/
  unset value must be well-defined (maps to the generic internal-error
  category).
- **Custom code with no predefined meaning**: an error that is purely
  application-defined still occupies a sensible predefined category (e.g. a
  generic "application error" bucket) so generic clients are not left with an
  empty category.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: Error responses MUST carry a machine-readable status code in
  addition to (and separate from) the existing message/body.
- **FR-002**: The status code MUST be partitioned into three non-overlapping
  logical sections: (a) **flags** — framework-defined, combinable boolean
  signals for control flow (e.g. retryable); (b) **predefined code** —
  framework-owned discrete categories of common failures; (c) **custom code** —
  an application-owned value the framework does not interpret.
- **FR-003**: The predefined section MUST be owned exclusively by the framework;
  application/custom errors MUST NOT be able to set or alter it. The custom
  section MUST be writable by applications and MUST NOT be able to overwrite the
  predefined or flags sections.
- **FR-004**: Error responses MUST be able to carry an arbitrary serializable
  body chosen by the error type (not limited to a string), mirroring the
  Actix `ResponseError` model where the error decides both its status and its
  rendered response body.
- **FR-005**: The error-response trait (replacing/refactoring the current
  `IntoErrorResponse`) MUST expose both a status code and a body for an error
  type, with sensible defaults so that an error type can opt into a code and/or
  a custom body incrementally.
- **FR-006**: Existing handlers returning `Result<T, E>` where `E` converts into
  the framework error MUST continue to work without source changes, mapping to a
  default predefined code (generic internal error) and a body derived from the
  error message.
- **FR-007**: The framework MUST provide a catalog of common predefined codes
  (at minimum: invalid input, internal error, not found, unauthorized) so that
  the existing internal framework errors map onto meaningful categories.
- **FR-008**: A consumer MUST be able to read each section of the code
  independently: test a flag, match the predefined category, and read the custom
  value, without deserializing the body.
- **FR-009**: A consumer that receives a predefined code it does not recognize
  MUST be able to handle the response as an unknown category without failing to
  parse it (forward compatibility across framework versions).
- **FR-010**: Mid-stream streaming errors MUST carry the same status code + body
  shape as unary errors, so error handling is uniform across unary and streaming
  calls.
- **FR-011**: If the chosen error body cannot be serialized, the framework MUST
  still emit a well-formed error response that preserves the status code, using
  a fallback body, and MUST log the serialization failure.
- **FR-012**: The flags section MUST support combining multiple flags in a single
  response, each independently testable by the consumer.
- **FR-013**: Successful responses MUST remain unchanged by this feature (no
  status code added to the success path in this increment).

### Constitution Alignment *(mandatory)*

- **Scope Control**: In scope — the error-response path (unary and streaming),
  the refactor of `IntoErrorResponse` into an Actix-`ResponseError`-style trait,
  the structured status code (flags + predefined + custom sections), the
  predefined code catalog, and mapping existing framework errors onto it. Out of
  scope — status codes on successful responses, client-SDK generation (TODO #4),
  multi-frame bodies (TODO #9), and any transport other than the existing
  TCP/Unix/in-memory ones.
- **Independent Verification**: US1 verified by round-tripping a code + body
  through a handler and asserting the client reads both, plus a regression test
  that an unchanged `Result<T, E>` handler still works. US2 verified by setting a
  custom code and asserting section isolation. US3 verified by setting and
  testing flags. Each story has its own tests that pass without the later
  stories implemented.
- **Interfaces & Data Contracts**: This changes the wire contract. `WireOutcome`
  today is `Ok(Vec<u8>) | Err(String)`; the error arm becomes a structured
  `{ code, body }` (code = the packed status integer, body = serialized bytes).
  The streaming `StreamError { id, message: String }` frame gains the same code +
  body shape. The handler-facing contract changes: the `IntoErrorResponse` trait
  is refactored to surface a status code and a body. `CallResult::Err(String)`
  and the `Err(e) => responder.respond(CallResult::Err(e.to_string()))` path in
  the serve loop are replaced by the structured form. This is a **breaking wire
  change**; because the project is a pre-1.0 prototype with no deployed peers,
  no on-the-wire migration is required, but the change MUST be called out in the
  plan and the in-memory/test transports updated in lockstep.
- **Operational Safety**: Error bodies and codes MUST NOT embed secrets; the
  default body derived from an error message MUST follow the existing
  actionable-error practice and not leak internals beyond the current behavior.
  Body serialization failures MUST be logged (FR-011). No new persistence,
  network, credential, or permission boundary is introduced.

### Key Entities *(include if feature involves data)*

- **Status Code**: A fixed-width unsigned integer partitioned into three
  sections — flags (combinable bitflags, framework-defined), predefined code
  (discrete, framework-owned), and custom code (application-owned, opaque to the
  framework). Each section is independently readable.
- **Predefined Code Catalog**: The framework-owned set of common failure
  categories (invalid input, internal error, not found, unauthorized, ...),
  onto which existing internal framework errors are mapped. Extensible by the
  framework without colliding with custom codes.
- **Control-Flow Flags**: A small framework-defined set of combinable boolean
  signals (e.g. retryable) occupying the flags section, testable without reading
  the body.
- **Error Response**: The pairing of a status code with an arbitrary serializable
  body, carried by both the unary error arm of `WireOutcome` and the streaming
  error frame.
- **Error-Response Trait** (refactored `IntoErrorResponse`): The
  Actix-`ResponseError`-style contract an error type implements to declare its
  status code and render its body, with defaults that preserve existing
  behavior.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: A consumer can determine the failure category of 100% of error
  responses by reading the status code, without parsing the message text.
- **SC-002**: 100% of the framework's existing internal error variants map to a
  predefined category (no error falls back to "unknown" except by explicit
  default).
- **SC-003**: Existing handlers returning `Result<T, E>` require zero source
  changes to keep working after the refactor (verified by the existing example
  and test suite continuing to pass).
- **SC-004**: A consumer can decide whether to retry a failed call using a single
  flag check, with no body deserialization required.
- **SC-005**: An application can define a custom error code and round-trip it to
  the consumer with the predefined and flags sections provably unaffected.
- **SC-006**: Unary and streaming error responses expose an identical
  code-and-body shape, verified by a test exercising both paths.

## Assumptions

- **Errors-only scope (confirmed)**: Status codes apply to error responses only
  in this increment; successful responses are unchanged. Adding success codes is
  deferred to a later increment.
- **Three-section structured code (confirmed direction)**: The code is split into
  a flags section (combinable, control-flow), a predefined framework-code section
  (discrete), and a custom application-code section (free-form). Only the flags
  section is bitflags; predefined and custom sections are discrete values.
- **Exact width is a planning decision**: The user described "two `u8`" growing to
  "three or four `u8`". The precise integer width (e.g. `u16` vs `u32`) and how
  many bytes the custom section gets (one or two) are left to the implementation
  plan; the spec only requires three non-overlapping sections with the ownership
  rules above.
- **Serialization format**: Error bodies use the same serialization format as the
  rest of the wire protocol (the project's existing postcard-based encoding); no
  new format is introduced.
- **Backward/wire compatibility**: As a pre-1.0 prototype with no deployed peers,
  the breaking wire change requires no migration path; all in-tree transports and
  tests are updated together.
- **Default mapping**: An error with no explicit code maps to a generic internal-
  error category; a purely application-defined error still occupies a sensible
  predefined bucket so generic consumers always see a category.
