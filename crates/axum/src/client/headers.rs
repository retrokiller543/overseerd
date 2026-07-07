//! Per-call request headers for the generated client.
//!
//! A generated method's `_with_headers` sibling (native) takes an `Option<http::HeaderMap>` directly.
//! A browser client can't name `http::HeaderMap` across the wasm boundary, so it builds this
//! [`RequestHeaders`] type instead — a thin `#[wasm_bindgen]` wrapper over a `HeaderMap` — and hands
//! it to the generated `{method}(…, headers?)`. It is exported to JS the same way
//! [`Multipart`](super::Multipart) is (named `RequestHeaders` to avoid clashing with the DOM
//! `Headers`), so the browser builds headers with `new RequestHeaders().set("authorization", token)`.

use http::header::{HeaderMap, HeaderName, HeaderValue};

/// A set of request headers built on the client and folded into one call's request. Invalid header
/// names/values are ignored (a browser client can't surface a `Result` ergonomically); the generated
/// native `_with_headers` methods take an `http::HeaderMap` directly and need no wrapper.
#[cfg_attr(
    all(target_family = "wasm", feature = "reqwest"),
    ::wasm_bindgen::prelude::wasm_bindgen
)]
#[derive(Default)]
pub struct RequestHeaders(HeaderMap);

#[cfg_attr(
    all(target_family = "wasm", feature = "reqwest"),
    ::wasm_bindgen::prelude::wasm_bindgen
)]
impl RequestHeaders {
    /// An empty header set.
    #[cfg_attr(
        all(target_family = "wasm", feature = "reqwest"),
        ::wasm_bindgen::prelude::wasm_bindgen(constructor)
    )]
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets a header (replacing any current value for that name). An invalid name or value is a no-op.
    pub fn set(&mut self, name: String, value: String) {
        if let (Ok(name), Ok(value)) = (name.parse::<HeaderName>(), value.parse::<HeaderValue>()) {
            self.0.insert(name, value);
        }
    }
}

impl RequestHeaders {
    /// The built header map — used by the generated wasm client wrapper to forward these headers to
    /// the transport-generic `_with_headers` method.
    pub fn into_inner(self) -> HeaderMap {
        self.0
    }
}

impl From<HeaderMap> for RequestHeaders {
    fn from(headers: HeaderMap) -> Self {
        Self(headers)
    }
}
