//! End-to-end test of a **WebSocket controller**: build the app, serve it on an ephemeral port,
//! connect a real ws client (`tokio-tungstenite`), and exercise both a plain `#[message]` handler
//! and one that mixes the JSON payload with route-level `Inject` DI — proving ws handlers get the
//! same request-scoped dependency injection as REST routes. The server is shut down at the end so
//! the test never hangs.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use futures::{SinkExt, StreamExt};
use overseerd::axum::client::{TokioTungsteniteWs, WebsocketClient};
use overseerd::axum::prelude::*;
use overseerd::client::ClientError;
use overseerd::prelude::*;
use overseerd::{component, methods};
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::Message;

/// A shared greeting backend (singleton), field-injected into the ws controller.
#[component(by_value)]
#[derive(Clone)]
struct Greeter {
    #[default]
    greetings: Arc<AtomicU64>,
}

impl Greeter {
    fn greet(&self, who: &str) -> (String, u64) {
        let count = self.greetings.fetch_add(1, Ordering::Relaxed) + 1;

        (format!("Hello, {who}!"), count)
    }
}

/// A per-request component — for ws, "request" means one inbound message — resolved through DI.
#[component(scope = Request)]
struct RequestTicket {
    #[default]
    id: u64,
}

#[methods]
impl RequestTicket {
    #[init]
    async fn init() -> Self {
        Self { id: 4242 }
    }
}

#[dto]
struct Who {
    who: String,
}

#[dto]
struct Greeting {
    message: String,
    count: u64,
}

#[dto]
struct Ticketed {
    message: String,
    ticket: u64,
}

#[controller(ws = JsonWs)]
struct Sock {
    greeter: Greeter,
}

#[handlers(ws = JsonWs)]
impl Sock {
    #[message("greet")]
    async fn greet(&self, msg: Who) -> Greeting {
        let (message, count) = self.greeter.greet(&msg.who);

        Greeting { message, count }
    }

    /// Mixes the JSON payload with an injected, request-scoped `RequestTicket`.
    #[message("ticket")]
    async fn ticket(&self, msg: Who, Inject(ticket): Inject<Arc<RequestTicket>>) -> Ticketed {
        let (message, _) = self.greeter.greet(&msg.who);

        Ticketed {
            message,
            ticket: ticket.id,
        }
    }
}

#[tokio::test]
async fn ws_controller_dispatches_and_injects() {
    let app = app! {
        name: "ws-test",
        protocol: overseerd::axum::AxumPlugin,
    }
    .register_ws::<JsonWs>("/ws")
    .build()
    .await
    .expect("app builds");

    let listener = TcpListener::bind(("127.0.0.1", 0)).await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    let shutdown = app.shutdown_handle();
    let server = tokio::spawn(async move { app.serve(listener).await });

    let (mut socket, _response) = tokio_tungstenite::connect_async(format!("ws://{addr}/ws"))
        .await
        .expect("ws connect");

    // Plain `#[message]` handler: payload in, JSON reply out, id echoed.
    socket
        .send(Message::Text(
            r#"{"dest":"greet","id":1,"payload":{"who":"world"}}"#.into(),
        ))
        .await
        .expect("send greet");

    let reply = next_json(&mut socket).await;
    assert_eq!(reply["dest"], "greet");
    assert_eq!(reply["id"], 1);
    assert_eq!(reply["ok"]["message"], "Hello, world!");
    assert_eq!(reply["ok"]["count"], 1);

    // DI handler: the injected request-scoped ticket is resolved per message.
    socket
        .send(Message::Text(
            r#"{"dest":"ticket","id":2,"payload":{"who":"di"}}"#.into(),
        ))
        .await
        .expect("send ticket");

    let reply = next_json(&mut socket).await;
    assert_eq!(reply["dest"], "ticket");
    assert_eq!(reply["id"], 2);
    assert_eq!(reply["ok"]["message"], "Hello, di!");
    assert_eq!(reply["ok"]["ticket"], 4242);

    // Unknown destination → an error frame correlating the id.
    socket
        .send(Message::Text(
            r#"{"dest":"nope","id":3,"payload":{}}"#.into(),
        ))
        .await
        .expect("send nope");

    let reply = next_json(&mut socket).await;
    assert_eq!(reply["dest"], "nope");
    assert_eq!(reply["id"], 3);
    assert_eq!(reply["error"], "no handler for destination");

    let ws = TokioTungsteniteWs::<JsonWs>::connect(format!("ws://{addr}/ws"))
        .await
        .expect("typed ws connect");
    let typed = SockClient::new(ws.clone());

    let reply = typed
        .greet(Who {
            who: "typed".into(),
        })
        .await
        .expect("typed greet");
    assert_eq!(reply.message, "Hello, typed!");
    assert_eq!(reply.count, 3);

    let reply = typed
        .ticket(Who { who: "ws".into() })
        .await
        .expect("typed ticket");
    assert_eq!(reply.message, "Hello, ws!");
    assert_eq!(reply.ticket, 4242);

    let error = WebsocketClient::<JsonWs, (), serde_json::Value>::websocket_call(&ws, "nope", ())
        .await
        .expect_err("unknown destination is remote error");
    match error {
        ClientError::Remote(body) => {
            assert_eq!(body.code(), overseerd::axum::JsonWsStatus::Error);
            assert_eq!(
                String::from_utf8(body.into_raw()).unwrap(),
                "no handler for destination"
            );
        }

        other => panic!("expected remote ws error, got {other:?}"),
    }

    shutdown.shutdown();
    let _ = server.await;
}

/// Reads the next text frame off the socket and parses it as JSON.
async fn next_json<S>(socket: &mut S) -> serde_json::Value
where
    S: StreamExt<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin,
{
    loop {
        let message = socket.next().await.expect("a frame").expect("ok frame");

        if let Message::Text(text) = message {
            return serde_json::from_str(&text).expect("json reply");
        }
    }
}
