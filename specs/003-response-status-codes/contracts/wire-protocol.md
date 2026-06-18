# Contract: Wire Protocol (error path)

Defines the on-the-wire error contract. Owned by `crates/transport`. This is a
**breaking change** to the error arm; the success arm is unchanged. No migration
path is provided (pre-1.0 prototype, no deployed peers); all in-tree transports
(`stream`, `memory`) are updated together.

## StatusCode wire encoding

A `u32` serialized via postcard (varint). Section layout (most- to least-
significant):

```text
 31      24 23      16 15                   0
+----------+----------+----------------------+
|  flags   | predef.  |       custom         |
|  (u8)    |  (u8)    |       (u16)          |
+----------+----------+----------------------+
```

- **flags** `u8`: bitflags, framework-defined. `RETRYABLE = 0x01` (bit 0 of the
  flags byte). Multiple flags OR together.
- **predefined** `u8`: discrete framework code. `0 = Internal`, `1 = BadInput`,
  `2 = NotFound`, `3 = Unauthorized`. Any other value MUST be accepted and treated
  as an unknown category (forward compatibility).
- **custom** `u16`: application-owned, opaque to the framework. The framework MUST
  NOT interpret it and MUST NOT let it affect the flags or predefined sections.

Decoders MUST treat every `u32` as valid (no out-of-range failure).

## WireOutcome

```text
enum WireOutcome {
    Ok(Vec<u8>),                                 // UNCHANGED
    Err { code: StatusCode, body: Vec<u8> },     // was: Err(String)
}
```

- `Ok` — postcard-encoded success body, exactly as today.
- `Err.code` — the packed status (always present, defaults to `Internal`).
- `Err.body` — postcard-encoded error body chosen by the error type; MAY be empty.

## WireMessage::StreamError

```text
StreamError { id: CallId, code: StatusCode, body: Vec<u8> }   // was: { id, message: String }
```

Terminates a streaming call with a failure carrying the same `{ code, body }` shape
as a unary error (uniform error handling across unary and streaming, FR-010/SC-006).

## Direction and ordering

Unchanged from the streaming feature: error frames are server→client, correlated by
`CallId`, written atomically under the shared write lock. An `Err`/`StreamError`
frame is terminal for its `CallId`.

## Consumer obligations

- A consumer MUST read `code` to classify a failure without parsing `body`.
- A consumer MUST be able to test a single flag (e.g. `RETRYABLE`) without
  deserializing `body`.
- A consumer encountering an unknown predefined byte MUST treat it as an unknown
  category, not a parse failure.
