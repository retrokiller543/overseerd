use overseerd_core::{Scope, StaticScope};

use super::{HttpRequest, SCOPE_TOPOLOGY};

#[cfg(feature = "ws")]
use super::{WebsocketConnection, WebsocketMessage};

#[test]
fn http_request_has_stable_identity_and_root_parent() {
    let topology = SCOPE_TOPOLOGY.prepare().expect("Axum topology is valid");

    assert_eq!(HttpRequest.name(), "HttpRequest");
    assert!(
        topology
            .parent_of(<HttpRequest as StaticScope>::ID)
            .expect("HTTP request boundary exists")
            .is_root()
    );
}

#[cfg(feature = "ws")]
#[test]
fn websocket_scopes_are_distinct_and_messages_descend_from_connections() {
    let topology = SCOPE_TOPOLOGY.prepare().expect("Axum topology is valid");

    assert_ne!(
        <HttpRequest as StaticScope>::ID,
        <WebsocketConnection as StaticScope>::ID
    );
    assert_ne!(
        <HttpRequest as StaticScope>::ID,
        <WebsocketMessage as StaticScope>::ID
    );
    assert_ne!(
        <WebsocketConnection as StaticScope>::ID,
        <WebsocketMessage as StaticScope>::ID
    );
    assert!(topology.is_ancestor(
        <WebsocketConnection as StaticScope>::ID,
        <WebsocketMessage as StaticScope>::ID
    ));
    assert!(!topology.is_reachable(
        <HttpRequest as StaticScope>::ID,
        <WebsocketConnection as StaticScope>::ID
    ));
}
