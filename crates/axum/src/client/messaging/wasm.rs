//! The wasm/JS bridge shared by both generated wasm clients: the `#[topics]` subscribe client and
//! the `#[controller(ws = ..)]` message (send/request) client.
//!
//! A [`Subscription`](super::Subscription) is a Rust `Stream` — JavaScript can't poll it. This
//! bridges it to the idiomatic JS shape: `subscribe_*` hands back a [`TopicSubscription`] handle and
//! delivers each decoded message to a callback. Both the handle and the [`pump`] are
//! protocol-agnostic; the only protocol-specific step — pulling the transport out of the shared
//! [`Connection`] — is [`TopicWasmClient`], which each protocol implements for its own tag (STOMP in
//! [`crate::client::stomp`]). Both wasm clients route through it: the topics binding to subscribe,
//! the controller binding to send/request. The generated method takes a **typed** callback
//! (`(message: T) => void`, via a per-method `typescript_type` extern), so JS/TS sees the real
//! message type, not `any`; at runtime it is a plain `js_sys::Function` here. `handle.unsubscribe()`
//! (or GC'ing the handle) aborts the pump task, which drops the `Subscription` and deregisters.

use futures::StreamExt;
use futures::future::{AbortHandle, Abortable};
use serde::Serialize;
use wasm_bindgen::prelude::*;

use crate::client::Connection;
use crate::messaging::MessagingClientProtocol;

use super::{Subscription, TopicSubscribe};

/// Pulls a protocol's client transport out of the shared browser [`Connection`]. This is the only
/// protocol-specific step in either generated wasm client (the topics subscribe binding and the
/// controller message binding); the rest (the [`TopicSubscription`] handle and the [`pump`]) is
/// protocol-agnostic. STOMP implements it over its shared socket; a new protocol implements its own,
/// and the generated bindings name neither concretely.
pub trait TopicWasmClient: MessagingClientProtocol + Sized {
    /// The concrete transport, obtained from the shared connection, that speaks this protocol. It
    /// must support every wasm client surface both bindings emit — topic subscription plus the
    /// point-to-point message send/request — so a missing capability is an error at the
    /// `impl TopicWasmClient` site rather than inside a generated method body.
    type Transport: Clone + Unpin + 'static;

    /// Pulls this protocol's transport out of the shared connection (errors if it isn't connected).
    fn transport(connection: &Connection) -> Result<Self::Transport, JsError>;
}

/// A live subscription handle returned to JS. Call [`unsubscribe`](Self::unsubscribe) to stop
/// receiving (it aborts the delivery task, which deregisters). Dropping the handle in JS (letting it
/// be garbage-collected) does the same.
#[wasm_bindgen]
pub struct TopicSubscription {
    abort: AbortHandle,
}

#[wasm_bindgen]
impl TopicSubscription {
    /// Stops the subscription: aborts message delivery and deregisters from the broker.
    pub fn unsubscribe(&self) {
        self.abort.abort();
    }
}

impl Drop for TopicSubscription {
    fn drop(&mut self) {
        self.abort.abort();
    }
}

/// Spawns the task pumping `subscription`'s decoded messages into the JS `on_message` callback, and
/// returns the [`TopicSubscription`] handle controlling it. Each message is serialized to its JS
/// value; a decode/transport error ends the stream (logged), matching the native stream's behavior.
/// Fully protocol-generic. Generated `subscribe_*` wasm methods call this (having converted their
/// typed callback param to a `js_sys::Function`).
pub fn pump<P, C, M>(
    subscription: Subscription<P, C, M>,
    on_message: js_sys::Function,
) -> TopicSubscription
where
    P: MessagingClientProtocol,
    C: TopicSubscribe<P> + Clone + Unpin + 'static,
    M: Serialize + Unpin + 'static,
{
    let (abort, registration) = AbortHandle::new_pair();

    // The pump owns the `Subscription`; aborting drops it → its `Drop` deregisters.
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
                                "topic message failed to serialize for JS; skipping"
                            );
                        }
                    },

                    Err(error) => {
                        tracing::warn!(
                            target: "overseerd::axum",
                            ?error,
                            "topic subscription error; ending subscription"
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

    TopicSubscription { abort }
}
