//! STOMP's client: its opaque status, its [`MessagingClientProtocol`] impl, the concrete transport
//! actor, and STOMP's wasm [`TopicWasmClient`](crate::client::TopicWasmClient) impl.
//!
//! The protocol-generic client capabilities ([`MessageSend`](crate::client::MessageSend),
//! [`MessageRequest`](crate::client::MessageRequest), [`TopicSubscribe`](crate::client::TopicSubscribe),
//! [`Subscription`](crate::client::Subscription)) live in [`crate::client::messaging`]. This module
//! supplies STOMP's implementations of them.
//!
//! STOMP's transport sends no heart-beats in v1 (see `docs/stomp.md` for the deferred-feature list).

use crate::Stomp;
use overseerd_axum::MessagingClientProtocol;

/// The status carried by a STOMP [`ClientError::Remote`](overseerd_client::ClientError::Remote),
/// mirroring [`WsStatus`](super::WsStatus). This is [`Stomp`]'s [`MessagingClientProtocol::Status`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StompStatus {
    /// The broker sent an `ERROR` frame (fatal — the connection is torn down).
    Error,

    /// A protocol violation (e.g. no `CONNECTED` during the handshake, or a version mismatch).
    Protocol,

    /// A request/response handler on the server returned an error. Non-fatal: only this one call
    /// resolves `Err`, and the connection stays open for further messages.
    Handler,
}

impl MessagingClientProtocol for Stomp {
    type Status = StompStatus;
}

#[cfg(feature = "tungstenite")]
#[path = "client/transport.rs"]
mod transport;

#[cfg(feature = "tungstenite")]
pub use transport::{StompClientTransport, StompConnectOptions};

// STOMP's `TopicWasmClient` impl (pulling its shared socket out of the browser `Connection`).
// wasm-only; the protocol-generic `TopicWasmClient`/`TopicSubscription`/`pump` live in
// `crate::client::messaging`.
#[cfg(all(target_family = "wasm", feature = "tungstenite"))]
#[path = "client/wasm.rs"]
mod wasm;

#[cfg(all(target_family = "wasm", feature = "tungstenite"))]
pub use wasm::{connect_stomp, connect_stomp_with_options, disconnect_stomp};
