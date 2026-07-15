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

/// The [`Dto`](dto::Dto) wire-data marker. Available on every target (the generated client and the
/// server both assert it), so it lives outside the server gate.
pub mod dto;

/// The wasm-safe STOMP wire types (`StompBody`, `StompCodec`, `Topic`, `TopicParam`) shared by the
/// server broker and the browser client. Available on every target so the wasm client names them.
#[cfg(feature = "stomp")]
pub mod stomp;

/// Stream framing. The markers (`Ndjson`/`RawStream`), the `StreamEncode` contract, and the
/// NDJSON encode/decode helpers are pure and compile everywhere (the generated streaming client
/// names them); the axum extractor/response impls inside are gated to non-wasm.
pub mod stream;

// Server-only modules: the DI bridge, controllers, the serve loop, and the ws broker. None of
// these exist on a wasm client target, where only the generated HTTP client is compiled.
#[cfg(not(target_family = "wasm"))]
pub mod config;
#[cfg(not(target_family = "wasm"))]
pub mod controller;
#[cfg(not(target_family = "wasm"))]
pub mod error;
#[cfg(not(target_family = "wasm"))]
pub mod extract;
#[cfg(not(target_family = "wasm"))]
pub mod middleware;
#[cfg(not(target_family = "wasm"))]
pub mod plugin;
#[cfg(not(target_family = "wasm"))]
pub mod protocol;
#[cfg(not(target_family = "wasm"))]
pub mod request_meta;
#[cfg(not(target_family = "wasm"))]
pub mod scope;
#[cfg(all(feature = "ws", not(target_family = "wasm")))]
pub mod ws;

#[cfg(not(target_family = "wasm"))]
pub use config::{AXUM_CONFIG_PATH, AxumConfig};
#[cfg(not(target_family = "wasm"))]
pub use controller::{CONTROLLERS, Controller, ControllerDescriptor};
#[cfg(not(target_family = "wasm"))]
pub use error::{Error, Result};

#[cfg(all(feature = "ws", not(target_family = "wasm")))]
pub use ws::{
    JsonWs, WS_CONTROLLERS, WebsocketController, WebsocketHandler, WebsocketProtocol, WsCodec,
    WsControllerDescriptor, WsDispatchError, WsReply, WsRespond, WsRoute, WsShutdown,
};

/// The wasm-safe STOMP wire types, re-exported at the crate root on every target — the browser
/// client's generated `#[topics]`/`#[message]` code names them through the plugin path.
#[cfg(feature = "stomp")]
pub use stomp::{JsonCodec, StompBody, StompCodec, Topic, TopicParam};

/// The STOMP pub/sub protocol surface (server side): the [`Stomp`](ws::stomp::Stomp) protocol and
/// its broker/session/publish types. Server-only.
#[cfg(all(feature = "stomp", not(target_family = "wasm")))]
pub use ws::stomp::{
    Broker, Direct, Injected, IntoAuthenticator, Publish, Publisher, ResolvedAuthenticator, Stomp,
    StompAuthFuture, StompAuthenticationError, StompAuthenticator, StompConfig, StompConnect,
    StompError, StompHeaders, StompOutcome, StompPrincipal, StompSession, StompTopicBus,
};

/// Re-exported so `#[topics]`-generated `Topic::encode` impls name the codec error without a
/// separate `overseerd-transport` dependency.
#[cfg(feature = "stomp")]
pub use overseerd_transport::CodecError;

/// The `multipart/form-data` request extractor, re-exported from axum under the `multipart` feature
/// so a handler reaches it through the facade (`overseerd::axum::Multipart`) — pairing with the
/// client-side [`client::Multipart`] body builder used to send the upload.
#[cfg(all(feature = "multipart", not(target_family = "wasm")))]
pub use axum::extract::Multipart;
#[cfg(not(target_family = "wasm"))]
pub use extract::{Inject, InjectRejection, ScopeHandle};
#[cfg(not(target_family = "wasm"))]
pub use middleware::AxumMiddleware;
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
#[cfg(not(target_family = "wasm"))]
pub use plugin::{AxumAppBuilder, AxumAppServe, AxumPlugin};
#[cfg(not(target_family = "wasm"))]
pub use protocol::Axum;
#[cfg(not(target_family = "wasm"))]
pub use request_meta::RequestMeta;
/// The `StreamBody` request extractor is an axum `FromRequest` — server-only.
#[cfg(not(target_family = "wasm"))]
pub use stream::StreamBody;
/// Pure stream framing, available on every target (the generated streaming client names these).
pub use stream::{Ndjson, RawStream, StreamEncode, chunk_u8};

/// Re-exported so streaming-client codegen can project a concrete stream's item type
/// (`<S as Stream>::Item`) and name the item stream it returns. Referenced only by generated code.
#[doc(hidden)]
pub use futures::Stream as __Stream;

/// The [`Dto`](dto::Dto) wire-data marker, re-exported at the crate root so `#[dto]`-generated impls
/// and the handler assertions name it through a stable path on every target.
pub use dto::Dto;

/// The axum controller macros' companion `#[dto]` attribute: derives `serde` (+ `tsify::Tsify` on
/// wasm) and marks a type [`Dto`] so it may cross the HTTP wire.
pub use overseerd_axum_macros::dto;

/// The `bytes` crate, re-exported so raw-stream client codegen names `bytes::Bytes` through a
/// stable path (`::overseerd_axum::bytes`) without depending on axum's re-export — the client
/// body/stream path stays wasm-safe. Referenced by generated code and available on every target.
pub use bytes;

/// Re-exported so `#[message]` ws-handler codegen can name the per-message scope container the
/// generated handler resolves its `Inject<T>` parameters from. Referenced only by generated code.
#[cfg(all(feature = "ws", not(target_family = "wasm")))]
#[doc(hidden)]
pub use overseerd_di::ScopeContainer as __ScopeContainer;

/// The axum app type: an [`App`](overseerd_app::App) specialized to [`AxumPlugin`].
/// `App::builder(name)` resolves through this alias without a turbofish.
#[cfg(not(target_family = "wasm"))]
pub type App = overseerd_app::App<AxumPlugin>;

/// The axum app builder: [`AppBuilder`](overseerd_app::AppBuilder) specialized to [`AxumPlugin`].
#[cfg(not(target_family = "wasm"))]
pub type AppBuilder = overseerd_app::AppBuilder<AxumPlugin>;

// Re-export the agnostic app surface so a standalone `overseerd-axum` user has one import.
#[cfg(not(target_family = "wasm"))]
pub use overseerd_app::{
    AppRegistry, AppRuntime, LoggingConfig, Plugin, Protocol, ProtocolPlugin, Serve, ServerConfig,
    ShutdownHandle, ShutdownSignal,
};

/// Re-exported so macro-generated code can reach the `#[distributed_slice]` attribute for
/// the `CONTROLLERS` slice through a stable path.
#[cfg(not(target_family = "wasm"))]
#[doc(hidden)]
pub use linkme;

/// Re-exported so `#[controller]`/`#[handlers]` generated code and users reach axum through a
/// stable path without a separate dependency.
#[cfg(not(target_family = "wasm"))]
pub use axum;

/// Re-exported (mirroring the RPC protocol's own `pub use tower;`) so a raw `tower::Layer`
/// registered via [`AxumAppBuilder::layer`] — or a test driving the router with
/// `tower::ServiceExt::oneshot` — needs no separate `tower` dependency.
#[cfg(not(target_family = "wasm"))]
pub use tower;

/// The `http` crate (verb, headers, request/response), re-exported at the crate root so a
/// dependant resolves `::overseerd_axum::http` — the path the generated client builds its
/// `http::Request` against. Natively this rides on axum's re-export; on wasm (no axum) the `http`
/// crate is depended on directly via the `client` feature.
#[cfg(not(target_family = "wasm"))]
pub use axum::http;
#[cfg(all(target_family = "wasm", feature = "client"))]
pub use http;
