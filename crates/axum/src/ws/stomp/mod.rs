//! STOMP 1.2 pub/sub over WebSocket.
//!
//! [`Stomp`] is a [`WebsocketProtocol`](crate::ws::WebsocketProtocol) implementation that adds a
//! broker on top of the shared ws seam: a `SEND` to an app destination (`/app/**`) invokes a
//! `#[message]` handler, a `SUBSCRIBE` registers interest, and a `SEND` to a broker destination
//! (`/topic/**`, `/queue/**`) — or an app handler publishing through a `Publisher` — fans a
//! `MESSAGE` out to every subscriber, across connections.
//!
//! Framing is delegated to the [`stomp-parser`](https://crates.io/crates/stomp-parser) crate;
//! this module owns the broker, the connection serve loop, DI scope seeding, and the typed
//! [`Topic`]/[`Publisher`] publish surface.

mod body;
mod broker;
mod error;

pub use body::{Publish, StompBody, StompOutcome, Topic};
pub use broker::{Broker, ConnectionId};
pub use error::StompError;
