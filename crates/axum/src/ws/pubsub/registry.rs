//! The protocol-neutral subscription registry and `MESSAGE` fan-out.
//!
//! [`SubscriptionRegistry<F>`] is the protocol-neutral core: a destination → subscribers map plus id
//! counters, generic over the outbound frame type `F` a protocol delivers. It is shared (via `Arc`)
//! across every connection served by a pub/sub endpoint and by the [`TopicBus`](super::TopicBus) so
//! an HTTP handler and a message handler reach the same live subscriptions. A `SUBSCRIBE` records
//! interest; a publish fans a frame out to every matching subscriber — the server-push inversion:
//! the registry holds each connection's write-half sender, so a message can reach a socket with no
//! request in flight. The registry never frames a message itself; the caller supplies a *framer*
//! closure, so STOMP frames a `MESSAGE` and another protocol frames its own event.

use std::collections::HashMap;
use std::sync::RwLock;
use std::sync::atomic::{AtomicU64, Ordering};

use futures::StreamExt;
use tokio::sync::mpsc;

/// Identifies one live connection on a registry. Minted by [`SubscriptionRegistry::register`].
pub type ConnectionId = u64;

/// One live subscription: the connection, its client-chosen id, and that connection's writer sink.
/// Holding the `tx` here lets a publish fan out without touching a second lock.
struct SubEntry<F> {
    conn: ConnectionId,
    sub_id: String,
    tx: mpsc::Sender<F>,
}

/// The protocol-neutral subscription registry: destination → subscribers, plus id counters, generic
/// over the outbound frame type `F`. Publishing takes a read lock only long enough to clone the
/// matching senders, then releases it before sending — no `await` is ever held across the lock.
pub struct SubscriptionRegistry<F> {
    subs: RwLock<HashMap<String, Vec<SubEntry<F>>>>,
    next_conn: AtomicU64,
    next_message: AtomicU64,
}

impl<F> SubscriptionRegistry<F> {
    /// A fresh, empty registry.
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

    /// Mints a fresh outbound `message-id`, shared with the fan-out counter so a directed reply
    /// (routed outside the subscription tables) never collides with a broadcast frame.
    pub fn next_message_id(&self) -> u64 {
        self.next_message.fetch_add(1, Ordering::Relaxed)
    }

    /// Records a subscription: `conn`'s `sub_id` now receives frames published to `destination`.
    pub fn subscribe(
        &self,
        conn: ConnectionId,
        sub_id: &str,
        destination: &str,
        tx: mpsc::Sender<F>,
    ) {
        let mut subs = self.subs.write().expect("registry subs lock poisoned");

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
        let mut subs = self.subs.write().expect("registry subs lock poisoned");

        for entries in subs.values_mut() {
            entries.retain(|entry| !(entry.conn == conn && entry.sub_id == sub_id));
        }
    }

    /// Drops every subscription belonging to `conn` (called when its socket closes).
    pub fn unregister(&self, conn: ConnectionId) {
        let mut subs = self.subs.write().expect("registry subs lock poisoned");

        for entries in subs.values_mut() {
            entries.retain(|entry| entry.conn != conn);
        }

        subs.retain(|_, entries| !entries.is_empty());
    }

    /// Collects the `(sub_id, tx)` of every subscriber of `destination` under a read lock, releasing
    /// it before the caller sends — so a slow/backpressured consumer never blocks the registry.
    fn targets(&self, destination: &str) -> Vec<(String, mpsc::Sender<F>)>
    where
        F: Send,
    {
        let subs = self.subs.read().expect("registry subs lock poisoned");

        match subs.get(destination) {
            Some(entries) => entries
                .iter()
                .map(|entry| (entry.sub_id.clone(), entry.tx.clone()))
                .collect(),

            None => Vec::new(),
        }
    }

    /// Fire-and-forget fan-out: builds a frame per subscriber via `make_frame` (given the
    /// subscriber's id and a fresh message id) and drops it into each subscriber's buffer without
    /// awaiting. A full or closed channel means a slow/gone consumer; the frame is dropped for it
    /// rather than blocking the publisher. Cleanup happens on the consumer's own `unregister`.
    pub fn publish_frames(&self, destination: &str, make_frame: impl Fn(&str, u64) -> F)
    where
        F: Send,
    {
        let targets = self.targets(destination);

        for (sub_id, tx) in targets {
            let message_id = self.next_message.fetch_add(1, Ordering::Relaxed);
            let frame = make_frame(&sub_id, message_id);

            let _ = tx.try_send(frame);
        }
    }

    /// Fans out with **backpressure**, up to `N` subscribers concurrently. Like
    /// [`publish_frames`](Self::publish_frames) but awaits room in each subscriber's buffer
    /// (`Sender::send`) instead of dropping when a queue is full. When it returns, the frame is
    /// committed to every still-live subscriber's outbound queue. The sends run as a
    /// `buffer_unordered(N)` stream, so at most `N` buffers are awaited at once (`N = 1` is
    /// sequential); a subscriber whose channel has closed resolves to an `Err` and is skipped.
    pub async fn deliver_frames<const N: usize>(
        &self,
        destination: &str,
        make_frame: impl Fn(&str, u64) -> F,
    ) where
        F: Send,
    {
        let targets = self.targets(destination);

        // `N == 0` would make `buffer_unordered` poll nothing and stall forever; clamp it to a
        // sequential fan-out so a mistaken `deliver::<0>` degrades to `deliver::<1>` rather than hang.
        let concurrency = N.max(1);

        futures::stream::iter(targets.into_iter().map(|(sub_id, tx)| {
            let message_id = self.next_message.fetch_add(1, Ordering::Relaxed);
            let frame = make_frame(&sub_id, message_id);

            async move {
                let _ = tx.send(frame).await;
            }
        }))
        .buffer_unordered(concurrency)
        .for_each(|()| async {})
        .await;
    }
}

impl<F> Default for SubscriptionRegistry<F> {
    fn default() -> Self {
        Self::new()
    }
}
