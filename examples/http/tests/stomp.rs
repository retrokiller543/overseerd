//! End-to-end test of a **STOMP pub/sub controller**: build the app, serve it on an ephemeral
//! port, then drive it with the *generated typed clients* — no destination strings at the call
//! sites. One client subscribes to a topic via `ChatTopicsClient::subscribe_room()`; another sends
//! to an app destination via `ChatControllerClient::chat(..)`, whose handler publishes to the
//! topic; the subscriber's typed `Subscription` stream then yields the broadcast message. The
//! server is shut down at the end so the test never hangs.

use futures::StreamExt;
use overseerd::axum::CodecError;
use overseerd::axum::client::StompClientTransport;
use overseerd::axum::prelude::*;
use overseerd::prelude::*;
use serde::{Deserialize, Serialize};
use tokio::net::TcpListener;

/// A message a client sends to the app (`/app/chat`).
#[derive(Serialize, Deserialize)]
struct SendChat {
    text: String,
}

/// A message broadcast to subscribers of `/topic/room`.
#[derive(Clone, Serialize, Deserialize)]
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
            .publish(ChatTopics::Room(RoomMsg { text: msg.text }))
            .await
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
