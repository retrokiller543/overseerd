//! The protocol-generic pub/sub runtime: the neutral subscription registry, the shared topic bus,
//! and the injected [`Publisher`].
//!
//! These types carry no STOMP knowledge — they are generic over a
//! [`PubSubProtocol`](crate::ws::PubSubProtocol), so a non-STOMP WebSocket protocol reuses them by
//! implementing that trait (plus [`MessagingProtocol`](crate::messaging::MessagingProtocol) for the
//! wire body). They compile behind `feature = "ws"` (not `stomp`). STOMP's concrete framing
//! (`OutFrame`, the `MESSAGE` builder) and its DI bus descriptor live in [`crate::ws::stomp`].

mod publisher;
mod registry;
#[cfg(test)]
mod tests;
mod topic_bus;

pub use publisher::Publisher;
pub use registry::{ConnectionId, SubscriptionRegistry};
pub use topic_bus::{DEFAULT_PUBLISH_FANOUT, TopicBus, register_topic_bus, topic_bus_descriptor};
