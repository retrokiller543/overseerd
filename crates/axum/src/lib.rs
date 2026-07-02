//! The Overseerd axum/HTTP protocol, built on the protocol-agnostic `overseerd-app` core.
//!
//! This crate is a [`ProtocolPlugin`]: it builds a real [`axum::Router`] from `#[controller]`
//! components, bridges the framework's dependency injection into axum via the [`Inject`]
//! extractor (so route handlers mix native axum extractors with DI), and serves over HTTP.
//! Depend on it directly, or reach it through the `overseerd` facade's `axum` feature.
//!
//! The DI bridge is deliberately thin and one-directional: nothing in `overseerd-di` or
//! `overseerd-core` knows axum exists. A per-request scope layer threads an
//! [`Arc<ScopeContainer>`](overseerd_di::ScopeContainer) through the request extensions, and
//! [`Inject`] resolves components from it.

#[cfg(feature = "client")]
pub mod client;
pub mod controller;
pub mod error;
pub mod extract;
pub mod plugin;
pub mod protocol;
pub mod scope;
pub mod stream;
#[cfg(feature = "ws")]
pub mod ws;

pub use controller::{CONTROLLERS, Controller, ControllerDescriptor};
pub use error::{Error, Result};

#[cfg(feature = "ws")]
pub use ws::{
    JsonWs, WS_CONTROLLERS, WebsocketController, WebsocketHandler, WebsocketProtocol, WsCodec,
    WsControllerDescriptor, WsDispatchError, WsReply, WsRespond, WsRoute, WsShutdown,
};

/// The STOMP pub/sub protocol surface (server side): the [`Stomp`](ws::stomp::Stomp) protocol, its
/// broker/session/publish types, and the [`Topic`](ws::stomp::Topic) contract.
#[cfg(feature = "stomp")]
pub use ws::stomp::{
    Broker, JsonCodec, Publish, Publisher, Stomp, StompBody, StompCodec, StompConfig, StompError,
    StompHeaders, StompOutcome, StompSession, Topic, TopicParam,
};

/// Re-exported so `#[topics]`-generated `Topic::encode` impls name the codec error without a
/// separate `overseerd-transport` dependency.
#[cfg(feature = "stomp")]
pub use overseerd_transport::CodecError;

pub use extract::{Inject, InjectRejection, ScopeHandle};
/// The STOMP topic-set macro (`#[topics]`).
#[cfg(feature = "stomp")]
pub use overseerd_axum_macros::topics;
/// The axum controller macros (`#[controller]`, `#[handlers]`, the route attributes), owned by
/// this protocol crate. Their generated code roots plugin types at this crate
/// (`::overseerd_axum::*`) by default, or at `::overseerd::axum::*` under the `facade` feature —
/// so they work whether `overseerd-axum` is used directly or through the `overseerd` facade. The
/// core macros (`app!`, `#[component]`, …) come from `overseerd` (the always-present core).
pub use overseerd_axum_macros::{
    controller, delete, get, handlers, head, message, options, patch, post, put, route,
};
pub use plugin::{AxumAppBuilder, AxumPlugin};
pub use protocol::Axum;
pub use stream::{Ndjson, RawStream, StreamBody, StreamEncode, chunk_u8};

/// Re-exported so streaming-client codegen can project a concrete stream's item type
/// (`<S as Stream>::Item`) and name the item stream it returns. Referenced only by generated code.
#[doc(hidden)]
pub use futures::Stream as __Stream;

/// Re-exported so `#[message]` ws-handler codegen can name the per-message scope container the
/// generated handler resolves its `Inject<T>` parameters from. Referenced only by generated code.
#[cfg(feature = "ws")]
#[doc(hidden)]
pub use overseerd_di::ScopeContainer as __ScopeContainer;

/// The axum app type: an [`App`](overseerd_app::App) specialized to [`AxumPlugin`].
/// `App::builder(name)` resolves through this alias without a turbofish.
pub type App = overseerd_app::App<AxumPlugin>;

/// The axum app builder: [`AppBuilder`](overseerd_app::AppBuilder) specialized to [`AxumPlugin`].
pub type AppBuilder = overseerd_app::AppBuilder<AxumPlugin>;

// Re-export the agnostic app surface so a standalone `overseerd-axum` user has one import.
pub use overseerd_app::{
    AppRegistry, AppRuntime, LoggingConfig, Plugin, Protocol, ProtocolPlugin, Serve, ServerConfig,
    ShutdownHandle, ShutdownSignal,
};

/// Re-exported so macro-generated code can reach the `#[distributed_slice]` attribute for
/// the `CONTROLLERS` slice through a stable path.
#[doc(hidden)]
pub use linkme;

/// Re-exported so `#[controller]`/`#[handlers]` generated code and users reach axum through a
/// stable path without a separate dependency.
pub use axum;

/// The `http` crate (verb, headers, request/response), re-exported at the crate root so a
/// standalone `overseerd-axum` dependant resolves `::overseerd_axum::http` — the path the
/// generated client builds its `http::Request` against.
pub use axum::http;
