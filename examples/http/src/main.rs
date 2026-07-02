//! A small but complete Overseerd **HTTP** daemon, demonstrating the axum protocol: a
//! controller whose routes mix native axum extractors (`Path`, `Json`) with framework
//! dependency injection (`Inject`), a singleton dependency field-injected into the
//! controller, and a request-scoped component reachable only from a route.
//!
//! It also serves a **WebSocket** controller (`#[controller(ws = JsonWs)]`) on `/ws`, opted in
//! with `register_ws::<JsonWs>("/ws")`. Each `#[message("dest")]` method answers a JSON frame
//! `{ "dest": "<dest>", "id": <n>, "payload": <json> }` with `{ "dest", "id", "ok": <json> }`.
//!
//! Run it, then exercise the routes:
//!
//! ```text
//! cargo run -p overseerd-example-http
//! curl localhost:3001/greet/world
//! curl -X POST localhost:3001/greet -H 'content-type: application/json' -d '"there"'
//! curl localhost:3001/greet/world/ticket
//!
//! # WebSocket (using websocat: https://github.com/vi/websocat)
//! echo '{"dest":"greet","id":1,"payload":{"who":"world"}}' | websocat ws://localhost:3001/ws
//! # → {"dest":"greet","id":1,"ok":{"message":"Hello, world!","count":1}}
//!
//! # Middleware (see the `auth` module): a global request logger, a path-scoped auth guard,
//! # and a request-scoped component fetching a "user" once from the `Authorization` header.
//! curl localhost:3001/me/public
//! curl localhost:3001/me/whoami                          # 401, no Authorization header
//! curl localhost:3001/me/whoami -H 'authorization: Bearer alice'
//! # → {"name":"user:alice","same_instance":true}
//! ```

mod auth;
mod stomp;

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use overseerd::axum::axum::Json;
use overseerd::axum::axum::extract::Path;
use overseerd::axum::prelude::*;
use overseerd::prelude::*;
use overseerd::{component, methods};
use serde::{Deserialize, Serialize};

#[config(path = "example")]
#[derive(Serialize, Deserialize)]
struct ExampleConfig {
    #[default = "Hello from the config"]
    message: String,
}

/// A greeting backend: a singleton component shared across all requests. Field-injected into
/// the controller, proving a controller can depend on longer-or-equal-lived components.
#[component(by_value)]
#[derive(Clone)]
struct Greeter {
    #[default]
    greetings: Arc<AtomicU64>,
}

impl Greeter {
    /// Stamps and counts a greeting. The counter is shared (it lives behind the internal
    /// `Arc`), so it survives across requests.
    fn greet(&self, who: &str) -> (String, u64) {
        let count = self.greetings.fetch_add(1, Ordering::Relaxed) + 1;

        (format!("Hello, {who}!"), count)
    }
}

/// A per-request ticket: a request-scoped component, so each request gets a fresh instance.
/// A singleton controller cannot field-inject this (it outlives no request) — a route reaches
/// it through [`Inject`] instead, which is the whole point of route-level injection.
#[component(scope = Request)]
struct RequestTicket {
    #[default]
    id: u64,
}

#[methods]
impl RequestTicket {
    #[init]
    async fn init() -> Self {
        use std::time::{SystemTime, UNIX_EPOCH};

        let id = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.subsec_nanos() as u64)
            .unwrap_or(0);

        Self { id }
    }
}

/// The greeting response body, serialized to JSON by axum's `Json` responder.
#[derive(Serialize)]
struct GreetResponse {
    message: String,
    count: u64,
}

/// The greeting response stamped with the per-request ticket id.
#[derive(Serialize)]
struct TicketResponse {
    message: String,
    count: u64,
    ticket: u64,
}

/// The HTTP controller, mounted under `/greet`. A singleton holding the shared [`Greeter`].
#[controller(path = "/greet")]
struct GreetController {
    greeter: Greeter,
}

#[handlers]
impl GreetController {
    /// `GET /greet/{who}` — greets the path segment, mixing the native axum `Path` extractor
    /// with the controller's own `&self` state.
    #[get("/{who}")]
    async fn greet_path(&self, Path(who): Path<String>) -> Json<GreetResponse> {
        let (message, count) = self.greeter.greet(&who);

        Json(GreetResponse { message, count })
    }

    #[get("/")]
    async fn get_cfg(&self, Inject(cfg): Inject<Cfg<ExampleConfig>>) -> Json<String> {
        let cfg = cfg.snapshot();

        let message = cfg.message.clone();
        Json(message)
    }

    /// `POST /greet` — greets a JSON string body.
    #[post("/")]
    async fn greet_body(&self, Json(who): Json<String>) -> Json<GreetResponse> {
        let (message, count) = self.greeter.greet(&who);

        Json(GreetResponse { message, count })
    }

    /// `GET /greet/{who}/ticket` — greets, and stamps the per-request ticket resolved through
    /// route-level DI. The `Inject` extractor reaches the request scope; the controller could
    /// not hold this request-scoped component itself.
    #[get("/{who}/ticket")]
    async fn greet_ticketed(
        &self,
        Path(who): Path<String>,
        Inject(ticket): Inject<Arc<RequestTicket>>,
    ) -> Json<TicketResponse> {
        let (message, count) = self.greeter.greet(&who);

        Json(TicketResponse {
            message,
            count,
            ticket: ticket.id,
        })
    }
}

/// A WebSocket greeting request, decoded from a frame's JSON `payload`.
#[derive(Deserialize)]
struct WsGreet {
    who: String,
}

/// A WebSocket controller, speaking the JSON-envelope protocol. Like a REST controller, it is a
/// singleton holding the shared [`Greeter`]; its `#[message]` methods route on the frame's `dest`.
#[controller(ws = JsonWs)]
struct GreetSocket {
    greeter: Greeter,
}

#[handlers]
impl GreetSocket {
    /// `dest = "greet"` — greets the payload's `who`, reusing the same shared [`Greeter`] the HTTP
    /// controller uses, so the greeting count is shared across HTTP and WebSocket callers.
    #[message("greet")]
    async fn greet(&self, msg: WsGreet) -> GreetResponse {
        let (message, count) = self.greeter.greet(&msg.who);

        GreetResponse { message, count }
    }

    /// `dest = "greet_ticketed"` — like the HTTP `/greet/{who}/ticket` route, this mixes the JSON
    /// payload with route-level **dependency injection**: `Inject` resolves a fresh per-message
    /// [`RequestTicket`] from the message's request scope (parented at the socket's connection
    /// scope), proving ws handlers get the same request-scoped DI as REST handlers.
    #[message("greet_ticketed")]
    async fn greet_ticketed(
        &self,
        msg: WsGreet,
        Inject(ticket): Inject<Arc<RequestTicket>>,
    ) -> TicketResponse {
        let (message, count) = self.greeter.greet(&msg.who);

        TicketResponse {
            message,
            count,
            ticket: ticket.id,
        }
    }
}

#[tokio::main]
async fn main() -> overseerd::axum::Result<()> {
    overseerd::builtins::init_tracing(&Default::default()).ok();

    // No `controllers:` listing: a `#[controller]` registers itself (its DI component and its
    // route table) into link-time slices that `auto_discover` folds in, and its dependency
    // graph is checked at its own definition. `app!` only needs the protocol.
    //
    // WebSockets are opt-in: `register_ws::<JsonWs>("/ws")` activates the JSON ws protocol and
    // mounts its upgrade handler at `/ws`, serving every `#[controller(ws = JsonWs)]` controller.
    //
    // `.layer(..)` takes a raw `tower`/axum layer directly — standard middleware needs no
    // wrapping to keep working alongside the DI-backed `AxumMiddleware` kind (see `auth.rs`).
    let app = app! {
        name: "example-http",
        protocol: AxumPlugin,
    }
    .layer(overseerd::axum::axum::middleware::from_fn(
        auth::log_requests,
    ))
    .register_ws::<JsonWs>("/ws")
    .register_ws::<Stomp>("/ws/stomp")
    .build()
    .await?;

    println!("{app}");

    let addr = SocketAddr::from(([127, 0, 0, 1], 3001));
    println!("listening on http://{addr}");

    app.serve(addr).await
}
