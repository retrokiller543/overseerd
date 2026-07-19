use stomp_parser::client::ClientFrame;
use stomp_parser::headers::HeaderValue;

use super::{StompConnectOptions, connect_frame};

#[test]
fn connect_options_encode_credentials_and_custom_headers() {
    let frame = connect_frame(
        StompConnectOptions::new()
            .with_host("example.test")
            .with_login("alice")
            .with_passcode("secret")
            .with_header("authorization", "Bearer token"),
    );
    let ClientFrame::Connect(connect) = ClientFrame::try_from(frame).expect("parse CONNECT") else {
        panic!("expected CONNECT");
    };

    assert_eq!(connect.host().value(), "example.test");
    assert_eq!(connect.login().expect("login").value(), "alice");
    assert_eq!(connect.passcode().expect("passcode").value(), "secret");
    assert!(connect.custom.iter().any(|header| {
        header.header_name() == "authorization" && *header.value() == "Bearer token"
    }));
}

#[test]
fn connect_options_debug_redacts_the_passcode() {
    let options = StompConnectOptions::new()
        .with_login("alice")
        .with_passcode("super-secret")
        .with_header("authorization", "Bearer secret-token");
    let debug = format!("{options:?}");

    assert!(debug.contains("alice"));
    assert!(debug.contains("[REDACTED]"));
    assert!(debug.contains("authorization"));
    assert!(!debug.contains("super-secret"));
    assert!(!debug.contains("secret-token"));
}
