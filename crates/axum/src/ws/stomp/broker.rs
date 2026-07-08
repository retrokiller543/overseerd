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

use futures::StreamExt;
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

        subs.entry(destination.to_owned())
            .or_default()
            .push(SubEntry {
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

    /// Fans `body` out with **backpressure**, up to `N` subscribers concurrently. Like
    /// [`publish`](Self::publish), but awaits room in each subscriber's buffer (`Sender::send`)
    /// instead of dropping the frame when a queue is full (`try_send`). When it returns, the
    /// `MESSAGE` is committed to every still-live subscriber's outbound queue — the delivery
    /// guarantee the fire-and-forget path deliberately forgoes.
    ///
    /// `N` is the fan-out concurrency the caller picks: the sends run as a `buffer_unordered(N)`
    /// stream, so at most `N` subscriber buffers are awaited at once (`N = 1` is fully sequential).
    /// A slow consumer then only stalls its own lane, not the whole fan-out. A subscriber whose
    /// channel has already closed (its socket is gone) resolves to an `Err` and is skipped; it is
    /// removed on its own `unregister`. The senders are cloned out under the read lock, which is
    /// released *before* any `await`, so no lock is held across the backpressure wait.
    pub async fn deliver<const N: usize>(
        &self,
        destination: &str,
        body: &StompBody,
        extra_headers: &[(String, String)],
    ) {
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

        // `N == 0` would make `buffer_unordered` poll nothing and stall forever; clamp it to a
        // sequential fan-out so a mistaken `deliver::<0>` degrades to `deliver::<1>` rather than hang.
        let concurrency = N.max(1);

        futures::stream::iter(targets.into_iter().map(|(sub_id, tx)| {
            let frame = self.build_message(&sub_id, destination, body, extra_headers);

            // Awaits capacity rather than dropping; an `Err` means the consumer's channel closed
            // (its socket is gone), which its own `unregister` cleans up — nothing to do here.
            async move {
                let _ = tx.send(OutFrame::Frame(frame)).await;
            }
        }))
        .buffer_unordered(concurrency)
        .for_each(|()| async {})
        .await;
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
mod tests;
