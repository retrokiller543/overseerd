//! Typed handlers and parameter extraction, in the style of axum/actix.
//!
//! Instead of receiving the raw `RpcCallContext`, a handler declares its
//! dependencies as parameters (`Payload<T>`, `Conn`, `Extension<T>`, ...). Any
//! async fn whose parameters are all `FromContext` and that returns
//! `Result<R: Serialize, E: Into<Error>>` then satisfies `Handler`, and
//! `dispatch_with` adapts it to the erased future the router invokes.

use std::{future::Future, pin::Pin, sync::Arc};

use serde::{Serialize, de::DeserializeOwned};

use crate::{Error, RpcCallContext, RpcResponse, connection::ConnectionInfo};

/// A value a handler can extract from the call context.
///
/// Extractors run in parameter order before the handler body. Each only reads
/// from the context, so any number of them can coexist on one handler.
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

/// An async handler whose parameters are all `FromContext` and that returns a
/// serializable value or an error.
///
/// `Args` is a marker for the parameter-tuple type, which lets the compiler
/// select the right arity impl (the same trick axum uses).
pub trait Handler<Args>: Send + Sized + 'static {
    fn call(
        self,
        ctx: RpcCallContext,
    ) -> impl Future<Output = crate::Result<RpcResponse>> + Send + 'static;
}

macro_rules! impl_handler {
    ( $($ty:ident),* ) => {
        impl<F, Fut, Res, Err, $($ty,)*> Handler<($($ty,)*)> for F
        where
            F: Fn($($ty,)*) -> Fut + Send + Sync + 'static,
            Fut: Future<Output = core::result::Result<Res, Err>> + Send + 'static,
            Res: Serialize + 'static,
            Err: Into<Error> + 'static,
            $( $ty: FromContext + Send + 'static, )*
        {
            #[allow(non_snake_case, unused_variables)]
            async fn call(self, ctx: RpcCallContext) -> crate::Result<RpcResponse> {
                $( let $ty = <$ty as FromContext>::from_context(&ctx).await?; )*

                let output = (self)($($ty,)*).await.map_err(Into::into)?;
                let payload = postcard::to_allocvec(&output)
                    .map_err(|e| Error::Serialization(e.to_string()))?;

                Ok(RpcResponse { payload })
            }
        }
    };
}

impl_handler!();
impl_handler!(T1);
impl_handler!(T1, T2);
impl_handler!(T1, T2, T3);
impl_handler!(T1, T2, T3, T4);
impl_handler!(T1, T2, T3, T4, T5);

/// Adapts a typed `Handler` into the erased future the router invokes.
///
/// A `#[rpc]` proc macro will emit a tiny non-capturing wrapper fn around this
/// per handler; because it captures nothing, the wrapper coerces to the
/// `RpcHandler` fn pointer stored in a `static` `RpcDescriptor`. Until the
/// macro exists, write that one-line wrapper by hand (see the `scoped_deps`
/// example).
pub fn dispatch_with<H, Args>(
    handler: H,
    ctx: RpcCallContext,
) -> Pin<Box<dyn Future<Output = crate::Result<RpcResponse>> + Send>>
where
    H: Handler<Args>,
{
    Box::pin(handler.call(ctx))
}
