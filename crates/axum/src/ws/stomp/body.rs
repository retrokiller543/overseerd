//! STOMP message bodies, the handler outcome, and the [`Topic`] contract.
//!
//! [`StompBody`] is the protocol's [`Payload`](crate::ws::WebsocketProtocol::Payload): opaque bytes
//! plus an optional content type, decoded into a handler parameter by
//! [`WsCodec`](crate::ws::WsCodec) and produced by [`Topic::encode`]. [`StompOutcome`] is the
//! protocol's [`Outcome`](crate::ws::WebsocketProtocol::Outcome): what a `#[message]` handler
//! returns, either nothing or a set of fan-out [`Publish`]es.

// The wire contract (the protocol capability traits, the pluggable codec, the `Topic` contract, and
// STOMP's `StompBody`/`StompCodec`/`JsonCodec`) lives in the wasm-safe `crate::stomp` module so the
// browser client can name it too; re-exported here so the broker's internal `crate::ws::stomp::*`
// paths and the crate's public surface are unchanged.
pub use crate::stomp::{
    JsonCodec, StompBody, StompCodec, Topic, TopicClientProtocol, TopicCodec, TopicParam,
    TopicProtocol,
};

/// One outbound fan-out: a destination, its body, and any extra headers to attach to the `MESSAGE`.
pub struct Publish {
    /// The destination to broadcast to (e.g. `/topic/room`).
    pub destination: String,

    /// The message body.
    pub body: StompBody,

    /// Extra headers to set on the outbound `MESSAGE` frame.
    pub headers: Vec<(String, String)>,
}

impl Publish {
    /// A publish to `destination` carrying `body`, with no extra headers.
    pub fn new(destination: impl Into<String>, body: StompBody) -> Self {
        Self {
            destination: destination.into(),
            body,
            headers: Vec::new(),
        }
    }
}

/// What a STOMP `#[message]` handler yields before framing: nothing (the common case — handlers
/// publish imperatively through an injected [`Publisher`](super::Publisher)), or an explicit set of
/// fan-out publishes. Produced from the handler's return value by
/// [`WsRespond`](crate::ws::WsRespond).
pub enum StompOutcome {
    /// The handler produced no direct output (it may still have published via a `Publisher`).
    Nothing,

    /// The handler asks the broker to fan these out.
    Publish(Vec<Publish>),
}
