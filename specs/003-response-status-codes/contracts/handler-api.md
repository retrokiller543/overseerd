# Contract: Handler API (error responses)

Defines the server-side API a service author uses to attach a status code and body
to an error. Owned by `crates/core`; `StatusCode`/`PredefinedCode`/flags are
re-exported from `crates/transport`.

## IntoErrorResponse (refactored)

Replaces the current `trait IntoErrorResponse { fn into_error_response(self) -> Error; }`
with an Actix `ResponseError`-style contract:

```text
trait IntoErrorResponse {
    /// The status code for this error. Defaults to PredefinedCode::Internal.
    fn status_code(&self) -> StatusCode { /* Internal */ }

    /// Render the error to a code + serialized body.
    /// Default: body = serialized Display string, code = self.status_code().
    fn error_response(self) -> ErrorResponse { /* default */ }
}
```

**Guarantees**:
- Both methods have defaults, so an implementor can override *just* the status
  code, *just* the body, or both — incremental opt-in (FR-005).
- A blanket impl for `E: Into<Error>` is retained: any error convertible to
  `overseerd_core::Error` satisfies the trait with the default (Internal) code,
  so existing `Result<T, E>` handlers compile unchanged (FR-006).
- `overseerd_core::Error` overrides `status_code` to map its variants onto the
  predefined catalog (`InvalidPayload`/`NotStreaming` → `BadInput`,
  `RouteNotFound` → `NotFound`, others → `Internal`).
- If body serialization fails, `error_response` MUST still return a well-formed
  `ErrorResponse` preserving `code`, using a fallback body, and log the failure
  (FR-011).

## ErrorResponse

```text
struct ErrorResponse { code: StatusCode, body: Vec<u8> }
ErrorResponse::new(code: StatusCode, body: Vec<u8>) -> Self
```

The error currency of the dispatch path: the erased handler future resolves to
`Result<RpcOutcome, ErrorResponse>`. `FallibleHandler` maps `Err(e)` via
`e.error_response()`; extractor failures (`crate::Error`) convert via the blanket
impl.

## StatusCode construction (author-facing)

```text
StatusCode::from(PredefinedCode::NotFound)              // predefined only
    .with_custom(0x0042)                                // app subcode
    .with_flag(StatusCode::RETRYABLE)                   // control-flow flag
```

- `with_custom` writes only the low 16 bits; it can never alter the predefined or
  flags sections (FR-003).
- Custom errors have **no** API to write the predefined byte except through
  `PredefinedCode` (framework-owned), enforcing section ownership.

## Example author usage (illustrative)

```text
#[derive(Debug, thiserror::Error)]
enum GreetError {
    #[error("name is empty")]
    EmptyName,
}

impl IntoErrorResponse for GreetError {
    fn status_code(&self) -> StatusCode {
        StatusCode::from(PredefinedCode::BadInput).with_custom(1)
    }
    // default error_response serializes the Display string as the body
}

#[rpc]
async fn greet(Payload(req): Payload<GreetRequest>) -> Result<GreetResponse, GreetError> { ... }
```

This compiles through the existing `dispatch_fallible` path with no macro change —
the macro already routes `Result`-returning handlers to `FallibleHandler`, which
now enforces the refactored `IntoErrorResponse`.
