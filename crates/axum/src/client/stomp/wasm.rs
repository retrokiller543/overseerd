//! STOMP's [`TopicWasmClient`] impl: it pulls STOMP's shared socket out of the browser
//! [`Connection`]. The protocol-agnostic wasm bridge (the [`TopicSubscription`] handle and the
//! `pump`) lives in [`crate::client::messaging`]; this is the one protocol-specific step.

use wasm_bindgen::prelude::*;

use crate::client::{Connection, TopicWasmClient};
use crate::stomp::Stomp;

use super::StompClientTransport;

impl TopicWasmClient for Stomp {
    type Transport = StompClientTransport;

    fn transport(connection: &Connection) -> Result<StompClientTransport, JsError> {
        connection.stomp()
    }
}
