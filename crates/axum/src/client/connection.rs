//! The shared browser-client connection.
//!
//! `Connection` is the single object a JS/TS app creates and hands to every generated client — the
//! REST controller clients, the STOMP `#[topics]` subscribe client, and the STOMP `#[message]` SEND
//! client. They all share its one `reqwest::Client` (so the HTTP connection pool and the browser's
//! cookie handling stay consistent across every call) and its one STOMP socket (so publishing and
//! subscribing ride the same WebSocket, not two). It is exported to JS and is meant to be *extended*
//! in TS — a subclass that layers on auth or config still passes as a `Connection` at the boundary.
//!
//! wasm-only: the native client shares a transport by simply cloning it into each generated
//! `Client::new(transport)`, which needs no wrapper.

#[cfg(all(feature = "stomp", feature = "tungstenite"))]
use core::cell::RefCell;

use wasm_bindgen::prelude::*;

use super::ReqwestClient;
#[cfg(all(feature = "stomp", feature = "tungstenite"))]
use super::{StompClientTransport, StompConnectOptions};

/// The shared connection every generated browser client is constructed from. Holds one HTTP
/// transport (and, once [`connectStomp`](Connection::connect_stomp) has run, one STOMP socket),
/// cheaply cloned into each client so they all share the same underlying Rust connection.
#[wasm_bindgen]
pub struct Connection {
    /// The base URL (scheme + authority, e.g. `http://localhost:3001`), kept so the ws scheme can be
    /// swapped in for the STOMP upgrade. The HTTP transport holds its own copy, so this is needed
    /// only when a ws transport can be attached — hence gated to that build.
    #[cfg(all(feature = "stomp", feature = "tungstenite"))]
    base_url: String,

    /// The shared HTTP transport — one `reqwest::Client` (pool + cookies) for every REST client.
    http: ReqwestClient,

    /// The shared STOMP socket, once connected. Interior-mutable so `connectStomp` can attach it
    /// through a `&self` async method (no `&mut self` held across `await`).
    #[cfg(all(feature = "stomp", feature = "tungstenite"))]
    stomp: RefCell<Option<StompClientTransport>>,
}

#[wasm_bindgen]
impl Connection {
    /// Opens a connection against `base_url` (e.g. `"http://localhost:3001"`). HTTP is ready
    /// immediately; call [`connectStomp`](Connection::connect_stomp) to also attach a STOMP socket.
    #[wasm_bindgen(constructor)]
    pub fn new(base_url: String) -> Connection {
        Connection {
            http: ReqwestClient::new(base_url.clone()),
            #[cfg(all(feature = "stomp", feature = "tungstenite"))]
            base_url,
            #[cfg(all(feature = "stomp", feature = "tungstenite"))]
            stomp: RefCell::new(None),
        }
    }

    /// Installs or clears the global synchronous request callback. It receives a mutable
    /// `ClientRequest`, whose method, URL, and headers can be changed before fetch runs.
    #[wasm_bindgen(js_name = onRequest)]
    pub fn on_request(&self, callback: Option<js_sys::Function>) {
        self.http.interceptor().set_on_request(callback);
    }

    /// Installs or clears the global synchronous response callback. It receives a mutable
    /// `ClientResponse` before status classification and body decoding.
    #[wasm_bindgen(js_name = onResponse)]
    pub fn on_response(&self, callback: Option<js_sys::Function>) {
        self.http.interceptor().set_on_response(callback);
    }

    /// Installs or clears the global error callback. It receives a read-only `ClientError` with
    /// `kind`, `message`, and an optional HTTP `status`.
    #[wasm_bindgen(js_name = onError)]
    pub fn on_error(&self, callback: Option<js_sys::Function>) {
        self.http.interceptor().set_on_error(callback);
    }

    /// Connects (and performs the STOMP handshake over) the WebSocket at `endpoint` — the upgrade
    /// *path* on this connection's own host (e.g. `"/ws/stomp"`). The scheme is derived from the
    /// base URL (`http`→`ws`, `https`→`wss`), so the STOMP socket rides the same host as REST. Await
    /// it once before using the subscribe/SEND clients built from this connection.
    #[cfg(all(feature = "stomp", feature = "tungstenite"))]
    #[wasm_bindgen(js_name = connectStomp)]
    pub async fn connect_stomp(&self, endpoint: String) -> Result<(), JsError> {
        self.connect_stomp_with_options(endpoint, StompConnectOptions::default())
            .await
    }

    /// Connects STOMP with standard credentials and/or custom CONNECT headers. Reconnecting first
    /// disconnects the previous shared socket, so clients created from this `Connection` observe a
    /// single deterministic lifecycle.
    #[cfg(all(feature = "stomp", feature = "tungstenite"))]
    #[wasm_bindgen(js_name = connectStompWithOptions)]
    pub async fn connect_stomp_with_options(
        &self,
        endpoint: String,
        options: StompConnectOptions,
    ) -> Result<(), JsError> {
        self.disconnect_stomp().await?;

        let url = self.ws_url(&endpoint);
        let transport = StompClientTransport::connect_with_options(url, options)
            .await
            .map_err(|e| JsError::new(&e.to_string()))?;

        *self.stomp.borrow_mut() = Some(transport);

        Ok(())
    }

    /// Gracefully disconnects the shared STOMP socket. Idempotent; all generated clients and live
    /// subscriptions created from this `Connection` observe the same connection closing.
    #[cfg(all(feature = "stomp", feature = "tungstenite"))]
    #[wasm_bindgen(js_name = disconnectStomp)]
    pub async fn disconnect_stomp(&self) -> Result<(), JsError> {
        let transport = self.stomp.borrow_mut().take();

        if let Some(transport) = transport {
            transport
                .disconnect()
                .await
                .map_err(|error| JsError::new(&error.to_string()))?;
        }

        Ok(())
    }

    #[cfg(all(feature = "stomp", feature = "tungstenite"))]
    #[wasm_bindgen(js_name = isStompConnected)]
    pub fn is_stomp_connected(&self) -> bool {
        self.stomp
            .borrow()
            .as_ref()
            .is_some_and(StompClientTransport::is_connected)
    }
}

// Internal accessors the generated wasm clients build on (not exported to JS).
impl Connection {
    /// A handle to the shared HTTP transport (a cheap `Arc`-backed clone of the one `reqwest::Client`).
    pub fn http(&self) -> ReqwestClient {
        self.http.clone()
    }

    /// Builds the WebSocket URL for `endpoint` on this connection's host: the base URL with its
    /// scheme swapped to the ws equivalent (`https`→`wss`, else `ws`) and `endpoint` appended. An
    /// `endpoint` that is already an absolute `ws(s)://` URL is used verbatim (an escape hatch for a
    /// STOMP broker on a different host).
    #[cfg(all(feature = "stomp", feature = "tungstenite"))]
    fn ws_url(&self, endpoint: &str) -> String {
        if endpoint.starts_with("ws://") || endpoint.starts_with("wss://") {
            return endpoint.to_owned();
        }

        let base = self.base_url.trim_end_matches('/');
        let ws_base = if let Some(rest) = base.strip_prefix("https://") {
            format!("wss://{rest}")
        } else if let Some(rest) = base.strip_prefix("http://") {
            format!("ws://{rest}")
        } else {
            base.to_owned()
        };

        if endpoint.starts_with('/') {
            format!("{ws_base}{endpoint}")
        } else {
            format!("{ws_base}/{endpoint}")
        }
    }

    /// A handle to the shared STOMP transport, or an error if `connectStomp` has not run yet.
    #[cfg(all(feature = "stomp", feature = "tungstenite"))]
    pub fn stomp(&self) -> Result<StompClientTransport, JsError> {
        let transport = self.stomp.borrow().clone().ok_or_else(|| {
            JsError::new("STOMP is not connected — call `connectStomp(url)` first")
        })?;

        if !transport.is_connected() {
            return Err(JsError::new(
                "STOMP connection is closed — call `connectStomp(url)` to reconnect",
            ));
        }

        Ok(transport)
    }
}

#[cfg(all(feature = "stomp", feature = "tungstenite"))]
impl Drop for Connection {
    fn drop(&mut self) {
        if let Some(transport) = self.stomp.get_mut().take() {
            transport.disconnect_now();
        }
    }
}
