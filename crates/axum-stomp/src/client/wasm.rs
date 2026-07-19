//! STOMP's [`TopicWasmClient`] impl: it pulls STOMP's shared socket out of the browser
//! [`Connection`]. The protocol-agnostic wasm bridge (the [`TopicSubscription`] handle and the
//! `pump`) lives in [`crate::client::messaging`]; this is the one protocol-specific step.

use wasm_bindgen::prelude::*;

use crate::Stomp;
use overseerd_axum::client::{Connection, TopicWasmClient};

use super::StompClientTransport;
use super::StompConnectOptions;

/// Connects STOMP in a browser and attaches it to the shared connection.
#[wasm_bindgen(js_name = connectStomp)]
pub async fn connect_stomp(connection: &Connection, endpoint: String) -> Result<(), JsError> {
    connect_stomp_with_options(connection, endpoint, StompConnectOptions::default()).await
}

/// Connects STOMP with explicit CONNECT options and replaces any previous transport.
#[wasm_bindgen(js_name = connectStompWithOptions)]
pub async fn connect_stomp_with_options(
    connection: &Connection,
    endpoint: String,
    options: StompConnectOptions,
) -> Result<(), JsError> {
    disconnect_stomp(connection).await?;

    let url = connection.websocket_url(&endpoint);
    let transport = StompClientTransport::connect_with_options(url, options)
        .await
        .map_err(|error| JsError::new(&error.to_string()))?;

    connection.attach_transport::<Stomp, _>(transport);

    Ok(())
}

/// Gracefully disconnects and detaches STOMP from the shared browser connection.
#[wasm_bindgen(js_name = disconnectStomp)]
pub async fn disconnect_stomp(connection: &Connection) -> Result<(), JsError> {
    if let Some(transport) = connection.detach_transport::<Stomp, StompClientTransport>() {
        transport
            .disconnect()
            .await
            .map_err(|error| JsError::new(&error.to_string()))?;
    }

    Ok(())
}

impl TopicWasmClient for Stomp {
    type Transport = StompClientTransport;

    fn transport(connection: &Connection) -> Result<StompClientTransport, JsError> {
        connection.transport::<Stomp, StompClientTransport>()
    }
}
