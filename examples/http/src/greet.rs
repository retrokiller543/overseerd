//! The plain-HTTP greeting controller and its data — the part of the app that compiles for **both**
//! the native server and a wasm browser client. On wasm only the generated `GreetControllerClient`
//! survives (the server halves are gated out by the framework macros).
//!
//! Response DTOs derive `Serialize` (the server encodes) **and** `Deserialize` (the client decodes)
//! — the one thing a type needs to travel in both directions.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use overseerd::axum::dto;
use overseerd::axum::prelude::*;
use overseerd::prelude::*;
use serde::{Deserialize, Serialize};

/// Example configuration bound from `application.toml`'s `example` table (server-side).
#[config(path = "example")]
#[derive(Serialize, Deserialize)]
pub struct ExampleConfig {
    #[default = "Hello from the config"]
    pub message: String,
}

/// A greeting backend: a singleton component shared across all requests, field-injected into the
/// controller.
#[component(by_value)]
#[derive(Clone)]
pub struct Greeter {
    #[default]
    greetings: Arc<AtomicU64>,
}

impl Greeter {
    /// Stamps and counts a greeting. The counter is shared (behind the internal `Arc`), so it
    /// survives across requests.
    pub fn greet(&self, who: &str) -> (String, u64) {
        let count = self.greetings.fetch_add(1, Ordering::Relaxed) + 1;

        (format!("Hello, {who}!"), count)
    }
}

/// A per-request ticket: a request-scoped component, so each request gets a fresh instance.
#[component(scope = Request)]
pub struct RequestTicket {
    #[default]
    pub id: u64,
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

/// The greeting response body. `Serialize` for the server to encode, `Deserialize` for the client
/// to decode — the same type crosses the wire in both directions.
//#[dto]
#[dto]
pub struct GreetResponse {
    pub message: String,
    pub count: u64,
}

/// The greeting response stamped with the per-request ticket id.
#[dto]
pub struct TicketResponse {
    pub message: String,
    pub count: u64,
    pub ticket: u64,
}

/// The HTTP controller, mounted under `/greet`. A singleton holding the shared [`Greeter`].
#[controller(path = "/greet")]
pub struct GreetController {
    greeter: Greeter,
}

#[handlers]
impl GreetController {
    /// `GET /greet/{who}` — greets the path segment.
    #[get("/{who}")]
    async fn greet_path(&self, Path(who): Path<String>) -> Json<GreetResponse> {
        let (message, count) = self.greeter.greet(&who);

        Json(GreetResponse { message, count })
    }

    /// `GET /greet` — returns the configured message.
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

    /// `GET /greet/{who}/ticket` — greets and stamps the per-request ticket resolved through
    /// route-level DI.
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
