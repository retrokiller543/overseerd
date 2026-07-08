//! End-to-end test of a **STOMP pub/sub controller**: build the app, serve it on an ephemeral
//! port, then drive it with the *generated typed clients* — no destination strings at the call
//! sites. One client subscribes to a topic via `ChatTopicsClient::subscribe_room()`; another sends
//! to an app destination via `ChatControllerClient::chat(..)`, whose handler publishes to the
//! topic; the subscriber's typed `Subscription` stream then yields the broadcast message. The
//! server is shut down at the end so the test never hangs.

use futures::StreamExt;
use overseerd::axum::client::{ReqwestClient, StompClientTransport};
use overseerd::axum::prelude::*;
use overseerd::axum::{CodecError, StompBody, StompCodec};
use overseerd::prelude::*;
use serde::Serialize;
use serde::de::DeserializeOwned;
use tokio::net::TcpListener;

/// A message a client sends to the app (`/app/chat`).
#[dto]
struct SendChat {
    text: String,
}

/// A message broadcast to subscribers of `/topic/room`.
#[dto]
#[derive(Clone)]
struct RoomMsg {
    text: String,
}

/// The app's broadcast topics — the single source of truth for both sides. Generates
/// `impl Topic for ChatTopics` (server publish) and `ChatTopicsClient<C>::subscribe_room()`
/// (client subscribe), typed to `RoomMsg`.
#[topics]
enum ChatTopics {
    #[topic("/topic/room")]
    Room(RoomMsg),
}

/// A STOMP controller: inbound `SEND /app/chat` is handled and re-broadcast to `/topic/room`.
#[controller(ws = Stomp)]
struct Chat {}

#[handlers(ws = Stomp)]
impl Chat {
    /// Handles an inbound chat SEND and publishes it to the room topic (typed, via `Publisher`).
    #[message("/app/chat")]
    async fn chat(
        &self,
        msg: SendChat,
        Inject(publisher): Inject<Publisher<ChatTopics>>,
    ) -> Result<(), CodecError> {
        publisher
            .publish_to::<1>(ChatTopics::Room(RoomMsg { text: msg.text }))
            .await
    }
}

/// A normal HTTP controller can publish to the same STOMP topics as a STOMP controller.
#[controller(path = "/events")]
struct RestEvents {}

#[handlers]
impl RestEvents {
    #[post("/room")]
    async fn room(
        &self,
        Inject(publisher): Inject<Publisher<ChatTopics>>,
        Json(msg): Json<RoomMsg>,
    ) -> Json<RoomMsg> {
        // A REST endpoint emits fire-and-forget: no `await`, just the encode result.
        publisher
            .emit(ChatTopics::Room(msg.clone()))
            .expect("test topic encodes");

        Json(msg)
    }
}

#[tokio::test]
async fn stomp_send_is_broadcast_to_typed_subscribers() {
    let app = app! {
        name: "stomp-test",
        protocol: overseerd::axum::AxumPlugin,
    }
    .register_ws::<Stomp>("/stomp")
    .build()
    .await
    .expect("app builds");

    let listener = TcpListener::bind(("127.0.0.1", 0)).await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    let shutdown = app.shutdown_handle();
    let server = tokio::spawn(async move { app.serve(listener).await });

    let url = format!("ws://{addr}/stomp");

    // ONE connection, shared across both typed client facades. `StompClientTransport` *is* the
    // connection and is cheaply `Clone` (a handle onto one actor + socket); cloning it does not
    // dial again. Send and subscribe here therefore ride the same socket.
    let connection = StompClientTransport::connect(&url).await.expect("connects");

    let mut room = ChatTopicsClient::new(connection.clone())
        .subscribe_room()
        .await
        .expect("subscribe_room");

    // Send to /app/chat via the generated typed method (no destination string), over the same
    // connection; the handler re-broadcasts to /topic/room, which this same socket is subscribed to.
    ChatClient::new(connection.clone())
        .chat(SendChat {
            text: "hello stomp".into(),
        })
        .await
        .expect("chat send");

    // The handler re-broadcast to /topic/room; the typed subscription yields the decoded message.
    let received = tokio::time::timeout(std::time::Duration::from_secs(5), room.next())
        .await
        .expect("a broadcast arrives before timeout")
        .expect("the subscription stream is live")
        .expect("a decoded RoomMsg");

    assert_eq!(received.text, "hello stomp");

    shutdown.shutdown();
    let _ = server.await;
}

#[tokio::test]
async fn http_handler_can_publish_to_typed_stomp_subscribers() {
    let app = app! {
        name: "stomp-http-publish-test",
        protocol: overseerd::axum::AxumPlugin,
    }
    .register_ws::<Stomp>("/stomp")
    .build()
    .await
    .expect("app builds");

    let listener = TcpListener::bind(("127.0.0.1", 0)).await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    let shutdown = app.shutdown_handle();
    let server = tokio::spawn(async move { app.serve(listener).await });

    let ws_url = format!("ws://{addr}/stomp");
    let connection = StompClientTransport::connect(&ws_url)
        .await
        .expect("connects");

    let mut room = ChatTopicsClient::new(connection)
        .subscribe_room()
        .await
        .expect("subscribe_room");

    let http = RestEventsClient::new(ReqwestClient::new(format!("http://{addr}")));
    let response = http
        .room(RoomMsg {
            text: "via rest".into(),
        })
        .await
        .expect("http publish");

    assert_eq!(response.text, "via rest");

    let received = tokio::time::timeout(std::time::Duration::from_secs(5), room.next())
        .await
        .expect("a REST-triggered broadcast arrives before timeout")
        .expect("the subscription stream is live")
        .expect("a decoded RoomMsg");

    assert_eq!(received.text, "via rest");

    shutdown.shutdown();
    let _ = server.await;
}

/// A deliberately non-JSON codec: it prepends a marker byte to the JSON and strips it on decode. If
/// either end of the SEND/broadcast path silently used plain JSON instead of this codec, the marker
/// byte would be missing (or unexpected) and decoding would fail — so a passing round trip proves
/// the codec is honored on **both** sides.
struct MarkedCodec;

const MARKER: u8 = 0xFE;

impl StompCodec for MarkedCodec {
    fn encode<T: Serialize>(value: &T) -> Result<StompBody, CodecError> {
        let mut bytes = vec![MARKER];
        bytes.extend(serde_json::to_vec(value).map_err(|e| CodecError::internal(e.to_string()))?);

        Ok(StompBody {
            content_type: Some("application/x-marked".to_owned()),
            bytes: bytes.into(),
        })
    }

    fn decode<T: DeserializeOwned>(body: StompBody) -> Result<T, CodecError> {
        match body.bytes.split_first() {
            Some((&MARKER, rest)) => {
                serde_json::from_slice(rest).map_err(|e| CodecError::bad_input(e.to_string()))
            }

            _ => Err(CodecError::bad_input(
                "body is missing the codec's marker byte",
            )),
        }
    }
}

/// A topic set using the custom codec on both publish and subscribe.
#[topics(codec = MarkedCodec)]
enum MarkedTopics {
    #[topic("/topic/marked")]
    Marked(RoomMsg),
}

/// A controller whose SEND payloads use the custom codec (via `codec = MarkedCodec`).
#[controller(ws = Stomp)]
struct Marked {}

#[handlers(ws = Stomp, codec = MarkedCodec)]
impl Marked {
    #[message("/app/marked")]
    async fn marked(
        &self,
        msg: SendChat,
        Inject(publisher): Inject<Publisher<MarkedTopics>>,
    ) -> Result<(), CodecError> {
        publisher
            .publish_to::<1>(MarkedTopics::Marked(RoomMsg { text: msg.text }))
            .await
    }
}

#[tokio::test]
async fn a_custom_codec_is_honored_on_both_ends_of_the_send_path() {
    let app = app! {
        name: "stomp-codec-test",
        protocol: overseerd::axum::AxumPlugin,
    }
    .register_ws::<Stomp>("/stomp")
    .build()
    .await
    .expect("app builds");

    let listener = TcpListener::bind(("127.0.0.1", 0)).await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    let shutdown = app.shutdown_handle();
    let server = tokio::spawn(async move { app.serve(listener).await });

    let url = format!("ws://{addr}/stomp");
    let connection = StompClientTransport::connect(&url).await.expect("connects");

    let mut marked = MarkedTopicsClient::new(connection.clone())
        .subscribe_marked()
        .await
        .expect("subscribe_marked");

    // The generated `marked` SEND encodes with MarkedCodec; the server decodes with MarkedCodec,
    // re-publishes with MarkedCodec, and the subscription decodes with MarkedCodec.
    MarkedClient::new(connection.clone())
        .marked(SendChat {
            text: "via marker".into(),
        })
        .await
        .expect("marked send");

    let received = tokio::time::timeout(std::time::Duration::from_secs(5), marked.next())
        .await
        .expect("a broadcast arrives before timeout")
        .expect("the subscription stream is live")
        .expect("a decoded RoomMsg — proving MarkedCodec round-tripped on every hop");

    assert_eq!(received.text, "via marker");

    shutdown.shutdown();
    let _ = server.await;
}
