//! Generic HTTP client interception.

use http::{StatusCode, request, response};
use overseerd_client::ClientError;

/// Hooks run by the bundled HTTP clients around every request.
///
/// The interceptor is a generic type parameter stored directly on the client transport. No trait
/// objects or callback registry are involved. Request/response hooks receive the actual HTTP parts,
/// so method, URI, status, and headers can be mutated in place.
pub trait ClientInterceptor {
    /// Runs after the absolute URI and default headers are assembled, immediately before send.
    fn on_request(&self, _request: &mut request::Parts) {}

    /// Runs as soon as response status and headers arrive, before classification or decoding.
    fn on_response(&self, _response: &mut response::Parts) {}

    /// Observes a terminal client error.
    fn on_error<E>(&self, _error: &ClientError<StatusCode, E>) {}
}

impl ClientInterceptor for () {}

/// The default interceptor stored by the bundled HTTP client transports.
#[cfg(not(all(target_family = "wasm", feature = "reqwest")))]
pub type DefaultClientInterceptor = ();

/// The default interceptor stored by [`ReqwestClient`](super::ReqwestClient) in a browser build.
#[cfg(all(target_family = "wasm", feature = "reqwest"))]
pub type DefaultClientInterceptor = WasmClientInterceptor;

/// Browser interceptor holding the three JavaScript callbacks directly.
#[cfg(all(target_family = "wasm", feature = "reqwest"))]
#[derive(Clone, Default)]
pub struct WasmClientInterceptor {
    callbacks: std::sync::Arc<std::sync::RwLock<WasmCallbacks>>,
}

#[cfg(all(target_family = "wasm", feature = "reqwest"))]
#[derive(Default)]
struct WasmCallbacks {
    request: Option<js_sys::Function>,
    response: Option<js_sys::Function>,
    error: Option<js_sys::Function>,
}

#[cfg(all(target_family = "wasm", feature = "reqwest"))]
impl WasmClientInterceptor {
    pub(crate) fn set_on_request(&self, callback: Option<js_sys::Function>) {
        self.callbacks
            .write()
            .unwrap_or_else(|poison| poison.into_inner())
            .request = callback;
    }

    pub(crate) fn set_on_response(&self, callback: Option<js_sys::Function>) {
        self.callbacks
            .write()
            .unwrap_or_else(|poison| poison.into_inner())
            .response = callback;
    }

    pub(crate) fn set_on_error(&self, callback: Option<js_sys::Function>) {
        self.callbacks
            .write()
            .unwrap_or_else(|poison| poison.into_inner())
            .error = callback;
    }

    fn callback(
        &self,
        select: impl FnOnce(&WasmCallbacks) -> Option<js_sys::Function>,
    ) -> Option<js_sys::Function> {
        select(
            &self
                .callbacks
                .read()
                .unwrap_or_else(|poison| poison.into_inner()),
        )
    }

    fn report_callback_error(name: &str, error: wasm_bindgen::JsValue) {
        tracing::warn!(
            target: "overseerd::axum",
            callback = name,
            error = ?error,
            "HTTP client callback threw"
        );
    }
}

#[cfg(all(target_family = "wasm", feature = "reqwest"))]
impl ClientInterceptor for WasmClientInterceptor {
    fn on_request(&self, request: &mut request::Parts) {
        use wasm_bindgen::JsValue;

        let Some(callback) = self.callback(|callbacks| callbacks.request.clone()) else {
            return;
        };
        let state = std::rc::Rc::new(std::cell::RefCell::new(std::mem::replace(
            request,
            empty_request_parts(),
        )));
        let argument = WasmRequestParts {
            state: std::rc::Rc::clone(&state),
        };

        if let Err(error) = callback.call1(&JsValue::NULL, &JsValue::from(argument)) {
            Self::report_callback_error("onRequest", error);
        }

        *request = std::mem::replace(&mut *state.borrow_mut(), empty_request_parts());
    }

    fn on_response(&self, response: &mut response::Parts) {
        use wasm_bindgen::JsValue;

        let Some(callback) = self.callback(|callbacks| callbacks.response.clone()) else {
            return;
        };
        let state = std::rc::Rc::new(std::cell::RefCell::new(std::mem::replace(
            response,
            empty_response_parts(),
        )));
        let argument = WasmResponseParts {
            state: std::rc::Rc::clone(&state),
        };

        if let Err(error) = callback.call1(&JsValue::NULL, &JsValue::from(argument)) {
            Self::report_callback_error("onResponse", error);
        }

        *response = std::mem::replace(&mut *state.borrow_mut(), empty_response_parts());
    }

    fn on_error<E>(&self, error: &ClientError<StatusCode, E>) {
        use wasm_bindgen::JsValue;

        let Some(callback) = self.callback(|callbacks| callbacks.error.clone()) else {
            return;
        };
        let argument = WasmError {
            kind: error_kind(error).to_owned(),
            message: error.to_string(),
            status: remote_status(error),
        };

        if let Err(error) = callback.call1(&JsValue::NULL, &JsValue::from(argument)) {
            Self::report_callback_error("onError", error);
        }
    }
}

#[cfg(all(target_family = "wasm", feature = "reqwest"))]
fn empty_request_parts() -> request::Parts {
    http::Request::new(()).into_parts().0
}

#[cfg(all(target_family = "wasm", feature = "reqwest"))]
fn empty_response_parts() -> response::Parts {
    http::Response::new(()).into_parts().0
}

#[cfg(all(target_family = "wasm", feature = "reqwest"))]
fn error_kind<E>(error: &ClientError<StatusCode, E>) -> &'static str {
    match error {
        ClientError::Transport(_) => "transport",
        ClientError::Encode(_) => "encode",
        ClientError::Decode(_) => "decode",
        ClientError::Remote(_) => "remote",
        ClientError::ConnectionClosed => "connectionClosed",
    }
}

#[cfg(all(target_family = "wasm", feature = "reqwest"))]
fn remote_status<E>(error: &ClientError<StatusCode, E>) -> Option<u16> {
    match error {
        ClientError::Remote(error) => Some(error.code().as_u16()),
        _ => None,
    }
}

#[cfg(all(target_family = "wasm", feature = "reqwest"))]
fn get_header(headers: &http::HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
}

#[cfg(all(target_family = "wasm", feature = "reqwest"))]
fn set_header(
    headers: &mut http::HeaderMap,
    name: String,
    value: String,
) -> Result<(), wasm_bindgen::JsError> {
    use http::header::{HeaderName, HeaderValue};

    let name = name
        .parse::<HeaderName>()
        .map_err(|error| wasm_bindgen::JsError::new(&error.to_string()))?;
    let value = value
        .parse::<HeaderValue>()
        .map_err(|error| wasm_bindgen::JsError::new(&error.to_string()))?;
    headers.insert(name, value);
    Ok(())
}

#[cfg(all(target_family = "wasm", feature = "reqwest"))]
#[wasm_bindgen::prelude::wasm_bindgen(js_name = ClientRequest)]
struct WasmRequestParts {
    state: std::rc::Rc<std::cell::RefCell<request::Parts>>,
}

#[cfg(all(target_family = "wasm", feature = "reqwest"))]
#[wasm_bindgen::prelude::wasm_bindgen(js_class = ClientRequest)]
impl WasmRequestParts {
    #[wasm_bindgen::prelude::wasm_bindgen(getter)]
    pub fn method(&self) -> String {
        self.state.borrow().method.to_string()
    }

    #[wasm_bindgen::prelude::wasm_bindgen(js_name = setMethod)]
    pub fn set_method(&self, method: String) -> Result<(), wasm_bindgen::JsError> {
        self.state.borrow_mut().method = method
            .parse::<http::Method>()
            .map_err(|error| wasm_bindgen::JsError::new(&error.to_string()))?;
        Ok(())
    }

    #[wasm_bindgen::prelude::wasm_bindgen(getter)]
    pub fn url(&self) -> String {
        self.state.borrow().uri.to_string()
    }

    #[wasm_bindgen::prelude::wasm_bindgen(js_name = setUrl)]
    pub fn set_url(&self, url: String) -> Result<(), wasm_bindgen::JsError> {
        self.state.borrow_mut().uri = url
            .parse::<http::Uri>()
            .map_err(|error| wasm_bindgen::JsError::new(&error.to_string()))?;
        Ok(())
    }

    #[wasm_bindgen::prelude::wasm_bindgen(js_name = getHeader)]
    pub fn get_header(&self, name: String) -> Option<String> {
        get_header(&self.state.borrow().headers, &name)
    }

    #[wasm_bindgen::prelude::wasm_bindgen(js_name = setHeader)]
    pub fn set_header(&self, name: String, value: String) -> Result<(), wasm_bindgen::JsError> {
        set_header(&mut self.state.borrow_mut().headers, name, value)
    }

    #[wasm_bindgen::prelude::wasm_bindgen(js_name = removeHeader)]
    pub fn remove_header(&self, name: String) {
        self.state.borrow_mut().headers.remove(name.as_str());
    }
}

#[cfg(all(target_family = "wasm", feature = "reqwest"))]
#[wasm_bindgen::prelude::wasm_bindgen(js_name = ClientResponse)]
struct WasmResponseParts {
    state: std::rc::Rc<std::cell::RefCell<response::Parts>>,
}

#[cfg(all(target_family = "wasm", feature = "reqwest"))]
#[wasm_bindgen::prelude::wasm_bindgen(js_class = ClientResponse)]
impl WasmResponseParts {
    #[wasm_bindgen::prelude::wasm_bindgen(getter)]
    pub fn status(&self) -> u16 {
        self.state.borrow().status.as_u16()
    }

    #[wasm_bindgen::prelude::wasm_bindgen(js_name = setStatus)]
    pub fn set_status(&self, status: u16) -> Result<(), wasm_bindgen::JsError> {
        self.state.borrow_mut().status = StatusCode::from_u16(status)
            .map_err(|error| wasm_bindgen::JsError::new(&error.to_string()))?;
        Ok(())
    }

    #[wasm_bindgen::prelude::wasm_bindgen(js_name = getHeader)]
    pub fn get_header(&self, name: String) -> Option<String> {
        get_header(&self.state.borrow().headers, &name)
    }

    #[wasm_bindgen::prelude::wasm_bindgen(js_name = setHeader)]
    pub fn set_header(&self, name: String, value: String) -> Result<(), wasm_bindgen::JsError> {
        set_header(&mut self.state.borrow_mut().headers, name, value)
    }

    #[wasm_bindgen::prelude::wasm_bindgen(js_name = removeHeader)]
    pub fn remove_header(&self, name: String) {
        self.state.borrow_mut().headers.remove(name.as_str());
    }
}

#[cfg(all(target_family = "wasm", feature = "reqwest"))]
#[wasm_bindgen::prelude::wasm_bindgen(js_name = ClientError)]
struct WasmError {
    kind: String,
    message: String,
    status: Option<u16>,
}

#[cfg(all(target_family = "wasm", feature = "reqwest"))]
#[wasm_bindgen::prelude::wasm_bindgen(js_class = ClientError)]
impl WasmError {
    #[wasm_bindgen::prelude::wasm_bindgen(getter)]
    pub fn kind(&self) -> String {
        self.kind.clone()
    }

    #[wasm_bindgen::prelude::wasm_bindgen(getter)]
    pub fn message(&self) -> String {
        self.message.clone()
    }

    #[wasm_bindgen::prelude::wasm_bindgen(getter)]
    pub fn status(&self) -> Option<u16> {
        self.status
    }
}

#[cfg(test)]
mod tests;
