//! A functional STOMP **chat** controller: clients `SEND` a message to `/app/chat`, the handler
//! records it in the room's history and broadcasts it to every subscriber of the `/topic/chat`
//! topic. Each message carries its `room`, so one topic serves many rooms (subscribers filter by
//! room); the server keeps per-room history in [`ChatState`].
//!
//! [`ChatState`] shows the concurrency shape a chat wants: a **lock-free** room registry (reads
//! load a snapshot and clone a single `Arc`, never the map or the history), with **per-room**
//! locking so writing to one room never blocks readers or writers of another.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use arc_swap::ArcSwap;
// The axum extractors are used only by the REST handler bodies below, which the macros gate out on
// wasm; the `axum` re-export doesn't exist on wasm, so the imports are gated with their uses.
#[cfg(not(target_family = "wasm"))]
use overseerd::axum::axum::Json;
#[cfg(not(target_family = "wasm"))]
use overseerd::axum::axum::extract::Path;
use overseerd::axum::*;
use overseerd::component;

/// One chat message: which room it belongs to, who sent it, and its text.
#[dto]
#[derive(Clone)]
pub struct ChatMessage {
    pub room: String,
    pub sender: String,
    pub text: String,
}

/// The reply to a request `#[message]`: a room and its message count after recording.
#[dto]
#[derive(Clone)]
pub struct RoomCount {
    pub room: String,
    pub count: usize,
}

/// The chat broadcast topics — one static, one templated, in the same set.
///
/// `Chat` is a global firehose: `/topic/chat` carries every room's messages (a subscriber filters
/// client-side). `Room` is **templated**: its `room` field fills the `{room}` hole, so each room is
/// its own destination (`/topic/room/general`) and a subscriber gets only that room. Generates
/// `ChatTopicClient::subscribe_chat()` (no args) and `subscribe_room(room: String)` (typed arg).
#[topics(protocol = Stomp)]
pub enum ChatTopic {
    #[topic("/topic/chat")]
    Chat(ChatMessage),

    #[topic("/topic/room/{room}")]
    Room {
        room: String,
        #[content]
        message: ChatMessage,
    },
}

/// One chat room's stored history. The messages live behind this room's own lock, so appending to
/// one room never blocks readers or writers of another.
#[derive(Default)]
pub struct Room {
    messages: RwLock<Vec<ChatMessage>>,
}

impl Room {
    /// Appends a message. Locks only this room, and only for the push.
    fn append(&self, message: ChatMessage) {
        // A poisoned lock means a writer panicked mid-push, leaving the history unusable; there is
        // nothing to recover, so propagating the panic is the honest outcome.
        self.messages
            .write()
            .expect("room lock poisoned")
            .push(message);
    }

    /// Reads this room's history under a shared read lock **without copying it out**: the closure
    /// sees the messages in place, so a reader that needs only a count or a serialized view pays no
    /// clone. Concurrent readers do not block each other.
    pub fn with_messages<R>(&self, read: impl FnOnce(&[ChatMessage]) -> R) -> R {
        let guard = self.messages.read().expect("room lock poisoned");

        read(&guard)
    }

    /// A cloned snapshot of this room's history, for callers that need an owned copy.
    pub fn history(&self) -> Vec<ChatMessage> {
        self.messages.read().expect("room lock poisoned").clone()
    }
}

/// The chat application state: a lock-free registry of rooms. Looking up a room loads a snapshot
/// (no lock) and clones a single `Arc<Room>` — never the map, never the history. A new room is
/// added with a read-copy-update that clones only the registry map (the rooms themselves are shared
/// `Arc`s), and only on a room's first message. Per-room writes lock only that room.
#[component]
pub struct ChatState {
    #[default]
    rooms: ArcSwap<HashMap<String, Arc<Room>>>,
}

impl ChatState {
    /// The room named `name`, if it exists. Lock-free: a snapshot load plus one `Arc` clone.
    pub fn room(&self, name: &str) -> Option<Arc<Room>> {
        self.rooms.load().get(name).cloned()
    }

    /// The room named `name`, creating it if absent. The fast path is lock-free; only a room's
    /// first-ever message takes the read-copy-update path (which clones the registry map, not the
    /// rooms). Concurrent creation of the same room resolves to one shared [`Room`].
    pub fn room_or_create(&self, name: &str) -> Arc<Room> {
        if let Some(room) = self.rooms.load().get(name) {
            return Arc::clone(room);
        }

        let created = Arc::new(Room::default());

        self.rooms.rcu(|current| {
            if current.contains_key(name) {
                Arc::clone(current)
            } else {
                let mut next = (**current).clone();
                next.insert(name.to_owned(), Arc::clone(&created));

                Arc::new(next)
            }
        });

        self.rooms
            .load()
            .get(name)
            .cloned()
            .expect("room present immediately after its rcu insert")
    }

    /// Records a message into its room's history (creating the room on first use).
    pub fn record(&self, message: &ChatMessage) {
        self.room_or_create(&message.room).append(message.clone());
    }

    /// The names of all live rooms. Lock-free; clones only the room-name strings.
    pub fn room_names(&self) -> Vec<String> {
        self.rooms.load().keys().cloned().collect()
    }
}

/// The STOMP chat controller: a singleton whose `#[message]` handler answers inbound `SEND`s to
/// `/app/chat`, records them, and re-broadcasts them to the `/topic/chat` topic.
#[controller(ws = Stomp)]
pub struct ChatHandler {
    state: Arc<ChatState>,
}

#[handlers(ws = Stomp)]
impl ChatHandler {
    /// Handles an inbound chat message: store it in its room's history (a brief per-room lock), then
    /// broadcast it to both the global `/topic/chat` feed and the room's own `/topic/room/{room}`
    /// topic. No lock is held across an `await`.
    #[message("/app/chat")]
    async fn on_chat(
        &self,
        message: ChatMessage,
        Inject(publisher): Inject<Publisher<ChatTopic>>,
    ) -> Result<(), CodecError> {
        self.state.record(&message);

        // Awaited fan-out (`N = 1`): when this returns the message reached every live subscriber's
        // buffer, rather than being dropped for a full one. A pure fire-and-forget path would use
        // `publisher.emit(..)` instead (sync, no `await`).
        publisher
            .publish_to::<1>(ChatTopic::Chat(message.clone()))
            .await?;
        publisher
            .publish_to::<1>(ChatTopic::Room {
                room: message.room.clone(),
                message,
            })
            .await
    }

    /// A **request/response** `#[message]` on the same pub/sub controller: it returns a value (not
    /// `()`), so the generated client method awaits a correlated reply instead of being
    /// fire-and-forget — and the reply goes only to the caller, never broadcast. Here it records the
    /// message (like `on_chat`) and echoes back the room's new message count so the caller learns its
    /// own position without subscribing.
    #[message("/app/count")]
    async fn count(&self, message: ChatMessage) -> Result<RoomCount, CodecError> {
        self.state.record(&message);

        let count = self
            .state
            .room(&message.room)
            .map_or(0, |room| room.with_messages(<[ChatMessage]>::len));

        Ok(RoomCount {
            room: message.room,
            count,
        })
    }

    /// A **request** `#[message]` that always fails: it proves the error path routes back to the
    /// caller. A request handler returning `Err` sends a directed *error* reply (not a broadcast, not
    /// silence), so the client's awaiting call resolves `Err` on a live connection instead of hanging
    /// forever waiting for a reply that never comes.
    #[message("/app/reject")]
    async fn reject(&self, _message: ChatMessage) -> Result<RoomCount, CodecError> {
        Err(CodecError::internal("rejected by the handler".to_owned()))
    }
}

/// The chat history REST surface (mounted under `/chat`), sharing the same [`ChatState`] the
/// WebSocket handler writes to. Demonstrates the lock-free read side: listing rooms and reading a
/// room's history or size never blocks a concurrent `SEND`.
#[controller(path = "/chat")]
pub struct ChatHistory {
    state: Arc<ChatState>,
}

#[handlers]
impl ChatHistory {
    /// `GET /chat/rooms` — the live room names (a lock-free registry read).
    #[get("/rooms")]
    async fn rooms(&self) -> Json<Vec<String>> {
        Json(self.state.room_names())
    }

    /// `GET /chat/{room}/count` — the room's message count, read with **zero copying** (the closure
    /// runs under the room's read lock and returns only the length).
    #[get("/{room}/count")]
    async fn count(&self, Path(room): Path<String>) -> Json<usize> {
        let count = self
            .state
            .room(&room)
            .map_or(0, |room| room.with_messages(<[ChatMessage]>::len));

        Json(count)
    }

    /// `GET /chat/{room}/history` — the room's full history (an owned copy for the response body).
    #[get("/{room}/history")]
    async fn history(&self, Path(room): Path<String>) -> Json<Vec<ChatMessage>> {
        let messages = self
            .state
            .room(&room)
            .map(|room| room.history())
            .unwrap_or_default();

        Json(messages)
    }
}

#[cfg(test)]
mod tests;
