//! The shared browser-client connection.
//!
//! `Connection` owns a base URL, an optional shared HTTP transport, and a protocol-keyed registry
//! of WebSocket transports. Protocol crates attach their transport through the generic Rust API;
//! generated clients retrieve it through [`TopicWasmClient`](super::TopicWasmClient).

use std::any::{Any, TypeId};
use std::cell::RefCell;
use std::collections::HashMap;

use wasm_bindgen::prelude::*;

#[cfg(feature = "reqwest")]
use super::ReqwestClient;

/// The shared connection every generated browser client is constructed from.
#[wasm_bindgen]
pub struct Connection {
    base_url: String,
    transports: RefCell<HashMap<TypeId, Box<dyn Any>>>,

    #[cfg(feature = "reqwest")]
    http: ReqwestClient,
}

#[wasm_bindgen]
impl Connection {
    /// Creates a shared browser connection rooted at `base_url`.
    #[wasm_bindgen(constructor)]
    pub fn new(base_url: String) -> Self {
        Self {
            #[cfg(feature = "reqwest")]
            http: ReqwestClient::new(base_url.clone()),
            base_url,
            transports: RefCell::new(HashMap::new()),
        }
    }

    /// Installs or clears the global synchronous HTTP request callback.
    #[cfg(feature = "reqwest")]
    #[wasm_bindgen(js_name = onRequest)]
    pub fn on_request(&self, callback: Option<js_sys::Function>) {
        self.http.interceptor().set_on_request(callback);
    }

    /// Installs or clears the global synchronous HTTP response callback.
    #[cfg(feature = "reqwest")]
    #[wasm_bindgen(js_name = onResponse)]
    pub fn on_response(&self, callback: Option<js_sys::Function>) {
        self.http.interceptor().set_on_response(callback);
    }

    /// Installs or clears the global HTTP error callback.
    #[cfg(feature = "reqwest")]
    #[wasm_bindgen(js_name = onError)]
    pub fn on_error(&self, callback: Option<js_sys::Function>) {
        self.http.interceptor().set_on_error(callback);
    }
}

impl Connection {
    /// A handle to the shared HTTP transport.
    #[cfg(feature = "reqwest")]
    pub fn http(&self) -> ReqwestClient {
        self.http.clone()
    }

    /// Builds a same-host WebSocket URL, or returns an already absolute `ws://`/`wss://` URL.
    pub fn websocket_url(&self, endpoint: &str) -> String {
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

    /// Attaches or replaces protocol `P`'s shared browser transport.
    pub fn attach_transport<P, T>(&self, transport: T)
    where
        P: 'static,
        T: Clone + 'static,
    {
        self.transports
            .borrow_mut()
            .insert(TypeId::of::<P>(), Box::new(transport));
    }

    /// Retrieves protocol `P`'s shared browser transport.
    pub fn transport<P, T>(&self) -> Result<T, JsError>
    where
        P: 'static,
        T: Clone + 'static,
    {
        self.transports
            .borrow()
            .get(&TypeId::of::<P>())
            .and_then(|transport| transport.downcast_ref::<T>())
            .cloned()
            .ok_or_else(|| {
                JsError::new(&format!(
                    "WebSocket protocol `{}` is not connected",
                    std::any::type_name::<P>()
                ))
            })
    }

    /// Detaches and returns protocol `P`'s shared browser transport.
    pub fn detach_transport<P, T>(&self) -> Option<T>
    where
        P: 'static,
        T: 'static,
    {
        self.transports
            .borrow_mut()
            .remove(&TypeId::of::<P>())
            .and_then(|transport| transport.downcast::<T>().ok())
            .map(|transport| *transport)
    }
}
