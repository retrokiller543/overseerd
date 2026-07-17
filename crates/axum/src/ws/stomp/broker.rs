//! STOMP's instantiation of the neutral [`SubscriptionRegistry`] and its `MESSAGE` framing.
//!
//! [`Broker`] is `SubscriptionRegistry<OutFrame>` with a `publish`/`deliver` surface that frames a
//! STOMP `MESSAGE` from a [`StompBody`]. The protocol-neutral registry itself lives in
//! [`crate::ws::pubsub`]; this module supplies only STOMP's outbound frame type and how a body
//! becomes a serialized `MESSAGE`.

use stomp_parser::server::MessageFrameBuilder;

use crate::ws::pubsub::SubscriptionRegistry;

use super::body::StompBody;

/// A frame queued to a connection's writer task. `Heartbeat` is an empty server heart-beat
/// (a bare newline), `Ping` probes an idle websocket peer, and `Frame` is a fully serialized STOMP
/// frame.
pub enum OutFrame {
    /// A serialized STOMP frame to write verbatim.
    Frame(Vec<u8>),

    /// A server heart-beat (`\n`), emitted when the connection is otherwise idle.
    Heartbeat,

    /// A WebSocket ping used by the framework-wide idle timeout.
    Ping,
}

/// STOMP's subscription registry: fans a `MESSAGE` frame out from a [`StompBody`].
pub type Broker = SubscriptionRegistry<OutFrame>;

/// Serializes one STOMP `MESSAGE` frame for `sub_id` carrying `body`, tagged with `message_id`. The
/// framer STOMP hands the registry (and [`PubSubProtocol::frame_message`](crate::ws::PubSubProtocol)).
pub(crate) fn build_message(
    message_id: u64,
    destination: &str,
    sub_id: &str,
    body: &StompBody,
    extra_headers: &[(String, String)],
) -> OutFrame {
    let mut builder = MessageFrameBuilder::new(
        message_id.to_string(),
        destination.to_owned(),
        sub_id.to_owned(),
    )
    .content_length(body.bytes.len() as u32);

    if let Some(content_type) = &body.content_type {
        builder = builder.content_type(content_type.clone());
    }

    for (name, value) in extra_headers {
        builder = builder.add_custom_header(name.clone(), value.clone());
    }

    OutFrame::Frame(builder.body(body.bytes.to_vec()).build().into())
}

impl SubscriptionRegistry<OutFrame> {
    /// Fans `body` out to every subscriber of `destination` as a `MESSAGE` frame (fire-and-forget).
    pub fn publish(&self, destination: &str, body: &StompBody, extra_headers: &[(String, String)]) {
        self.publish_frames(destination, |sub_id, message_id| {
            build_message(message_id, destination, sub_id, body, extra_headers)
        });
    }

    /// Fans `body` out with **backpressure**, up to `N` subscribers concurrently.
    pub async fn deliver<const N: usize>(
        &self,
        destination: &str,
        body: &StompBody,
        extra_headers: &[(String, String)],
    ) {
        self.deliver_frames::<N>(destination, |sub_id, message_id| {
            build_message(message_id, destination, sub_id, body, extra_headers)
        })
        .await;
    }
}

#[cfg(test)]
mod tests;
