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

use std::any::TypeId;
use std::future::Future;
use std::sync::Arc;

use axum::extract::ws::WebSocket;
use futures::future::BoxFuture;
use overseerd_app::AppRuntime;
use overseerd_core::TypeDescriptor;
use overseerd_di::ScopeContainer;

pub use json::JsonWs;

/// The JSON value a message payload decodes from / a response encodes to. Re-exported so generated
/// `#[message]` handlers name it without a separate `serde_json` dependency.
pub type WsValue = serde_json::Value;

/// The boxed future a [`WsHandlerFn`] returns — a decoded, dispatched, re-encoded message response.
pub type WsFuture = BoxFuture<'static, Result<WsValue, WsDispatchError>>;

/// A type-erased message handler. It is handed the decoded JSON payload and the message's
/// [`Request`-scope](crate::scope::Request) container, so it can decode the payload into the
/// handler's parameter *and* resolve the handler's `Inject<T>` parameters from the scope chain
/// (request → connection → singleton) — the same DI a REST route gets — before running the
/// controller method (the singleton captured by `Arc`) and encoding the response back to JSON.
pub type WsHandlerFn = Arc<dyn Fn(WsValue, Arc<ScopeContainer>) -> WsFuture + Send + Sync>;

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

/// One message route: a destination string mapped to its handler. A `#[message("dest")]` method
/// produces one of these (with the controller singleton already captured).
pub struct WsRoute {
    /// The destination this handler answers (e.g. `"chat.send"`).
    pub destination: &'static str,

    /// The handler, ready to call with a decoded JSON payload.
    pub handler: WsHandlerFn,
}

impl WsRoute {
    /// Builds a route from a destination and its handler. Called by generated `#[message]` code.
    pub fn new(destination: &'static str, handler: WsHandlerFn) -> Self {
        Self {
            destination,
            handler,
        }
    }
}

/// Decodes a message payload into a handler parameter. Called by generated `#[message]` code so the
/// codec (and its error mapping) lives here, not in emitted tokens.
pub fn decode_payload<T>(payload: WsValue) -> Result<T, WsDispatchError>
where
    T: serde::de::DeserializeOwned,
{
    serde_json::from_value(payload).map_err(|e| WsDispatchError::Decode(e.to_string()))
}

/// Encodes a handler response back to a JSON value. The dual of [`decode_payload`].
pub fn encode_response<R>(response: &R) -> Result<WsValue, WsDispatchError>
where
    R: serde::Serialize,
{
    serde_json::to_value(response).map_err(|e| WsDispatchError::Encode(e.to_string()))
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

    /// Resolves the controller singleton from the runtime and builds its message routes (the
    /// singleton captured in each handler).
    pub routes: fn(&AppRuntime) -> Vec<WsRoute>,
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

    /// Builds this controller's message routes, resolving its singleton from the runtime.
    fn ws_routes(runtime: &AppRuntime) -> Vec<WsRoute>;
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
    /// Builds the protocol's routing from the controllers registered to it. Called once per
    /// `register_ws` entrypoint at app build. The protocol keeps whatever it needs from `runtime`
    /// (e.g. a clone, to open per-message [`Request`](crate::scope::Request) scopes while serving).
    fn build(controllers: &[WsControllerDescriptor], runtime: &AppRuntime) -> Self;

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

/// Mounts one ws endpoint: builds the protocol `P` from its controllers, wires the framework's
/// generic upgrade handler at `path`, and returns a path-scoped router plus its management handle.
/// Monomorphized per `P`; a `fn` pointer to this is what `register_ws::<P>` stores.
pub(crate) fn mount_ws<P: WebsocketProtocol>(
    path: &str,
    controllers: Vec<WsControllerDescriptor>,
    runtime: &AppRuntime,
) -> (axum::Router, WebsocketHandler) {
    use axum::extract::ws::WebSocketUpgrade;

    let (tx, rx) = tokio::sync::watch::channel(false);
    let proto = Arc::new(P::build(&controllers, runtime));
    let shutdown = WsShutdown(rx);
    let runtime = runtime.clone();

    // The pre-built generic upgrade handler: it upgrades, opens this socket's `Connection` scope,
    // and hands the socket to the protocol, which owns the read→decode→dispatch→encode→send loop.
    let route_handler = move |ws: WebSocketUpgrade| {
        let proto = Arc::clone(&proto);
        let shutdown = shutdown.clone();
        let runtime = runtime.clone();

        async move {
            ws.on_upgrade(move |socket| async move {
                let connection = match runtime
                    .open_scope(&crate::scope::Connection, Arc::clone(runtime.root()), Vec::new())
                    .await
                {
                    Ok(scope) => scope,

                    Err(error) => {
                        tracing::error!(
                            target: "overseerd::axum",
                            %error,
                            "ws connection scope build failed; closing socket"
                        );

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
