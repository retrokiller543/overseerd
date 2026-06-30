//! [`JsonWs`]: the baseline JSON-envelope WebSocket protocol.
//!
//! Each inbound text frame is a `{ "dest": "<destination>", "id": <n>, "payload": <json> }` call;
//! each reply is a `{ "dest": <destination>, "id": <n>, "ok": <json> }` or
//! `{ "dest": <destination>, "id": <n>, "error": "<message>" }` frame, echoing the request `id` so a
//! client can correlate replies. This maps the existing request/response handler shape onto a
//! socket; a future STOMP protocol would extend the [`WebsocketProtocol`] seam with
//! subscription/broadcast (server-initiated `MESSAGE` fan-out) without changing it.

use std::collections::HashMap;
use std::sync::Arc;

use axum::extract::ws::{Message, Utf8Bytes, WebSocket};
use overseerd_app::AppRuntime;
use serde::{Deserialize, Serialize};
use tracing::{debug, warn};

use super::{
    WebsocketProtocol, WsControllerDescriptor, WsDispatchError, WsHandlerFn, WsShutdown, WsValue,
};

/// The baseline JSON-envelope protocol: a flat destination → handler table, point-to-point
/// request/response over one socket.
pub struct JsonWs {
    routes: HashMap<&'static str, WsHandlerFn>,
}

/// An inbound call frame.
#[derive(Deserialize)]
struct Inbound {
    dest: String,

    #[serde(default)]
    id: Option<u64>,

    #[serde(default)]
    payload: WsValue,
}

/// An outbound reply frame: exactly one of `ok` / `error` is set.
#[derive(Serialize)]
struct Outbound<'a> {
    dest: &'a str,

    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<u64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    ok: Option<WsValue>,

    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

impl WebsocketProtocol for JsonWs {
    fn build(controllers: &[WsControllerDescriptor], runtime: &AppRuntime) -> Self {
        let mut routes: HashMap<&'static str, WsHandlerFn> = HashMap::new();

        for descriptor in controllers {
            for route in (descriptor.routes)(runtime) {
                if routes.insert(route.destination, route.handler).is_some() {
                    warn!(
                        target: "overseerd::axum",
                        dest = route.destination,
                        "duplicate ws destination registered; last registration wins"
                    );
                }
            }
        }

        Self { routes }
    }

    async fn serve(self: Arc<Self>, mut socket: WebSocket, mut shutdown: WsShutdown) {
        loop {
            tokio::select! {
                _ = shutdown.wait() => {
                    let _ = socket.send(Message::Close(None)).await;

                    break;
                }

                inbound = socket.recv() => {
                    match inbound {
                        Some(Ok(Message::Text(text))) => {
                            let reply = self.handle_text(text.as_str()).await;

                            if let Some(reply) = reply
                                && socket.send(Message::Text(Utf8Bytes::from(reply))).await.is_err()
                            {
                                break;
                            }
                        }

                        Some(Ok(Message::Close(_))) | None => break,

                        // Binary/ping/pong are out of scope for the JSON protocol; ignore.
                        Some(Ok(_)) => {}

                        Some(Err(error)) => {
                            debug!(target: "overseerd::axum", %error, "ws connection read error");

                            break;
                        }
                    }
                }
            }
        }
    }
}

impl JsonWs {
    /// Routes one inbound text frame and renders its reply. Returns `None` for a frame that can't be
    /// parsed at all (no `dest` to correlate a reply against) — it is dropped with a warning.
    async fn handle_text(&self, text: &str) -> Option<String> {
        let inbound: Inbound = match serde_json::from_str(text) {
            Ok(inbound) => inbound,

            Err(error) => {
                warn!(target: "overseerd::axum", %error, "unparseable ws frame; dropping");

                return None;
            }
        };

        let result = match self.routes.get(inbound.dest.as_str()) {
            Some(handler) => handler(inbound.payload).await,

            None => Err(WsDispatchError::NotFound(inbound.dest.clone())),
        };

        let outbound = match result {
            Ok(value) => Outbound {
                dest: &inbound.dest,
                id: inbound.id,
                ok: Some(value),
                error: None,
            },

            Err(error) => Outbound {
                dest: &inbound.dest,
                id: inbound.id,
                ok: None,
                error: Some(error.to_string()),
            },
        };

        serde_json::to_string(&outbound).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A `JsonWs` with one in-memory `echo` route, bypassing the DI/macro plumbing.
    fn echo_protocol() -> JsonWs {
        let mut routes: HashMap<&'static str, WsHandlerFn> = HashMap::new();

        routes.insert(
            "echo",
            Arc::new(|payload: WsValue| {
                Box::pin(async move {
                    let text = payload
                        .get("text")
                        .and_then(WsValue::as_str)
                        .unwrap_or_default()
                        .to_string();

                    super::super::encode_response(&serde_json::json!({ "echo": text }))
                })
            }),
        );

        JsonWs { routes }
    }

    #[tokio::test]
    async fn dispatches_to_handler_and_echoes_id() {
        let proto = echo_protocol();

        let reply = proto
            .handle_text(r#"{"dest":"echo","id":7,"payload":{"text":"hi"}}"#)
            .await
            .expect("a reply frame");
        let value: WsValue = serde_json::from_str(&reply).expect("valid json reply");

        assert_eq!(value["dest"], "echo");
        assert_eq!(value["id"], 7);
        assert_eq!(value["ok"]["echo"], "hi");
        assert!(value.get("error").is_none());
    }

    #[tokio::test]
    async fn unknown_destination_is_an_error_frame() {
        let proto = echo_protocol();

        let reply = proto
            .handle_text(r#"{"dest":"nope","id":1,"payload":{}}"#)
            .await
            .expect("a reply frame");
        let value: WsValue = serde_json::from_str(&reply).expect("valid json reply");

        assert_eq!(value["dest"], "nope");
        assert_eq!(value["id"], 1);
        assert!(value["error"].as_str().unwrap().contains("nope"));
        assert!(value.get("ok").is_none());
    }

    #[tokio::test]
    async fn unparseable_frame_is_dropped() {
        let proto = echo_protocol();

        assert!(proto.handle_text("not json").await.is_none());
    }
}
