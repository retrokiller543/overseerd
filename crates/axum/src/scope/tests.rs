use overseerd_core::{Scope, StaticScope};

use super::{HTTP_REQUEST_SCOPE_ID, HttpRequest, SCOPE_TOPOLOGY};

#[cfg(feature = "ws")]
use super::{
    WEBSOCKET_CONNECTION_SCOPE_ID, WEBSOCKET_MESSAGE_SCOPE_ID, WebsocketConnection,
    WebsocketMessage,
};

#[test]
fn http_request_has_stable_identity_and_root_parent() {
    let topology = SCOPE_TOPOLOGY.prepare().expect("Axum topology is valid");

    assert_eq!(HttpRequest::ID, HTTP_REQUEST_SCOPE_ID);
    assert_eq!(HttpRequest.name(), "HttpRequest");
    assert!(
        topology
            .parent_of(HTTP_REQUEST_SCOPE_ID)
            .expect("HTTP request boundary exists")
            .is_root()
    );
}

#[cfg(feature = "ws")]
#[test]
fn websocket_scopes_are_distinct_and_messages_descend_from_connections() {
    let topology = SCOPE_TOPOLOGY.prepare().expect("Axum topology is valid");

    assert_ne!(HTTP_REQUEST_SCOPE_ID, WEBSOCKET_CONNECTION_SCOPE_ID);
    assert_ne!(HTTP_REQUEST_SCOPE_ID, WEBSOCKET_MESSAGE_SCOPE_ID);
    assert_ne!(WEBSOCKET_CONNECTION_SCOPE_ID, WEBSOCKET_MESSAGE_SCOPE_ID);
    assert_eq!(WebsocketConnection::ID, WEBSOCKET_CONNECTION_SCOPE_ID);
    assert_eq!(WebsocketMessage::ID, WEBSOCKET_MESSAGE_SCOPE_ID);
    assert!(topology.is_ancestor(WEBSOCKET_CONNECTION_SCOPE_ID, WEBSOCKET_MESSAGE_SCOPE_ID));
    assert!(!topology.is_reachable(HTTP_REQUEST_SCOPE_ID, WEBSOCKET_CONNECTION_SCOPE_ID));
}
