//! Proves the topics seam is genuinely protocol-generic, not accidentally STOMP-shaped: a throwaway
//! `TestProto` — with its own non-`StompBody` body and codec — drives `Topic`, `TopicCodec`,
//! `Publisher`, and `TopicBus` end to end. If any of those had been welded to STOMP, this module
//! would not compile or the round trip would not deliver.

use std::borrow::Cow;
use std::sync::Arc;

use axum::extract::ws::WebSocket;
use overseerd_app::AppRuntime;
use overseerd_di::ScopeContainer;
use serde::{Deserialize, Serialize};

use crate::stomp::{Topic, TopicClientProtocol, TopicCodec, TopicProtocol};
use crate::ws::stomp::{Publisher, TopicBus};
use crate::ws::{PubSubProtocol, WebsocketProtocol, WsControllerDescriptor, WsShutdown};
use overseerd_transport::CodecError;

/// A throwaway pub/sub protocol used only to prove genericity. Its body is deliberately *not*
/// [`StompBody`](crate::stomp::StompBody).
struct TestProto;

/// `TestProto`'s wire body: a distinct type, so a body welded to STOMP would fail to compile here.
#[derive(Clone, Default)]
struct TestBody(Vec<u8>);

/// `TestProto`'s codec: length-prefix-free JSON over the custom body.
struct TestCodec;

impl TopicCodec<TestProto> for TestCodec {
    fn encode<T: Serialize>(value: &T) -> Result<TestBody, CodecError> {
        let bytes = serde_json::to_vec(value).map_err(|e| CodecError::internal(e.to_string()))?;

        Ok(TestBody(bytes))
    }

    fn decode<T: serde::de::DeserializeOwned>(body: TestBody) -> Result<T, CodecError> {
        serde_json::from_slice(&body.0).map_err(|e| CodecError::bad_input(e.to_string()))
    }
}

impl TopicProtocol for TestProto {
    type Body = TestBody;
    type DefaultCodec = TestCodec;
}

impl TopicClientProtocol for TestProto {
    type Status = ();
}

impl WebsocketProtocol for TestProto {
    type Payload = ();
    type Outcome = ();
    type Options = ();

    fn build(_: &[WsControllerDescriptor], _: &AppRuntime, _: ()) -> Self {
        TestProto
    }

    async fn serve(
        self: Arc<Self>,
        socket: WebSocket,
        connection: Arc<ScopeContainer>,
        shutdown: WsShutdown,
    ) {
        // A tag protocol for the topics test; it never actually drives a socket.
        let _ = (socket, connection, shutdown);
    }
}

impl PubSubProtocol for TestProto {
    type OutFrame = TestBody;

    fn frame_message(
        _message_id: u64,
        _destination: &str,
        _sub_id: &str,
        body: &TestBody,
        _headers: &[(String, String)],
    ) -> TestBody {
        body.clone()
    }
}

/// A payload carried by the test topic set.
#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
struct Ping {
    seq: u32,
}

/// A hand-written topic set for `TestProto` (equivalent to what `#[topics(protocol = TestProto)]`
/// would generate), proving the `Topic` contract is protocol-generic.
enum TestTopics {
    Tick(Ping),
}

impl Topic for TestTopics {
    type Protocol = TestProto;

    fn destination(&self) -> Cow<'static, str> {
        match self {
            TestTopics::Tick(_) => Cow::Borrowed("/test/tick"),
        }
    }

    fn encode(&self) -> Result<TestBody, CodecError> {
        match self {
            TestTopics::Tick(ping) => <TestCodec as TopicCodec<TestProto>>::encode(ping),
        }
    }
}

/// `Publisher<T>` is generic over any pub/sub protocol: this compiles only because
/// `TestTopics::Protocol = TestProto: PubSubProtocol`.
fn _publisher_is_protocol_generic(_: &Publisher<TestTopics>) {}

/// The generated controller/topics clients bind on `MessageSend<P>`, `MessageRequest<P>`, and
/// `TopicSubscribe<P>` — the capability traits the `#[message]`/`#[topics]` codegen emits. These
/// bounds resolve for the non-STOMP `TestProto` only because the traits carry no STOMP type, so a
/// transport that implements them for `TestProto` slots into the generated code unchanged. The `()`
/// transport implements all three for any `P: TopicClientProtocol`, so naming it here is the
/// compile-time proof that the client seam is protocol-generic.
#[cfg(feature = "client")]
fn _message_client_is_protocol_generic() {
    use crate::client::{MessageRequest, MessageSend, TopicSubscribe};

    fn assert_send<C: MessageSend<TestProto>>() {}
    fn assert_request<C: MessageRequest<TestProto>>() {}
    fn assert_subscribe<C: TopicSubscribe<TestProto>>() {}

    assert_send::<()>();
    assert_request::<()>();
    assert_subscribe::<()>();
}

#[tokio::test]
async fn topic_bus_round_trips_over_a_non_stomp_protocol() {
    let bus = TopicBus::<TestProto>::new();
    let registry = bus.registry();

    let (tx, mut rx) = tokio::sync::mpsc::channel::<TestBody>(4);
    let conn = registry.register();
    registry.subscribe(conn, "sub-1", "/test/tick", tx);

    bus.emit(TestTopics::Tick(Ping { seq: 7 }))
        .expect("the topic encodes and fans out");

    let frame = rx
        .try_recv()
        .expect("the subscriber receives a framed body");
    let ping: Ping = <TestCodec as TopicCodec<TestProto>>::decode(frame).expect("body decodes");

    assert_eq!(ping, Ping { seq: 7 });
}
