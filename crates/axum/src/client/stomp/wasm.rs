//! The wasm/JS bridge for STOMP subscriptions.
//!
//! A [`Subscription`](super::Subscription) is a Rust `Stream` — JavaScript can't poll it. This
//! bridges it to the idiomatic JS shape: `subscribe_*` hands back a [`StompSubscription`] handle and
//! delivers each decoded message to a callback. The generated method takes a **typed** callback
//! (`(message: T) => void`, via a per-method `typescript_type` extern), so JS/TS sees the real
//! message type, not `any`; at runtime it is a plain `js_sys::Function` here. `handle.unsubscribe()`
//! (or GC'ing the handle) aborts the pump task, which drops the `Subscription` and sends
//! `UNSUBSCRIBE`.

use futures::StreamExt;
use futures::future::{AbortHandle, Abortable};
use serde::Serialize;
use wasm_bindgen::prelude::*;

use super::{StompSubscribe, Subscription};

/// A live STOMP subscription handle returned to JS. Call [`unsubscribe`](Self::unsubscribe) to stop
/// receiving (it aborts the delivery task, which sends `UNSUBSCRIBE`). Dropping the handle in JS
/// (letting it be garbage-collected) does the same.
#[wasm_bindgen]
pub struct StompSubscription {
    abort: AbortHandle,
}

#[wasm_bindgen]
impl StompSubscription {
    /// Stops the subscription: aborts message delivery and sends `UNSUBSCRIBE` to the broker.
    pub fn unsubscribe(&self) {
        self.abort.abort();
    }
}

impl Drop for StompSubscription {
    fn drop(&mut self) {
        self.abort.abort();
    }
}

/// Spawns the task pumping `subscription`'s decoded messages into the JS `on_message` callback, and
/// returns the [`StompSubscription`] handle controlling it. Each message is serialized to its JS
/// value; a decode/transport error ends the stream (logged), matching the native stream's behavior.
/// Generated `subscribe_*` wasm methods call this (having converted their typed callback param to a
/// `js_sys::Function`).
pub fn pump<C, M>(
    subscription: Subscription<C, M>,
    on_message: js_sys::Function,
) -> StompSubscription
where
    C: StompSubscribe + Clone + Unpin + 'static,
    M: Serialize + Unpin + 'static,
{
    let (abort, registration) = AbortHandle::new_pair();

    // The pump owns the `Subscription`; aborting drops it → its `Drop` sends `UNSUBSCRIBE`.
    let task = Abortable::new(
        async move {
            let mut subscription = subscription;

            while let Some(item) = subscription.next().await {
                match item {
                    Ok(message) => match serde_wasm_bindgen::to_value(&message) {
                        // Deliver the typed message to JS. A serialization failure is logged and
                        // skipped rather than tearing the whole subscription down.
                        Ok(value) => {
                            let _ = on_message.call1(&JsValue::NULL, &value);
                        }

                        Err(error) => {
                            tracing::warn!(
                                target: "overseerd::axum",
                                %error,
                                "STOMP message failed to serialize for JS; skipping"
                            );
                        }
                    },

                    Err(error) => {
                        tracing::warn!(
                            target: "overseerd::axum",
                            %error,
                            "STOMP subscription error; ending subscription"
                        );

                        break;
                    }
                }
            }
        },
        registration,
    );

    crate::client::ws_rt::spawn(async move {
        // Resolves on stream end or on abort; either way the subscription is finished.
        let _ = task.await;
    });

    StompSubscription { abort }
}
