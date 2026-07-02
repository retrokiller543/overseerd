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
#[cfg(feature = "client")]
use overseerd_client::{ClientError, ErrorBody};
use overseerd_di::ScopeContainer;
#[cfg(feature = "client")]
use overseerd_transport::CodecError;
#[cfg(feature = "client")]
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use tracing::{debug, warn};

use super::{
    WebsocketProtocol, WsCodec, WsControllerDescriptor, WsDispatchError, WsHandlerFn, WsRespond,
    WsShutdown, WsValue,
};
#[cfg(feature = "client")]
use crate::client::{WebsocketClientProtocol, WebsocketDecodes, WebsocketEncodes, WsStatus};

/// The baseline JSON-envelope protocol: a flat destination → handler table, point-to-point
/// request/response over one socket. Holds a clone of the [`AppRuntime`] so it can open a fresh
/// per-message [`Request`](crate::scope::Request) scope for handler DI.
pub struct JsonWs {
    routes: HashMap<&'static str, WsHandlerFn<Self>>,
    runtime: AppRuntime,
}

/// [`JsonWs`]'s handler outcome: the `ok` JSON value of a single correlated reply. A thin newtype
/// so the generalized [`WsRespond`] can target it while `serve()` keeps rendering one reply frame.
pub struct WsReply(pub WsValue);

/// An inbound call frame.
#[derive(Deserialize, Serialize)]
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

#[cfg(feature = "client")]
#[derive(Deserialize)]
struct ClientOutbound {
    id: Option<u64>,

    #[serde(default)]
    ok: Option<WsValue>,

    #[serde(default)]
    error: Option<String>,
}

impl WebsocketProtocol for JsonWs {
    type Payload = WsValue;
    type Outcome = WsReply;
    type Options = ();

    fn build(controllers: &[WsControllerDescriptor], runtime: &AppRuntime, _options: ()) -> Self {
        let mut routes: HashMap<&'static str, WsHandlerFn<Self>> = HashMap::new();

        for descriptor in controllers {
            for route in descriptor.routes_for::<Self>(runtime) {
                if routes.insert(route.destination, route.handler).is_some() {
                    warn!(
                        target: "overseerd::axum",
                        dest = route.destination,
                        "duplicate ws destination registered; last registration wins"
                    );
                }
            }
        }

        Self {
            routes,
            runtime: runtime.clone(),
        }
    }

    async fn serve(
        self: Arc<Self>,
        mut socket: WebSocket,
        connection: Arc<ScopeContainer>,
        mut shutdown: WsShutdown,
    ) {
        loop {
            tokio::select! {
                _ = shutdown.wait() => {
                    let _ = socket.send(Message::Close(None)).await;

                    break;
                }

                inbound = socket.recv() => {
                    match inbound {
                        Some(Ok(Message::Text(text))) => {
                            let reply = self.handle_text(text.as_str(), &connection).await;

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
    ///
    /// Opens a fresh per-message [`Request`](crate::scope::Request) scope parented at the socket's
    /// `connection` scope, so a handler's `Inject<T>` resolves request-scoped components per message
    /// (and connection-/singleton-scoped ones through the chain).
    async fn handle_text(&self, text: &str, connection: &Arc<ScopeContainer>) -> Option<String> {
        let inbound: Inbound = match serde_json::from_str(text) {
            Ok(inbound) => inbound,

            Err(error) => {
                warn!(target: "overseerd::axum", %error, "unparseable ws frame; dropping");

                return None;
            }
        };

        let result = match self.routes.get(inbound.dest.as_str()) {
            Some(handler) => match self
                .runtime
                .open_scope(&crate::scope::Request, Arc::clone(connection), Vec::new())
                .await
            {
                Ok(scope) => handler(inbound.payload, scope).await,

                Err(error) => Err(WsDispatchError::Inject(error.to_string())),
            },

            None => Err(WsDispatchError::NotFound(inbound.dest.clone())),
        };

        render_reply(&inbound.dest, inbound.id, result)
    }
}

impl<T> WsCodec<T> for JsonWs
where
    T: serde::de::DeserializeOwned,
{
    fn decode(payload: WsValue) -> Result<T, WsDispatchError> {
        serde_json::from_value(payload).map_err(|e| WsDispatchError::Decode(e.to_string()))
    }
}

impl<R> WsRespond<R> for JsonWs
where
    R: serde::Serialize,
{
    fn respond(response: R) -> Result<WsReply, WsDispatchError> {
        serde_json::to_value(&response)
            .map(WsReply)
            .map_err(|e| WsDispatchError::Encode(e.to_string()))
    }
}

#[cfg(feature = "client")]
impl WebsocketClientProtocol for JsonWs {
    type Key = u64;
    type Frame = String;
    type Payload = WsValue;

    fn next_key(counter: u64) -> Self::Key {
        counter
    }

    fn encode_call<T>(destination: &str, key: &Self::Key, payload: T) -> Result<String, CodecError>
    where
        Self: WebsocketEncodes<T>,
    {
        let inbound = Inbound {
            dest: destination.to_string(),
            id: Some(*key),
            payload: <Self as WebsocketEncodes<T>>::encode_payload(payload)?,
        };

        serde_json::to_string(&inbound).map_err(|e| CodecError::internal(e.to_string()))
    }

    fn reply_key(frame: &Self::Frame) -> Result<Option<Self::Key>, CodecError> {
        let outbound: ClientOutbound =
            serde_json::from_str(frame).map_err(|e| CodecError::bad_input(e.to_string()))?;

        Ok(outbound.id)
    }

    fn decode_reply<T>(frame: Self::Frame) -> Result<T, ClientError<WsStatus>>
    where
        Self: WebsocketDecodes<T>,
    {
        let outbound: ClientOutbound =
            serde_json::from_str(&frame).map_err(|e| ClientError::Decode(e.to_string()))?;

        if let Some(error) = outbound.error {
            return Err(ClientError::Remote(ErrorBody::new(
                WsStatus::Error,
                error.into_bytes(),
            )));
        }

        let ok = outbound
            .ok
            .ok_or_else(|| ClientError::Decode("ws reply contained neither ok nor error".into()))?;

        <Self as WebsocketDecodes<T>>::decode_payload(ok)
            .map_err(|e| ClientError::Decode(e.to_string()))
    }
}

#[cfg(feature = "client")]
impl<T> WebsocketEncodes<T> for JsonWs
where
    T: Serialize,
{
    fn encode_payload(value: T) -> Result<<Self as WebsocketClientProtocol>::Payload, CodecError> {
        serde_json::to_value(value).map_err(|e| CodecError::bad_input(e.to_string()))
    }
}

#[cfg(feature = "client")]
impl<T> WebsocketDecodes<T> for JsonWs
where
    T: DeserializeOwned,
{
    fn decode_payload(value: <Self as WebsocketClientProtocol>::Payload) -> Result<T, CodecError> {
        serde_json::from_value(value).map_err(|e| CodecError::bad_input(e.to_string()))
    }
}

/// Renders a dispatch result into the JSON reply frame (correlating the request `id`): a `{ok}`
/// frame on success, a `{error}` frame on failure. Pure (no scope), so the framing is unit-testable.
fn render_reply(
    dest: &str,
    id: Option<u64>,
    result: Result<WsReply, WsDispatchError>,
) -> Option<String> {
    let outbound = match result {
        Ok(WsReply(value)) => Outbound {
            dest,
            id,
            ok: Some(value),
            error: None,
        },

        Err(error) => Outbound {
            dest,
            id,
            ok: None,
            error: Some(error.to_string()),
        },
    };

    serde_json::to_string(&outbound).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ok_result_renders_an_ok_frame_echoing_the_id() {
        let reply = render_reply(
            "echo",
            Some(7),
            Ok(WsReply(serde_json::json!({ "echo": "hi" }))),
        )
        .expect("a reply frame");
        let value: WsValue = serde_json::from_str(&reply).expect("valid json reply");

        assert_eq!(value["dest"], "echo");
        assert_eq!(value["id"], 7);
        assert_eq!(value["ok"]["echo"], "hi");
        assert!(value.get("error").is_none());
    }

    #[test]
    fn error_result_renders_an_error_frame() {
        let reply = render_reply(
            "nope",
            Some(1),
            Err(WsDispatchError::NotFound("nope".to_string())),
        )
        .expect("a reply frame");
        let value: WsValue = serde_json::from_str(&reply).expect("valid json reply");

        assert_eq!(value["dest"], "nope");
        assert_eq!(value["id"], 1);
        assert!(value["error"].as_str().unwrap().contains("nope"));
        assert!(value.get("ok").is_none());
    }

    #[test]
    fn inbound_frame_parses_dest_id_and_payload() {
        let inbound: Inbound =
            serde_json::from_str(r#"{"dest":"chat.send","id":9,"payload":{"text":"hi"}}"#)
                .expect("parse");

        assert_eq!(inbound.dest, "chat.send");
        assert_eq!(inbound.id, Some(9));
        assert_eq!(inbound.payload["text"], "hi");
    }
}
