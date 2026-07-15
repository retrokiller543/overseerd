//! The wasm-safe, protocol-generic messaging wire contract shared by every WebSocket pub/sub
//! protocol.
//!
//! The protocol capability traits ([`MessagingProtocol`], [`MessagingClientProtocol`]), the pluggable
//! [`TopicCodec`], the [`Topic`] contract, and the [`TopicParam`] renderer are pure (bytes + serde,
//! no tokio/axum), so they compile on every target: the server broker (native) and the browser
//! client (wasm) both name them. They live behind `feature = "ws"` — not `stomp` — so a protocol
//! other than STOMP (a user-defined one, a future WAMP) reuses the whole topics/messages machinery
//! by adding its own impls, without enabling STOMP. STOMP is one implementation, in
//! [`crate::stomp`].

use std::borrow::Cow;

use overseerd_transport::CodecError;

/// A pub/sub WebSocket protocol's messaging wire vocabulary: the frame body a topic value encodes
/// to, and the codec used when a topic set names none. A protocol "has topics/messages" by
/// implementing this (plus [`MessagingClientProtocol`] for a client and
/// [`PubSubProtocol`](crate::ws::PubSubProtocol) for server publish); `#[topics]`, [`Topic`], the
/// publisher, and the client are all generic over it, so STOMP is one implementation and another
/// protocol adds its own without touching the machinery.
pub trait MessagingProtocol: 'static {
    /// The wire body a [`Topic`] value encodes to and the broker fans out to subscribers. Its
    /// `Default` is the empty body a no-payload `#[message]` SEND ships. This body is shared by both
    /// surfaces of a pub/sub-capable protocol: topic pub/sub (`#[topics]`) and point-to-point
    /// messages (`#[message]`).
    type Body: Clone + Default + Send + Sync + 'static;

    /// The codec a topic set uses when it names none via `#[topics(codec = ..)]`.
    type DefaultCodec: TopicCodec<Self>;
}

/// A [`MessagingProtocol`] that also exposes a client. Adds the opaque error status a client
/// surfaces; the framework never inspects it — a protocol owns what its failures mean. Its `Body`
/// (from [`MessagingProtocol`]) and `Status` are shared by both client surfaces: topic subscription
/// ([`TopicSubscribe`](crate::client::TopicSubscribe)) and point-to-point messages
/// ([`MessageSend`](crate::client::MessageSend) / `MessageRequest`).
pub trait MessagingClientProtocol: MessagingProtocol {
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
    type Protocol: MessagingProtocol;

    /// This value's destination. A static `#[topic("/topic/x")]` borrows the literal; a templated
    /// `#[topic("/topic/{room}")]` substitutes the variant's typed fields into an owned string —
    /// hence [`Cow`](std::borrow::Cow), so a static topic still allocates nothing.
    fn destination(&self) -> Cow<'static, str>;

    /// Serializes this value's payload into the protocol's body (using the topic set's codec).
    fn encode(&self) -> Result<<Self::Protocol as MessagingProtocol>::Body, CodecError>;
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
/// its default via [`MessagingProtocol::DefaultCodec`]. The same codec type is named on both sides,
/// so both directions follow it. For STOMP, implement the simpler
/// [`StompCodec`](crate::stomp::StompCodec) — a blanket impl makes any `StompCodec` a
/// `TopicCodec<Stomp>`.
pub trait TopicCodec<P>: Send + Sync + 'static
where
    P: MessagingProtocol + ?Sized,
{
    /// Serializes a payload into the protocol body.
    fn encode<T: serde::Serialize>(value: &T) -> Result<P::Body, CodecError>;

    /// Deserializes a payload from the protocol body.
    fn decode<T: serde::de::DeserializeOwned>(body: P::Body) -> Result<T, CodecError>;
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

    fn encode(&self) -> Result<<T::Protocol as MessagingProtocol>::Body, CodecError> {
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
/// without `ws` is a harmless no-op.
#[cfg(feature = "uuid")]
impl TopicParam for uuid::Uuid {
    fn render(&self) -> String {
        self.to_string()
    }
}
