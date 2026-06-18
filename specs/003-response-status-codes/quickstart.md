# Quickstart / Validation Guide: Response Status Codes

How to validate the feature end-to-end once implemented. References
`contracts/handler-api.md` and `contracts/wire-protocol.md` for the exact shapes.

## Prerequisites

- Workspace builds: `cargo build`
- Tests pass: `cargo test`

## Automated validation (in-memory transport)

Add/extend tests in `crates/core` (and `crates/transport`) driving
`MemoryTransport`. Each maps to a user story / success criterion:

| Test | Story | Asserts |
| --- | --- | --- |
| Typed error round-trip | US1 / SC-001 | A handler returning a custom error yields a `WireOutcome::Err { code, body }`; client reads the exact `PredefinedCode` and deserializes the exact body. |
| Existing handler unchanged | US1 / SC-003 | A handler returning `Result<T, overseer_core::Error>` still compiles and maps to `Internal`. |
| Body-serialization failure | US1 / FR-011 | When the body fails to serialize, the response still carries the intended `code` with a fallback body; failure is logged. |
| Custom-section isolation | US2 / SC-005 | Setting `with_custom(x)` leaves predefined + flags bytes untouched; client reads `x` back intact. |
| Predefined immutability | US2 / FR-003 | No author API writes the predefined byte via the custom path. |
| Retryable flag | US3 / SC-004 | An error marked `RETRYABLE` is detectable via `contains` with no body deserialization; two flags coexist. |
| Streaming error parity | FR-010 / SC-006 | A mid-stream `Err` produces a `StreamError { code, body }` with the same shape as a unary error. |
| Unknown predefined code | FR-009 | Decoding a `StatusCode` with an unrecognized predefined byte yields `Unknown(u8)`, not a parse error. |

Run: `cargo test` (optionally `cargo test -p overseer-core status` to scope).

## Manual validation (example binary)

`src/bin/example.rs` is extended to demonstrate the feature (per project
convention, new features are shown in the existing example, and the daemon is
started by the user — not by the agent).

1. Start the daemon (user runs this):
   `cargo run --bin example -- daemon tcp`
2. Run the client (user runs this):
   `cargo run --bin example -- client tcp`
3. Expected: the client triggers an error RPC and prints the decoded
   `PredefinedCode`, the custom subcode, whether `RETRYABLE` is set, and the
   deserialized error body — instead of today's `panic!("RPC error: {e}")`.

The updated `unpack()` reads `WireOutcome::Err { code, body }` and reports the
classified failure rather than a bare string.

## Definition of done (validation)

- All automated tests above pass under `cargo test`.
- The example prints a classified error (code + flag + body), demonstrating a
  consumer branching on the code without parsing the message.
- The success path (`WireOutcome::Ok`) behaves exactly as before (regression check
  via the existing ping/greet/streaming example flows).
