use std::time::Duration;

use futures::StreamExt;
use overseerd::axum::StompBody;
use overseerd::axum::client::{MessageRequest, StompClientTransport, StompConnectOptions};
use overseerd::prelude::*;
use tokio::net::TcpListener;

use super::*;

#[tokio::test]
async fn chat_message_is_recorded_and_broadcast() {
    let app = app! {
        name: "chat-test",
        protocol: overseerd::axum::AxumPlugin,
    }
    .register_ws::<Stomp>("/ws/stomp")
    .build()
    .await
    .expect("app builds");

    let listener = TcpListener::bind(("127.0.0.1", 0)).await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    let shutdown = app.shutdown_handle();
    let server = tokio::spawn(async move { app.serve(listener).await });

    let url = format!("ws://{addr}/ws/stomp");

    // One connection, two typed facades: subscribe to the chat topic, then send to /app/chat.
    let connection = StompClientTransport::connect(&url).await.expect("connects");

    let mut chat = ChatTopicClient::new(connection.clone())
        .subscribe_chat()
        .await
        .expect("subscribe_chat");

    ChatHandlerClient::new(connection.clone())
        .on_chat(ChatMessage {
            room: "general".into(),
            sender: "alice".into(),
            text: "hello, room".into(),
        })
        .await
        .expect("send chat");

    let received = tokio::time::timeout(Duration::from_secs(5), chat.next())
        .await
        .expect("a broadcast before timeout")
        .expect("the stream is live")
        .expect("a decoded ChatMessage");

    assert_eq!(received.room, "general");
    assert_eq!(received.sender, "alice");
    assert_eq!(received.text, "hello, room");

    shutdown.shutdown();
    let _ = server.await;
}

#[tokio::test]
async fn a_templated_room_subscription_gets_only_its_room() {
    let app = app! {
        name: "chat-room-test",
        protocol: overseerd::axum::AxumPlugin,
    }
    .register_ws::<Stomp>("/ws/stomp")
    .build()
    .await
    .expect("app builds");

    let listener = TcpListener::bind(("127.0.0.1", 0)).await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    let shutdown = app.shutdown_handle();
    let server = tokio::spawn(async move { app.serve(listener).await });

    let url = format!("ws://{addr}/ws/stomp");
    let connection = StompClientTransport::connect(&url).await.expect("connects");

    // The templated subscribe takes the room as a typed argument; it resolves to
    // `/topic/room/general`, so only messages for that room arrive.
    let mut general = ChatTopicClient::new(connection.clone())
        .subscribe_room("general".into())
        .await
        .expect("subscribe_room");

    let sender = ChatHandlerClient::new(connection.clone());

    // A message to another room must NOT reach the `general` subscription.
    sender
        .on_chat(ChatMessage {
            room: "random".into(),
            sender: "bob".into(),
            text: "elsewhere".into(),
        })
        .await
        .expect("send to random");

    sender
        .on_chat(ChatMessage {
            room: "general".into(),
            sender: "alice".into(),
            text: "for general".into(),
        })
        .await
        .expect("send to general");

    let received = tokio::time::timeout(Duration::from_secs(5), general.next())
        .await
        .expect("a broadcast before timeout")
        .expect("the stream is live")
        .expect("a decoded ChatMessage");

    // The first message the `general` stream yields is the general one — the random-room message
    // went to `/topic/room/random`, a different destination this subscription never saw.
    assert_eq!(received.room, "general");
    assert_eq!(received.text, "for general");

    shutdown.shutdown();
    let _ = server.await;
}

#[tokio::test]
async fn a_request_message_awaits_a_correlated_reply() {
    let app = app! {
        name: "chat-request-test",
        protocol: overseerd::axum::AxumPlugin,
    }
    .register_ws::<Stomp>("/ws/stomp")
    .build()
    .await
    .expect("app builds");

    let listener = TcpListener::bind(("127.0.0.1", 0)).await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    let shutdown = app.shutdown_handle();
    let server = tokio::spawn(async move { app.serve(listener).await });

    let url = format!("ws://{addr}/ws/stomp");
    let connection = StompClientTransport::connect(&url).await.expect("connects");

    // `count` returns a value, so the generated client method is a request that awaits its
    // correlated reply — no subscription involved. Two sends to the same room reply 1, then 2.
    let client = ChatHandlerClient::new(connection.clone());

    let first = client
        .count(ChatMessage {
            room: "general".into(),
            sender: "alice".into(),
            text: "one".into(),
        })
        .await
        .expect("first count reply");
    let second = client
        .count(ChatMessage {
            room: "general".into(),
            sender: "bob".into(),
            text: "two".into(),
        })
        .await
        .expect("second count reply");

    assert_eq!(first.room, "general");
    assert_eq!(first.count, 1);
    assert_eq!(second.count, 2);

    shutdown.shutdown();
    let _ = server.await;
}

#[tokio::test]
async fn a_failing_request_message_resolves_err_not_hang() {
    let app = app! {
        name: "chat-reject-test",
        protocol: overseerd::axum::AxumPlugin,
    }
    .register_ws::<Stomp>("/ws/stomp")
    .build()
    .await
    .expect("app builds");

    let listener = TcpListener::bind(("127.0.0.1", 0)).await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    let shutdown = app.shutdown_handle();
    let server = tokio::spawn(async move { app.serve(listener).await });

    let url = format!("ws://{addr}/ws/stomp");
    let connection = StompClientTransport::connect(&url).await.expect("connects");

    let client = ChatHandlerClient::new(connection.clone());

    // `reject` always returns `Err`. The server must route a directed error reply back so this
    // resolves `Err` — the `timeout` turns a regression (the old hang-forever bug) into a fast
    // failure rather than a stuck test.
    let outcome = tokio::time::timeout(
        Duration::from_secs(5),
        client.reject(ChatMessage {
            room: "general".into(),
            sender: "alice".into(),
            text: "please fail".into(),
        }),
    )
    .await
    .expect("the request resolves before the timeout (it must not hang)");

    assert!(
        outcome.is_err(),
        "a failing request handler must resolve Err, but the call returned Ok"
    );

    // The connection stays usable after a handler error (non-fatal): a subsequent request
    // succeeds on the same socket.
    let ok = tokio::time::timeout(
        Duration::from_secs(5),
        client.count(ChatMessage {
            room: "general".into(),
            sender: "bob".into(),
            text: "still alive".into(),
        }),
    )
    .await
    .expect("a follow-up request resolves before the timeout")
    .expect("the follow-up request succeeds on the still-open connection");

    assert_eq!(ok.room, "general");

    shutdown.shutdown();
    let _ = server.await;
}

#[tokio::test]
async fn a_request_without_a_reply_times_out() {
    let app = app! {
        name: "chat-timeout-test",
        protocol: overseerd::axum::AxumPlugin,
    }
    .register_ws::<Stomp>("/ws/stomp")
    .build()
    .await
    .expect("app builds");

    let listener = TcpListener::bind(("127.0.0.1", 0)).await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    let shutdown = app.shutdown_handle();
    let server = tokio::spawn(async move { app.serve(listener).await });

    let url = format!("ws://{addr}/ws/stomp");
    let connection = StompClientTransport::connect_with_options(
        &url,
        StompConnectOptions::default().with_request_timeout(Some(Duration::from_millis(300))),
    )
    .await
    .expect("connects");

    // A request to a destination with no handler is never replied to. With the connection healthy,
    // the call must give up after the configured timeout rather than hang until the socket closes;
    // the outer guard turns a regression into a fast failure instead of a stuck test.
    let result = tokio::time::timeout(
        Duration::from_secs(5),
        connection.request("/app/no-such-handler", StompBody::json(b"{}".to_vec())),
    )
    .await
    .expect("the request resolves before the outer guard (it must not hang)");

    let error = result.expect_err("a request with no reply must resolve Err");

    assert!(
        error.to_string().contains("timed out"),
        "expected a timeout error, got: {error}"
    );

    shutdown.shutdown();
    let _ = server.await;
}
