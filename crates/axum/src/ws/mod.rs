//! WebSocket controllers: the pluggable framing/routing seam and its registration.
//!
//! A `#[controller(ws = P)]` is a DI singleton whose `#[handlers]` `#[message("dest")]` methods
//! contribute **message routes** (a destination → handler map) rather than HTTP routes. The macro
//! emits a [`WsControllerDescriptor`] into the [`WS_CONTROLLERS`] slice and a [`WebsocketController`]
//! impl naming the controller's protocol `P`.
//!
//! WebSockets are **opt-in**: a controller is only mounted when the user activates its protocol on
//! the builder with [`register_ws::<P>(path)`](crate::AxumAppBuilder::register_ws). That call owns
//! the upgrade-endpoint path (it can't be inferred), mounts the framework's generic upgrade handler
//! there, and hands the controllers that speak `P` to [`WebsocketProtocol::build`] — so the protocol
//! sets up its own routing and the app never sees a route. The first bundled protocol is [`JsonWs`].

mod json;
#[cfg(feature = "stomp")]
pub mod stomp;

use std::any::{Any, TypeId};
use std::future::Future;
use std::sync::Arc;

use axum::extract::ws::{CloseFrame, Message, Utf8Bytes, WebSocket, close_code};
use futures::future::BoxFuture;
use overseerd_app::AppRuntime;
use overseerd_core::TypeDescriptor;
use overseerd_di::ScopeContainer;
use tokio::time::Duration;

/// How long the framework waits for a WS close handshake to flush before abandoning the socket.
/// Bounds [`mount_ws`]'s error-path close send so a peer that never drains its receive buffer
/// can't block the upgrade task forever.
const CLOSE_SEND_TIMEOUT: Duration = Duration::from_secs(5);

pub use json::{JsonWs, WsReply};

/// The JSON value [`JsonWs`] messages decode from / encode to. Kept as a public alias so generated
/// `JsonWs` `#[message]` code names it without a separate `serde_json` dependency; it is exactly
/// [`JsonWs`]'s [`WebsocketProtocol::Payload`].
pub type WsValue = serde_json::Value;

/// The boxed future a [`WsHandlerFn`] returns — a decoded, dispatched message [`Outcome`], generic
/// over the protocol `P` that owns the payload/outcome vocabulary.
///
/// [`Outcome`]: WebsocketProtocol::Outcome
pub type WsFuture<P> =
    BoxFuture<'static, Result<<P as WebsocketProtocol>::Outcome, WsDispatchError>>;

/// A type-erased message handler for protocol `P`. It is handed the decoded
/// [`Payload`](WebsocketProtocol::Payload) and the message's
/// [`Request`-scope](crate::scope::Request) container, so it can decode the payload into the
/// handler's parameter *and* resolve the handler's `Inject<T>` parameters from the scope chain
/// (request → connection → singleton) — the same DI a REST route gets — before running the
/// controller method (the singleton captured by `Arc`) and turning the response into `P`'s
/// [`Outcome`](WebsocketProtocol::Outcome).
pub type WsHandlerFn<P> = Arc<
    dyn Fn(<P as WebsocketProtocol>::Payload, Arc<ScopeContainer>) -> WsFuture<P> + Send + Sync,
>;

/// What can go wrong dispatching one message, independent of the wire protocol framing it.
#[derive(Debug, thiserror::Error)]
pub enum WsDispatchError {
    /// The destination named by an inbound frame matches no `#[message]` handler.
    #[error("no handler for ws destination `{0}`")]
    NotFound(String),

    /// The payload could not be decoded into the handler's parameter type.
    #[error("decoding ws payload: {0}")]
    Decode(String),

    /// An `Inject<T>` parameter could not be resolved from the message scope.
    #[error("injecting ws dependency: {0}")]
    Inject(String),

    /// The handler's response could not be encoded.
    #[error("encoding ws response: {0}")]
    Encode(String),
}

/// One message route for protocol `P`: a destination string mapped to its handler. A
/// `#[message("dest")]` method produces one of these (with the controller singleton already
/// captured).
pub struct WsRoute<P: WebsocketProtocol> {
    /// The destination this handler answers (e.g. `"chat.send"`).
    pub destination: &'static str,

    /// The handler, ready to call with a decoded [`Payload`](WebsocketProtocol::Payload).
    pub handler: WsHandlerFn<P>,
}

impl<P: WebsocketProtocol> WsRoute<P> {
    /// Builds a route from a destination and its handler. Called by generated `#[message]` code.
    pub fn new(destination: &'static str, handler: WsHandlerFn<P>) -> Self {
        Self {
            destination,
            handler,
        }
    }
}

/// Decodes a protocol payload into a handler parameter type `T`. Implemented per protocol so the
/// codec (and its error mapping) lives here, not in emitted `#[message]` tokens; the macro calls
/// `<P as WsCodec<T>>::decode(payload)` uniformly across protocols.
pub trait WsCodec<T>: WebsocketProtocol {
    /// Decodes this protocol's [`Payload`](WebsocketProtocol::Payload) into `T`.
    fn decode(payload: Self::Payload) -> Result<T, WsDispatchError>;
}

/// Turns a handler's response value `R` into this protocol's [`Outcome`](WebsocketProtocol::Outcome).
/// The dual of [`WsCodec`]; the macro calls `<P as WsRespond<R>>::respond(response)`.
pub trait WsRespond<R>: WebsocketProtocol {
    /// Renders `response` into this protocol's outcome (a reply frame for `JsonWs`, a publish set
    /// for STOMP, …).
    fn respond(response: R) -> Result<Self::Outcome, WsDispatchError>;
}

/// A ws controller's link-time registration: its identity, the protocol it speaks, and a builder
/// for its message routes. Mirrors [`ControllerDescriptor`](crate::ControllerDescriptor), but the
/// upgrade *path* is **not** here — it comes from `register_ws`.
#[derive(Clone, Copy)]
pub struct WsControllerDescriptor {
    /// The controller's id (defaults to the lowercased type name).
    pub id: &'static str,

    /// The controller's display name (the type name).
    pub name: &'static str,

    /// The controller's concrete type.
    pub ty: TypeDescriptor,

    /// The [`TypeId`] of the [`WebsocketProtocol`] this controller speaks, so `register_ws::<P>`
    /// selects exactly the controllers framed by `P`. A `fn` (not a const) because `TypeId::of`
    /// is not yet const.
    pub protocol: fn() -> TypeId,

    /// The protocol's type name, for diagnostics. A `fn` because `type_name` is not yet const.
    pub protocol_name: fn() -> &'static str,

    /// Resolves the controller singleton from the runtime and builds its message routes — **type
    /// erased** to `Box<dyn Any + Send>` (really a `Vec<WsRoute<P>>` for this controller's protocol
    /// `P`), because the [`WS_CONTROLLERS`] link-time slice cannot store a generic descriptor.
    /// Recover the typed vector with [`routes_for`](Self::routes_for) — sound because
    /// `register_ws::<P>` only ever calls it for controllers whose [`protocol`](Self::protocol)
    /// `TypeId` matches `P`.
    pub routes: fn(&AppRuntime) -> Box<dyn Any + Send>,
}

impl WsControllerDescriptor {
    /// Builds this controller's routes as a concrete `Vec<WsRoute<P>>`, downcasting the erased
    /// [`routes`](Self::routes) product. Panics only on a framework bug — a caller passing a `P`
    /// that disagrees with this controller's [`protocol`](Self::protocol) `TypeId`; `register_ws`
    /// filters by that `TypeId` first, so the downcast always succeeds in practice.
    pub fn routes_for<P: WebsocketProtocol>(&self, runtime: &AppRuntime) -> Vec<WsRoute<P>> {
        let erased = (self.routes)(runtime);

        *erased.downcast::<Vec<WsRoute<P>>>().unwrap_or_else(|_| {
            panic!(
                "ws controller `{}` routes downcast to the wrong protocol `{}`",
                self.name,
                std::any::type_name::<P>()
            )
        })
    }
}

/// The link-time slice every `#[controller(ws = ..)]` registers into, mirroring [`CONTROLLERS`].
///
/// [`CONTROLLERS`]: crate::CONTROLLERS
#[linkme::distributed_slice]
pub static WS_CONTROLLERS: [WsControllerDescriptor];

/// Implemented by every `#[controller(ws = P)]` struct: it names its protocol and builds its
/// message routes. Generated alongside the [`WsControllerDescriptor`]; the per-`#[handlers]`-block
/// assertion forces this to hold so a REST controller can never be given message routing.
pub trait WebsocketController {
    /// The protocol that frames and routes this controller's messages.
    type Protocol: WebsocketProtocol;

    /// Builds this controller's message routes (typed to its [`Protocol`](Self::Protocol)),
    /// resolving its singleton from the runtime.
    fn ws_routes(runtime: &AppRuntime) -> Vec<WsRoute<Self::Protocol>>;
}

/// A pluggable WebSocket sub-protocol: it owns framing (how a raw [`Message`](axum::extract::ws::Message)
/// maps to a destination + payload) and routing (how a destination maps to a handler). Driven
/// generically — never stored as a trait object — so it carries associated state freely.
///
/// [`build`](Self::build) receives every controller registered to this protocol type and sets up
/// its routing once; [`serve`](Self::serve) then drives one upgraded socket against it. The future
/// generic `JsonWs` is the first implementation; a STOMP impl would add subscription/broadcast on
/// top without changing this seam.
pub trait WebsocketProtocol: Send + Sync + Sized + 'static {
    /// The decoded body a handler receives / an outbound frame carries. `JsonWs` uses
    /// [`WsValue`](serde_json::Value); a STOMP protocol uses a bytes + content-type body.
    type Payload: Send + 'static;

    /// What a handler returns, before framing: a correlated reply for `JsonWs`, a publish set for
    /// STOMP. Produced from the handler's response value via [`WsRespond`].
    type Outcome: Send + 'static;

    /// Per-endpoint settings passed at registration — heart-beat/version policy for STOMP, `()` for
    /// a protocol with nothing to configure. A user supplies these through
    /// [`register_ws_with`](crate::AxumAppBuilder::register_ws_with); the plain
    /// [`register_ws`](crate::AxumAppBuilder::register_ws) uses [`Default`].
    type Options: Send + 'static;

    /// Builds the protocol's routing from the controllers registered to it and the endpoint
    /// `options`. Called once per `register_ws` entrypoint at app build. The protocol keeps whatever
    /// it needs from `runtime` (e.g. a clone, to open per-message
    /// [`Request`](crate::scope::Request) scopes while serving). Recover each controller's typed
    /// routes with [`WsControllerDescriptor::routes_for::<Self>`](WsControllerDescriptor::routes_for).
    fn build(
        controllers: &[WsControllerDescriptor],
        runtime: &AppRuntime,
        options: Self::Options,
    ) -> Self;

    /// Drives one upgraded connection until the peer closes it or graceful shutdown fires.
    /// `connection` is this socket's [`Connection`](crate::scope::Connection) scope (opened once by
    /// the framework); the protocol parents each per-message scope at it.
    fn serve(
        self: Arc<Self>,
        socket: WebSocket,
        connection: Arc<ScopeContainer>,
        shutdown: WsShutdown,
    ) -> impl Future<Output = ()> + Send;
}

/// A connection-side graceful-shutdown signal. A protocol's [`serve`](WebsocketProtocol::serve) loop
/// races [`wait`](Self::wait) against reading the socket, so it can drain on app shutdown rather than
/// blocking the server's graceful stop on a long-lived connection.
#[derive(Clone)]
pub struct WsShutdown(tokio::sync::watch::Receiver<bool>);

impl WsShutdown {
    /// Resolves when graceful shutdown has been signalled for this endpoint.
    pub async fn wait(&mut self) {
        // Already-signalled, or wait for the next change; either way, return so the caller drains.
        if *self.0.borrow() {
            return;
        }

        let _ = self.0.changed().await;
    }
}

/// The app-side management handle for one mounted ws endpoint: its path and protocol, and the
/// trigger that drains its live connections on graceful shutdown. Held by the [`Axum`](crate::Axum)
/// protocol so endpoints can be inspected and shut down.
pub struct WebsocketHandler {
    path: String,
    protocol_name: &'static str,
    shutdown: tokio::sync::watch::Sender<bool>,
}

impl WebsocketHandler {
    /// The upgrade path this endpoint is mounted at.
    pub fn path(&self) -> &str {
        &self.path
    }

    /// The name of the protocol serving this endpoint.
    pub fn protocol_name(&self) -> &'static str {
        self.protocol_name
    }

    /// Signals every live connection on this endpoint to drain and close.
    pub fn trigger_shutdown(&self) {
        let _ = self.shutdown.send(true);
    }
}

/// Mounts one ws endpoint: builds the protocol `P` from its controllers and `options`, wires the
/// framework's generic upgrade handler at `path`, and returns a path-scoped router plus its
/// management handle. Monomorphized per `P`; `register_ws` stores a closure that calls it.
pub(crate) fn mount_ws<P: WebsocketProtocol>(
    path: &str,
    controllers: Vec<WsControllerDescriptor>,
    runtime: &AppRuntime,
    options: P::Options,
) -> (axum::Router, WebsocketHandler) {
    use axum::extract::ws::WebSocketUpgrade;

    let (tx, rx) = tokio::sync::watch::channel(false);
    let proto = Arc::new(P::build(&controllers, runtime, options));
    let shutdown = WsShutdown(rx);
    let runtime = runtime.clone();

    // The pre-built generic upgrade handler: it upgrades, opens this socket's `Connection` scope,
    // and hands the socket to the protocol, which owns the read→decode→dispatch→encode→send loop.
    let route_handler = move |ws: WebSocketUpgrade| {
        let proto = Arc::clone(&proto);
        let shutdown = shutdown.clone();
        let runtime = runtime.clone();

        async move {
            ws.on_upgrade(move |mut socket| async move {
                let connection = match runtime
                    .open_scope(
                        &crate::scope::Connection,
                        Arc::clone(runtime.root()),
                        Vec::new(),
                    )
                    .await
                {
                    Ok(scope) => scope,

                    Err(error) => {
                        tracing::error!(
                            target: "overseerd::axum",
                            %error,
                            "ws connection scope build failed; closing socket"
                        );

                        let close = Message::Close(Some(CloseFrame {
                            code: close_code::ERROR,
                            reason: Utf8Bytes::from_static("connection scope build failed"),
                        }));

                        let _ = tokio::time::timeout(CLOSE_SEND_TIMEOUT, socket.send(close)).await;

                        return;
                    }
                };

                proto.serve(socket, connection, shutdown).await;
            })
        }
    };

    let router = axum::Router::new().route(path, axum::routing::any(route_handler));
    let handler = WebsocketHandler {
        path: path.to_string(),
        protocol_name: std::any::type_name::<P>(),
        shutdown: tx,
    };

    (router, handler)
}
