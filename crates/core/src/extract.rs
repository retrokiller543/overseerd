//! Typed handlers, parameter extraction, and response conversion, in the style
//! of axum/actix.
//!
//! Instead of receiving the raw `RpcCallContext`, a handler declares its
//! dependencies as parameters (`Payload<T>`, `Streaming<T>`, `Conn`, ...) and
//! returns any type that implements [`Responder`]. Any async fn whose
//! parameters are all `FromContext` and whose return type is a `Responder` then
//! satisfies `Handler`, and `dispatch_with` adapts it to the erased future the
//! router invokes.
//!
//! [`Responder`] decides what a *successful* return value becomes on the wire.
//! A blanket impl serializes any `Serialize` value as a unary body (the default
//! format), so handlers can return `T`, `()`, or `Option<T>` directly, and
//! [`ResponseStream<T>`] (a local type, so no overlap with the blanket) produces
//! a stream of items.
//!
//! Fallibility is handled one layer up, at dispatch. `Result<T, E>` is
//! deliberately *not* a `Responder`: Rust's intercrate coherence refuses to let
//! a blanket `impl<T: Serialize>` coexist with an `impl … for Result` (it will
//! not prove `Result: !Serialize` even though serde never implements it). So a
//! handler returning `Result<R, E>` satisfies [`FallibleHandler`] instead of
//! [`Handler`] — that is where `E: ResponseError` is enforced and `Err` is
//! mapped to a transport error. The `#[rpc]` macro picks the matching
//! `dispatch_*` from the return type, so for a given handler exactly one of the
//! two traits ever applies.

use std::{
    future::Future,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

use futures::{Stream, StreamExt};
use overseer_transport::{PredefinedCode, StatusCode};
use serde::{Serialize, de::DeserializeOwned};
use tokio_stream::wrappers::ReceiverStream;
use tokio_util::sync::CancellationToken;
use tracing::warn;

use crate::{
    Error, RpcCallContext, RpcResponse, connection::ConnectionInfo, descriptors::RpcOutcome,
};

/// A value a handler can extract from the call context.
///
/// Extractors run in parameter order before the handler body. Most only read
/// from the context; `Streaming<T>` takes the inbound request stream, so it may
/// appear at most once.
pub trait FromContext: Sized {
    fn from_context(ctx: &RpcCallContext) -> impl Future<Output = crate::Result<Self>> + Send;
}

/// Deserializes the request body into `T`.
pub struct Payload<T>(pub T);

impl<T> FromContext for Payload<T>
where
    T: DeserializeOwned + Send,
{
    async fn from_context(ctx: &RpcCallContext) -> crate::Result<Self> {
        let value =
            postcard::from_bytes(&ctx.payload).map_err(|e| Error::InvalidPayload(e.to_string()))?;

        Ok(Payload(value))
    }
}

/// A shared handle to the full connection context, for ad-hoc `get::<T>()`
/// lookups of connection-scoped state.
pub struct Conn(pub Arc<ConnectionInfo>);

impl FromContext for Conn {
    async fn from_context(ctx: &RpcCallContext) -> crate::Result<Self> {
        Ok(Conn(Arc::clone(&ctx.connection)))
    }
}

/// A clone of a connection-scoped value of type `T`, inserted by a
/// `ConnectionHandler` in `on_connect`. Fails if no such value is present.
pub struct Extension<T>(pub T);

impl<T> FromContext for Extension<T>
where
    T: Clone + Send + Sync + 'static,
{
    async fn from_context(ctx: &RpcCallContext) -> crate::Result<Self> {
        let value = ctx
            .connection
            .get::<T>()
            .cloned()
            .ok_or(Error::MissingExtension(std::any::type_name::<T>()))?;

        Ok(Extension(value))
    }
}

/// The call's cancellation token, fired when the peer cancels the call or the
/// connection drops. Long-running and streaming handlers observe it to unwind.
pub struct Cancel(pub CancellationToken);

impl FromContext for Cancel {
    async fn from_context(ctx: &RpcCallContext) -> crate::Result<Self> {
        Ok(Cancel(ctx.cancel.clone()))
    }
}

/// The inbound request stream for client- and bidirectional-streaming calls,
/// yielding each item deserialized into `T`. Implements [`Stream`], so handlers
/// drive it with `.next().await` (or any `StreamExt` combinator).
pub struct Streaming<T> {
    inner: Pin<Box<dyn Stream<Item = crate::Result<T>> + Send>>,
}

impl<T> FromContext for Streaming<T>
where
    T: DeserializeOwned + Send + 'static,
{
    async fn from_context(ctx: &RpcCallContext) -> crate::Result<Self> {
        let rx = ctx.take_requests().ok_or(Error::NotStreaming)?;

        let stream = ReceiverStream::new(rx).map(|bytes| {
            postcard::from_bytes(&bytes).map_err(|e| Error::InvalidPayload(e.to_string()))
        });

        Ok(Streaming {
            inner: Box::pin(stream),
        })
    }
}

impl<T> Stream for Streaming<T> {
    type Item = crate::Result<T>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.get_mut().inner.as_mut().poll_next(cx)
    }
}

/// The handler-side error currency: a status code plus a serialized body, carried
/// to the wire error arm of a call (unary or streaming).
#[derive(Debug)]
pub struct ErrorResponse {
    pub code: StatusCode,
    pub body: Vec<u8>,
}

impl ErrorResponse {
    /// Builds an error response from a status code and an already-serialized body.
    pub fn new(code: StatusCode, body: Vec<u8>) -> Self {
        Self { code, body }
    }

    /// Builds an error response by serializing `body`, degrading to an empty body
    /// with a logged warning rather than failing (FR-011): the status code must
    /// survive even when the body cannot be encoded.
    pub fn with_serialized_body<T: Serialize + ?Sized>(code: StatusCode, body: &T) -> Self {
        match postcard::to_allocvec(body) {
            Ok(bytes) => Self::new(code, bytes),

            Err(e) => {
                warn!(error = %e, "failed to serialize error body; falling back to empty body");

                Self::new(code, Vec::new())
            }
        }
    }
}

/// Converts an error into the status code + body sent back to the caller.
///
/// `status_code` defaults to `Internal`, so most implementors override only that.
/// A blanket impl covers any `E: Serialize`, serializing the error value itself
/// as the body under the default code — so a serializable error type is usable as
/// a handler error with no boilerplate. A type wanting a custom code, or a body
/// distinct from its own serialization, implements the trait directly (and is
/// then not `Serialize`, so it does not overlap the blanket). [`Error`] is one
/// such type: it maps its variants to categories via [`Error::status_code`](crate::Error::status_code).
pub trait ResponseError {
    type Body: Serialize;
    
    /// The status code for this error. Defaults to `Internal`.
    fn status_code(&self) -> StatusCode {
        StatusCode::from(PredefinedCode::Internal)
    }

    /// Renders the error to a code + serialized body, attaching
    /// [`status_code`](Self::status_code).
    fn error_response(self) -> ErrorResponse;
}

impl<E> ResponseError for E
where
    E: Serialize,
{
    type Body = E;
    
    fn error_response(self) -> ErrorResponse {
        ErrorResponse::with_serialized_body(self.status_code(), &self)
    }
}

impl From<Error> for ErrorResponse {
    fn from(err: Error) -> Self {
        err.error_response()
    }
}

/// What a *successful* handler return value becomes on the wire.
///
/// Implemented for any `Serialize` type (the default unary body format) and for
/// [`ResponseStream<T>`] (a stream of items). Implement it by hand for custom
/// body formats. Fallibility is not modelled here — see [`FallibleHandler`] —
/// because a blanket `impl<T: Serialize>` and an `impl … for Result` cannot
/// coexist under Rust's coherence rules.
pub trait Responder {
    fn respond(self) -> Result<RpcOutcome, ErrorResponse>;
}

impl<T> Responder for T
where
    T: Serialize,
{
    fn respond(self) -> Result<RpcOutcome, ErrorResponse> {
        let payload =
            postcard::to_allocvec(&self).map_err(|e| Error::Serialization(e.to_string()))?;

        Ok(RpcOutcome::Unary(RpcResponse { payload }))
    }
}

/// A stream of response items returned by server- and bidirectional-streaming
/// handlers. Each item is serialized to a `StreamItem` frame; a yielded `Err`
/// terminates the stream with a `StreamError`.
pub struct ResponseStream<T> {
    inner: Pin<Box<dyn Stream<Item = crate::Result<T>> + Send>>,
}

impl<T> ResponseStream<T> {
    pub fn new(stream: impl Stream<Item = crate::Result<T>> + Send + 'static) -> Self {
        Self {
            inner: Box::pin(stream),
        }
    }
}

impl<T> Responder for ResponseStream<T>
where
    T: Serialize + Send + 'static,
{
    fn respond(self) -> Result<RpcOutcome, ErrorResponse> {
        let items = self.inner.map(|item| -> Result<Vec<u8>, ErrorResponse> {
            let value = item?;
            let bytes =
                postcard::to_allocvec(&value).map_err(|e| Error::Serialization(e.to_string()))?;

            Ok(bytes)
        });

        Ok(RpcOutcome::Stream(Box::pin(items)))
    }
}

/// An async handler whose parameters are all `FromContext` and whose return
/// type is a [`Responder`].
///
/// `Args` is a marker for the parameter-tuple type, which lets the compiler
/// select the right arity impl (the same trick axum uses).
pub trait Handler<Args>: Send + Sized + 'static {
    fn call(
        self,
        ctx: RpcCallContext,
    ) -> impl Future<Output = Result<RpcOutcome, ErrorResponse>> + Send + 'static;
}

macro_rules! impl_handler {
    ( $($ty:ident),* ) => {
        impl<F, Fut, Res, $($ty,)*> Handler<($($ty,)*)> for F
        where
            F: Fn($($ty,)*) -> Fut + Send + Sync + 'static,
            Fut: Future<Output = Res> + Send + 'static,
            Res: Responder + 'static,
            $( $ty: FromContext + Send + 'static, )*
        {
            #[allow(non_snake_case, unused_variables)]
            async fn call(self, ctx: RpcCallContext) -> Result<RpcOutcome, ErrorResponse> {
                $( let $ty = <$ty as FromContext>::from_context(&ctx).await?; )*

                let output = (self)($($ty,)*).await;

                output.respond()
            }
        }
    };
}

/// The fallible counterpart to [`Handler`]: an async fn returning
/// `Result<R, E>` where `R: Responder` and `E: ResponseError`.
///
/// A `Result` return cannot go through [`Handler`] (it is not a `Responder`),
/// so this trait carries it instead — mapping `Ok` through the responder and
/// `Err` to a transport error. Both traits erase to the same `RpcHandler` fn
/// pointer, so this split is invisible past dispatch.
pub trait FallibleHandler<Args>: Send + Sized + 'static {
    fn call(
        self,
        ctx: RpcCallContext,
    ) -> impl Future<Output = Result<RpcOutcome, ErrorResponse>> + Send + 'static;
}

macro_rules! impl_fallible_handler {
    ( $($ty:ident),* ) => {
        impl<F, Fut, Res, Err, $($ty,)*> FallibleHandler<($($ty,)*)> for F
        where
            F: Fn($($ty,)*) -> Fut + Send + Sync + 'static,
            Fut: Future<Output = core::result::Result<Res, Err>> + Send + 'static,
            Res: Responder + 'static,
            Err: ResponseError + 'static,
            $( $ty: FromContext + Send + 'static, )*
        {
            #[allow(non_snake_case, unused_variables)]
            async fn call(self, ctx: RpcCallContext) -> Result<RpcOutcome, ErrorResponse> {
                $( let $ty = <$ty as FromContext>::from_context(&ctx).await?; )*

                match (self)($($ty,)*).await {
                    Ok(value) => value.respond(),
                    Err(e) => Err(e.error_response()),
                }
            }
        }
    };
}

macro_rules! impl_params {
    ($handler:ident) => {
        $handler!();
        $handler!(T1);
        $handler!(T1, T2);
        $handler!(T1, T2, T3);
        $handler!(T1, T2, T3, T4);
        $handler!(T1, T2, T3, T4, T5);
        $handler!(T1, T2, T3, T4, T5, T6);
        $handler!(T1, T2, T3, T4, T5, T6, T7);
        $handler!(T1, T2, T3, T4, T5, T6, T7, T8);
        $handler!(T1, T2, T3, T4, T5, T6, T7, T8, T9);
        $handler!(T1, T2, T3, T4, T5, T6, T7, T8, T9, T10);
    };
}

impl_params!(impl_handler);
impl_params!(impl_fallible_handler);

/// Adapts an infallible typed [`Handler`] into the erased future the router
/// invokes.
///
/// A `#[rpc]` proc macro emits a tiny non-capturing wrapper fn around this (or
/// [`dispatch_fallible`]) per handler; because it captures nothing, the wrapper
/// coerces to the `RpcHandler` fn pointer stored in a `static` `RpcDescriptor`.
pub fn dispatch_with<H, Args>(
    handler: H,
    ctx: RpcCallContext,
) -> Pin<Box<dyn Future<Output = Result<RpcOutcome, ErrorResponse>> + Send>>
where
    H: Handler<Args>,
{
    Box::pin(handler.call(ctx))
}

/// Adapts a [`FallibleHandler`] (a `Result`-returning handler) into the erased
/// future the router invokes. Counterpart to [`dispatch_with`].
pub fn dispatch_fallible<H, Args>(
    handler: H,
    ctx: RpcCallContext,
) -> Pin<Box<dyn Future<Output = Result<RpcOutcome, ErrorResponse>> + Send>>
where
    H: FallibleHandler<Args>,
{
    Box::pin(handler.call(ctx))
}

#[cfg(test)]
mod tests {
    use overseer_transport::PredefinedCode;
    use serde::Serializer;
    use serde::ser::Error as _;

    use super::*;

    /// A type whose serialization always fails, to exercise the FR-011 fallback.
    struct Unserializable;

    impl Serialize for Unserializable {
        fn serialize<S: Serializer>(&self, _serializer: S) -> Result<S::Ok, S::Error> {
            Err(S::Error::custom("intentional serialization failure"))
        }
    }

    #[test]
    fn body_serialization_failure_preserves_code_with_empty_body() {
        // FR-011: a body that fails to serialize yields the intended code with a
        // fallback (empty) body rather than dropping the code or panicking.
        let code = StatusCode::from(PredefinedCode::NotFound);
        let response = ErrorResponse::with_serialized_body(code, &Unserializable);

        assert_eq!(response.code, code);
        assert!(response.body.is_empty());
    }

    #[test]
    fn serializable_error_uses_blanket_impl() {
        // A `Serialize` error gets the blanket `ResponseError`: the default
        // `Internal` code, with the error value itself as the body.
        #[derive(serde::Serialize, serde::Deserialize, PartialEq, Debug)]
        struct PlainError {
            reason: String,
        }

        let error = PlainError {
            reason: "boom".to_string(),
        };
        let response = error.error_response();
        let decoded: PlainError = postcard::from_bytes(&response.body).expect("decode body");

        assert_eq!(decoded.reason, "boom");
        assert_eq!(response.code.predefined(), PredefinedCode::Internal);
    }
}
