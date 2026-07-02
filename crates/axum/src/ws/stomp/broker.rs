//! The subscription registry and `MESSAGE` fan-out.
//!
//! One [`Broker`] is shared (via `Arc`) across every connection served by a [`Stomp`](super::Stomp)
//! endpoint. Each connection registers an outbound channel; a `SUBSCRIBE` records interest in a
//! destination; a `SEND` to a `/topic/**` destination (or an app handler's `Publisher`) fans a
//! `MESSAGE` out to every matching subscriber. This is the server-push inversion: the broker holds
//! each connection's write-half sender, so a message can reach a socket with no request in flight.

use std::collections::HashMap;
use std::sync::RwLock;
use std::sync::atomic::{AtomicU64, Ordering};

use stomp_parser::server::MessageFrameBuilder;
use tokio::sync::mpsc;

use super::body::StompBody;

/// Identifies one live connection on a broker. Minted by [`Broker::register`].
pub type ConnectionId = u64;

/// A frame queued to a connection's writer task. `Heartbeat` is an empty server heart-beat
/// (a bare newline); `Frame` is a fully serialized STOMP frame.
pub enum OutFrame {
    /// A serialized STOMP frame to write verbatim.
    Frame(Vec<u8>),

    /// A server heart-beat (`\n`), emitted when the connection is otherwise idle.
    Heartbeat,
}

/// One live subscription: the connection, its client-chosen id, and that connection's writer sink.
/// Holding the `tx` here lets [`publish`](Broker::publish) fan out without touching a second lock.
struct SubEntry {
    conn: ConnectionId,
    sub_id: String,
    tx: mpsc::Sender<OutFrame>,
}

/// The subscription registry: destination → subscribers, plus id counters. Publishing takes a read
/// lock only long enough to clone the matching senders, then releases it before sending — no
/// `await` is ever held across the lock (see [`publish`](Broker::publish)).
pub struct Broker {
    subs: RwLock<HashMap<String, Vec<SubEntry>>>,
    next_conn: AtomicU64,
    next_message: AtomicU64,
}

impl Broker {
    /// A fresh, empty broker.
    pub fn new() -> Self {
        Self {
            subs: RwLock::new(HashMap::new()),
            next_conn: AtomicU64::new(1),
            next_message: AtomicU64::new(1),
        }
    }

    /// Mints a new connection id. Each served socket calls this once.
    pub fn register(&self) -> ConnectionId {
        self.next_conn.fetch_add(1, Ordering::Relaxed)
    }

    /// Records a subscription: `conn`'s `sub_id` now receives messages published to `destination`.
    pub fn subscribe(
        &self,
        conn: ConnectionId,
        sub_id: &str,
        destination: &str,
        tx: mpsc::Sender<OutFrame>,
    ) {
        let mut subs = self.subs.write().expect("broker subs lock poisoned");

        subs.entry(destination.to_owned()).or_default().push(SubEntry {
            conn,
            sub_id: sub_id.to_owned(),
            tx,
        });
    }

    /// Removes one subscription by `(conn, sub_id)`. The destination is not needed — a client
    /// `UNSUBSCRIBE` carries only the id.
    pub fn unsubscribe(&self, conn: ConnectionId, sub_id: &str) {
        let mut subs = self.subs.write().expect("broker subs lock poisoned");

        for entries in subs.values_mut() {
            entries.retain(|entry| !(entry.conn == conn && entry.sub_id == sub_id));
        }
    }

    /// Drops every subscription belonging to `conn` (called when its socket closes).
    pub fn unregister(&self, conn: ConnectionId) {
        let mut subs = self.subs.write().expect("broker subs lock poisoned");

        for entries in subs.values_mut() {
            entries.retain(|entry| entry.conn != conn);
        }

        subs.retain(|_, entries| !entries.is_empty());
    }

    /// Fans `body` out to every subscriber of `destination` as a `MESSAGE` frame. Collects the
    /// matching senders under a read lock, releases it, then sends — a slow/backpressured consumer
    /// never blocks the registry. Dead connections are cleaned up on their own `unregister`.
    pub fn publish(&self, destination: &str, body: &StompBody, extra_headers: &[(String, String)]) {
        let targets: Vec<(String, mpsc::Sender<OutFrame>)> = {
            let subs = self.subs.read().expect("broker subs lock poisoned");

            match subs.get(destination) {
                Some(entries) => entries
                    .iter()
                    .map(|entry| (entry.sub_id.clone(), entry.tx.clone()))
                    .collect(),

                None => Vec::new(),
            }
        };

        for (sub_id, tx) in targets {
            let frame = self.build_message(&sub_id, destination, body, extra_headers);

            // A full or closed channel means a slow/gone consumer; drop the message for it rather
            // than block the publisher. `try_send` never awaits, so no lock concern here.
            let _ = tx.try_send(OutFrame::Frame(frame));
        }
    }

    /// Serializes one `MESSAGE` frame for `sub_id` carrying `body`.
    fn build_message(
        &self,
        sub_id: &str,
        destination: &str,
        body: &StompBody,
        extra_headers: &[(String, String)],
    ) -> Vec<u8> {
        let message_id = self.next_message.fetch_add(1, Ordering::Relaxed);

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

        builder.body(body.bytes.to_vec()).build().into()
    }
}

impl Default for Broker {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body(text: &str) -> StompBody {
        StompBody::json(text.as_bytes().to_vec())
    }

    #[tokio::test]
    async fn publish_reaches_only_matching_subscribers() {
        let broker = Broker::new();
        let conn_a = broker.register();
        let conn_b = broker.register();
        let (tx_a, mut rx_a) = mpsc::channel(4);
        let (tx_b, mut rx_b) = mpsc::channel(4);

        broker.subscribe(conn_a, "sub-1", "/topic/room", tx_a);
        broker.subscribe(conn_b, "sub-2", "/topic/other", tx_b);
        broker.publish("/topic/room", &body("hi"), &[]);

        let got = rx_a.try_recv();
        assert!(matches!(got, Ok(OutFrame::Frame(_))), "subscriber A gets the message");
        assert!(rx_b.try_recv().is_err(), "subscriber B on another topic gets nothing");
    }

    #[tokio::test]
    async fn unsubscribe_and_unregister_stop_delivery() {
        let broker = Broker::new();
        let conn = broker.register();
        let (tx, mut rx) = mpsc::channel(4);

        broker.subscribe(conn, "sub-1", "/topic/room", tx);
        broker.unsubscribe(conn, "sub-1");
        broker.publish("/topic/room", &body("hi"), &[]);

        assert!(rx.try_recv().is_err(), "an unsubscribed connection receives nothing");

        let (tx2, mut rx2) = mpsc::channel(4);
        broker.subscribe(conn, "sub-2", "/topic/room", tx2);
        broker.unregister(conn);
        broker.publish("/topic/room", &body("hi"), &[]);

        assert!(rx2.try_recv().is_err(), "an unregistered connection receives nothing");
    }
}
