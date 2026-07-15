//! The wasm-safe topic wire contract and STOMP's implementation of it.
//!
//! The protocol capability traits ([`TopicProtocol`], [`TopicClientProtocol`]), the pluggable
//! [`TopicCodec`], and the [`Topic`] contract are pure (bytes + serde, no tokio/axum), so they
//! compile on every target: the server broker (native) and the browser client (wasm) both name
//! them. They are generic over the protocol, so STOMP is one implementation and a new protocol
//! (WAMP, a user-defined one) reuses the whole topics machinery by adding its own impls.
//!
//! The [`Stomp`] type itself lives here (not the server-only `ws` module) so a `#[topics(protocol =
//! Stomp)]` set and its generated client can name it on wasm; its server-only state is behind a
//! `cfg`, so on wasm it is a fieldless protocol tag. The stateful server behavior
//! (`WebsocketProtocol`, the broker serve loop) is implemented in [`crate::ws::stomp`]. The
//! server-only fan-out types ([`Publish`](crate::ws::stomp::Publish) /
//! [`StompOutcome`](crate::ws::stomp::StompOutcome)) also stay in that module.

use std::borrow::Cow;

use bytes::Bytes;
use overseerd_transport::CodecError;

#[cfg(not(target_family = "wasm"))]
use std::collections::HashMap;
#[cfg(not(target_family = "wasm"))]
use std::sync::Arc;

#[cfg(not(target_family = "wasm"))]
use overseerd_app::AppRuntime;

#[cfg(not(target_family = "wasm"))]
use crate::ws::WsHandlerFn;
#[cfg(not(target_family = "wasm"))]
use crate::ws::stomp::{Broker, StompConfig};

/// A pub/sub WebSocket protocol's topic wire vocabulary: the frame body a topic value encodes to,
/// and the codec used when a topic set names none. A protocol "has topics" by implementing this
/// (plus [`TopicClientProtocol`] for a client and [`PubSubProtocol`](crate::ws::PubSubProtocol) for
/// server publish); `#[topics]`, [`Topic`], the publisher, and the client are all generic over it,
/// so STOMP is one implementation and another protocol adds its own without touching the topics
/// machinery.
pub trait TopicProtocol: 'static {
    /// The wire body a [`Topic`] value encodes to and the broker fans out to subscribers.
    type Body: Clone + Send + Sync + 'static;

    /// The codec a topic set uses when it names none via `#[topics(codec = ..)]`.
    type DefaultCodec: TopicCodec<Self>;
}

/// A [`TopicProtocol`] that also exposes a client. Adds the opaque error status a client surfaces;
/// the framework never inspects it — a protocol owns what its failures mean.
pub trait TopicClientProtocol: TopicProtocol {
    /// The status carried by a client [`ClientError`](overseerd_client::ClientError) for this
    /// protocol (STOMP uses [`StompStatus`](crate::client::StompStatus)).
    type Status: std::fmt::Debug + Clone + Send + 'static;
}

/// A set of broadcast topics declared once and shared by client and server — the guardrail against
/// client/server drift. Each implementor (an enum via `#[topics]`) names its
/// [`Protocol`](Self::Protocol), maps a value to its destination, and serializes its payload;
/// because a value can only be built with the right payload type, the wrong type can never reach a
/// topic.
pub trait Topic {
    /// The protocol this topic set is published over — determines the wire body and the bus.
    type Protocol: TopicProtocol;

    /// This value's destination. A static `#[topic("/topic/x")]` borrows the literal; a templated
    /// `#[topic("/topic/{room}")]` substitutes the variant's typed fields into an owned string —
    /// hence [`Cow`](std::borrow::Cow), so a static topic still allocates nothing.
    fn destination(&self) -> Cow<'static, str>;

    /// Serializes this value's payload into the protocol's body (using the topic set's codec).
    fn encode(&self) -> Result<<Self::Protocol as TopicProtocol>::Body, CodecError>;
}

/// A typed value that fills one `{name}` hole in a templated [`Topic`] destination — on the server
/// when building the destination to publish to, and on the client as a `subscribe_*` argument. The
/// same rendering runs on both sides, so a param round-trips: whatever `render` produces is what a
/// subscriber must pass.
///
/// Implemented for the common std/core path-segment types (strings, integers, `bool`). It is
/// **not** a blanket `Display` impl — that would seal the trait and forbid a user newtype (e.g. a
/// `RoomId`) from implementing it. For a custom id type, add a one-line impl (usually delegating to
/// `Display`): `impl TopicParam for RoomId { fn render(&self) -> String { self.to_string() } }`.
pub trait TopicParam {
    /// Renders this value into its path segment.
    fn render(&self) -> String;
}

/// The wire codec for a topic set's message bodies, generic over the protocol `P` whose body it
/// produces and consumes — how a payload is serialized on the server's publish and deserialized on
/// the client's subscribe. Selected per topic set with `#[topics(codec = ..)]`; a protocol supplies
/// its default via [`TopicProtocol::DefaultCodec`]. The same codec type is named on both sides, so
/// both directions follow it. For STOMP, implement the simpler [`StompCodec`] — a blanket impl makes
/// any `StompCodec` a `TopicCodec<Stomp>`.
pub trait TopicCodec<P>: Send + Sync + 'static
where
    P: TopicProtocol + ?Sized,
{
    /// Serializes a payload into the protocol body.
    fn encode<T: serde::Serialize>(value: &T) -> Result<P::Body, CodecError>;

    /// Deserializes a payload from the protocol body.
    fn decode<T: serde::de::DeserializeOwned>(body: P::Body) -> Result<T, CodecError>;
}

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

/// Implements [`TopicParam`] for a list of types by delegating to their [`Display`](std::fmt::Display).
macro_rules! topic_param_via_display {
    ($($ty:ty),* $(,)?) => {
        $(
            impl TopicParam for $ty {
                fn render(&self) -> String {
                    ::std::string::ToString::to_string(self)
                }
            }
        )*
    };
}

topic_param_via_display! {
    String, &str, bool, char,
    u8, u16, u32, u64, u128, usize,
    i8, i16, i32, i64, i128, isize,
}

impl<T: Topic> Topic for &T {
    type Protocol = T::Protocol;

    fn destination(&self) -> Cow<'static, str> {
        T::destination(self)
    }

    fn encode(&self) -> Result<<T::Protocol as TopicProtocol>::Body, CodecError> {
        T::encode(self)
    }
}

impl<'a, T: TopicParam + Clone> TopicParam for Cow<'a, T> {
    fn render(&self) -> String {
        self.as_ref().render()
    }
}

impl<'a> TopicParam for Cow<'a, str> {
    fn render(&self) -> String {
        self.to_string()
    }
}

/// A [`Uuid`](uuid::Uuid) renders as its hyphenated string. A common id type for templated topics
/// (`/topic/room/{id}`). Gated on the cross-cutting `uuid` integration flag, so enabling `uuid`
/// without `stomp` is a harmless no-op.
#[cfg(feature = "uuid")]
impl TopicParam for uuid::Uuid {
    fn render(&self) -> String {
        self.to_string()
    }
}

/// The STOMP protocol tag and (on the server) its stateful pub/sub state.
///
/// On wasm this is a fieldless tag: it names the protocol for a `#[topics(protocol = Stomp)]` set
/// and the generated client's `TopicSend<Stomp>`/`TopicSubscribe<Stomp>` bounds. On the server its
/// `cfg`-gated fields hold the `/app/**` handler table, the shared [`Broker`], the runtime, and the
/// endpoint config; the [`WebsocketProtocol`](crate::ws::WebsocketProtocol) impl and serve loop live
/// in [`crate::ws::stomp`].
pub struct Stomp {
    /// The `/app/**` destination → `#[message]` handler table.
    #[cfg(not(target_family = "wasm"))]
    pub(crate) app_routes: HashMap<&'static str, WsHandlerFn<Stomp>>,

    /// The shared broker for `/topic/**` fan-out (one per endpoint, cloned from the DI bus).
    #[cfg(not(target_family = "wasm"))]
    pub(crate) broker: Arc<Broker>,

    /// The runtime, kept to open per-message [`Request`](crate::scope::Request) scopes while serving.
    #[cfg(not(target_family = "wasm"))]
    pub(crate) runtime: AppRuntime,

    /// Heart-beat/version/authenticator policy for this endpoint.
    #[cfg(not(target_family = "wasm"))]
    pub(crate) config: StompConfig,
}

impl TopicProtocol for Stomp {
    type Body = StompBody;
    type DefaultCodec = JsonCodec;
}

/// The wire codec for a STOMP topic set's message bodies — how a payload is serialized on the
/// server's publish and deserialized on the client's subscribe. Selected per topic set with
/// `#[topics(codec = ..)]`; defaults to [`JsonCodec`]. Provide your own for a more compact wire
/// format (e.g. a postcard-backed codec) and both directions follow it, since the same codec type
/// is named on both sides. A blanket impl makes every `StompCodec` a `TopicCodec<Stomp>`, so it
/// slots into the protocol-generic topics machinery.
pub trait StompCodec: Send + Sync + 'static {
    /// Serializes a payload into a body.
    fn encode<T: serde::Serialize>(value: &T) -> Result<StompBody, CodecError>;

    /// Deserializes a payload from a body.
    fn decode<T: serde::de::DeserializeOwned>(body: StompBody) -> Result<T, CodecError>;
}

impl<C> TopicCodec<Stomp> for C
where
    C: StompCodec,
{
    fn encode<T: serde::Serialize>(value: &T) -> Result<StompBody, CodecError> {
        <C as StompCodec>::encode(value)
    }

    fn decode<T: serde::de::DeserializeOwned>(body: StompBody) -> Result<T, CodecError> {
        <C as StompCodec>::decode(body)
    }
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
