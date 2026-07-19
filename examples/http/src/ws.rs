//! The WebSocket greeting controller (`#[controller(ws = JsonWs)]`). Server-only for now — its
//! generated client is not wasm-ready yet — so this module is gated out of wasm builds by `lib.rs`.

use std::sync::Arc;

use overseerd::axum::prelude::*;

use crate::greet::{GreetResponse, Greeter, RequestTicket, TicketResponse};

/// A WebSocket greeting request, decoded from a frame's JSON `payload`.
#[dto]
pub struct WsGreet {
    who: String,
}

/// A WebSocket controller speaking the JSON-envelope protocol. A singleton holding the shared
/// [`Greeter`]; its `#[message]` methods route on the frame's `dest`.
#[controller(ws = JsonWs)]
pub struct GreetSocket {
    greeter: Greeter,
}

#[handlers(ws = JsonWs)]
impl GreetSocket {
    /// `dest = "greet"` — greets the payload's `who`, reusing the same shared [`Greeter`] the HTTP
    /// controller uses, so the greeting count is shared across HTTP and WebSocket callers.
    #[message("greet")]
    async fn greet(&self, msg: WsGreet) -> GreetResponse {
        let (message, count) = self.greeter.greet(&msg.who);

        GreetResponse { message, count }
    }

    /// `dest = "greet_ticketed"` — mixes the JSON payload with route-level DI: `Inject` resolves a
    /// fresh per-message [`RequestTicket`] from the message's request scope.
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
