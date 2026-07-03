//! Shared client-side WebSocket runtime helpers.
//!
//! A single target-agnostic task `spawn` both client WebSocket transports drive their connection
//! actor through — STOMP today, and the JsonWs request/reply backend when it moves off the
//! native-only `tokio-tungstenite` path. Keeping it here means adding wasm support to another ws
//! transport is a one-line change (spawn the actor via this), not a per-transport `#[cfg]` dance.

use std::future::Future;

/// Spawns a detached background task driving a client WebSocket connection actor. Native uses the
/// tokio runtime (`Send` future); wasm uses `wasm_bindgen_futures::spawn_local`, which is
/// single-threaded and so accepts a `!Send` future (the browser `WebSocket` handle is `!Send`).
#[cfg(not(target_family = "wasm"))]
pub fn spawn<F>(future: F)
where
    F: Future<Output = ()> + Send + 'static,
{
    tokio::spawn(future);
}

/// See the native variant. On wasm the actor future need not be `Send`.
#[cfg(target_family = "wasm")]
pub fn spawn<F>(future: F)
where
    F: Future<Output = ()> + 'static,
{
    wasm_bindgen_futures::spawn_local(future);
}
