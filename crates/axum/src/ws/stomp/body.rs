//! STOMP message bodies, the handler outcome, and the [`Topic`] contract.
//!
//! [`StompBody`] is the protocol's [`Payload`](crate::ws::WebsocketProtocol::Payload): opaque bytes
//! plus an optional content type, decoded into a handler parameter by
//! [`WsCodec`](crate::ws::WsCodec) and produced by [`Topic::encode`]. [`StompOutcome`] is the
//! protocol's [`Outcome`](crate::ws::WebsocketProtocol::Outcome): what a `#[message]` handler
//! returns, either nothing or a set of fan-out [`Publish`]es.

use bytes::Bytes;
use overseerd_transport::CodecError;

/// A STOMP frame body: opaque bytes with an optional `content-type`. Handlers usually receive a
/// decoded type (via [`WsCodec`](crate::ws::WsCodec) JSON decoding) rather than this directly.
#[derive(Clone, Debug, Default)]
pub struct StompBody {
    /// The `content-type` header value, if the frame carried one.
    pub content_type: Option<String>,

    /// The raw body bytes.
    pub bytes: Bytes,
}

impl StompBody {
    /// A JSON body: the bytes plus `application/json`.
    pub fn json(bytes: impl Into<Bytes>) -> Self {
        Self {
            content_type: Some("application/json".to_owned()),
            bytes: bytes.into(),
        }
    }

    /// Serializes `value` to a JSON body. Used by `#[topics]`-generated [`Topic::encode`] impls.
    pub fn from_serialize<T: serde::Serialize>(value: &T) -> Result<Self, CodecError> {
        let bytes = serde_json::to_vec(value).map_err(|e| CodecError::internal(e.to_string()))?;

        Ok(Self::json(bytes))
    }
}

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

/// A set of broadcast topics declared once and shared by client and server — the guardrail against
/// client/server drift. Each implementor (an enum via `#[topics]`) maps a value to its destination
/// and serializes its payload; because a value can only be built with the right payload type, the
/// wrong type can never reach a topic.
pub trait Topic {
    /// This value's destination (the variant's `#[topic("..")]`).
    fn destination(&self) -> &'static str;

    /// Serializes this value's payload into a [`StompBody`] (using the topic set's [`StompCodec`]).
    fn encode(&self) -> Result<StompBody, CodecError>;
}

/// The wire codec for a topic set's message bodies — how a payload is serialized on the server's
/// publish and deserialized on the client's subscribe. Selected per topic set with
/// `#[topics(codec = ..)]`; defaults to [`JsonCodec`]. Provide your own for a more compact wire
/// format (e.g. a postcard-backed codec) and both directions follow it, since the same codec type
/// is named on both sides.
pub trait StompCodec: Send + Sync + 'static {
    /// Serializes a payload into a body.
    fn encode<T: serde::Serialize>(value: &T) -> Result<StompBody, CodecError>;

    /// Deserializes a payload from a body.
    fn decode<T: serde::de::DeserializeOwned>(body: StompBody) -> Result<T, CodecError>;
}

/// The default [`StompCodec`]: JSON bodies (`application/json`).
pub struct JsonCodec;

impl StompCodec for JsonCodec {
    fn encode<T: serde::Serialize>(value: &T) -> Result<StompBody, CodecError> {
        StompBody::from_serialize(value)
    }

    fn decode<T: serde::de::DeserializeOwned>(body: StompBody) -> Result<T, CodecError> {
        serde_json::from_slice(&body.bytes).map_err(|e| CodecError::bad_input(e.to_string()))
    }
}
