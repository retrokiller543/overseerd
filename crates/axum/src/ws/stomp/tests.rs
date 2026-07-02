//! Tests for the STOMP serve-loop helpers (see the parent [`super`] module).

use stomp_parser::client::ClientFrame;

use super::*;

#[test]
fn host_is_injected_so_a_hostless_connect_parses() {
    // A stomp.js-style CONNECT with no `host` header — rejected by stomp-parser as-is.
    let frame = b"CONNECT\naccept-version:1.0,1.1,1.2\nheart-beat:0,0\n\n\x00".to_vec();
    assert!(
        ClientFrame::try_from(frame.clone()).is_err(),
        "hostless CONNECT is rejected raw"
    );

    let patched = ensure_connect_host(frame);
    let parsed = ClientFrame::try_from(patched).expect("patched CONNECT parses");

    assert!(matches!(parsed, ClientFrame::Connect(_)));
}

#[test]
fn a_connect_with_a_host_is_left_untouched() {
    let frame = b"CONNECT\naccept-version:1.2\nhost:example\n\n\x00".to_vec();
    let out = ensure_connect_host(frame.clone());

    assert_eq!(out, frame, "an existing host is not duplicated");
}

#[test]
fn non_connect_frames_are_left_untouched() {
    let frame = b"SEND\ndestination:/app/chat\n\nhi\x00".to_vec();
    let out = ensure_connect_host(frame.clone());

    assert_eq!(out, frame);
}

/// Regression test: a `SEND`'s custom headers (e.g. correlation or auth metadata) must reach
/// the handler's `Inject<StompHeaders>`, not just `destination`/`content-type`.
#[test]
fn send_header_seed_carries_custom_headers_through() {
    let headers = send_header_seed(
        "/app/chat",
        Some("application/json".to_owned()),
        vec![("correlation-id".to_owned(), "abc-123".to_owned())],
    );

    assert_eq!(
        headers,
        vec![
            ("destination".to_owned(), "/app/chat".to_owned()),
            ("content-type".to_owned(), "application/json".to_owned()),
            ("correlation-id".to_owned(), "abc-123".to_owned()),
        ]
    );
}
