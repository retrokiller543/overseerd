//! Tests for [`JsonWs`](super::JsonWs)'s reply framing and inbound parsing.

use super::*;

#[test]
fn ok_result_renders_an_ok_frame_echoing_the_id() {
    let reply = render_reply(
        "echo",
        Some(7),
        Ok(WsReply(Some(serde_json::json!({ "echo": "hi" })))),
    )
    .expect("a reply frame");
    let value: WsValue = serde_json::from_str(&reply).expect("valid json reply");

    assert_eq!(value["dest"], "echo");
    assert_eq!(value["id"], 7);
    assert_eq!(value["ok"]["echo"], "hi");
    assert!(value.get("error").is_none());
}

#[test]
fn uncorrelated_send_success_and_error_emit_no_reply() {
    assert!(render_reply("send", None, Ok(WsReply(None))).is_none());
    assert!(
        render_reply(
            "send",
            None,
            Err(WsDispatchError::Application("visible".to_owned())),
        )
        .is_none()
    );
}

#[test]
fn application_error_text_is_visible_when_correlated() {
    let reply = render_reply(
        "request",
        Some(9),
        Err(WsDispatchError::Application("safe message".to_owned())),
    )
    .expect("correlated error reply");
    let value: WsValue = serde_json::from_str(&reply).expect("valid json reply");

    assert_eq!(value["error"], "safe message");
}

#[test]
fn error_result_renders_an_error_frame() {
    let reply = render_reply(
        "nope",
        Some(1),
        Err(WsDispatchError::NotFound("nope".to_string())),
    )
    .expect("a reply frame");
    let value: WsValue = serde_json::from_str(&reply).expect("valid json reply");

    assert_eq!(value["dest"], "nope");
    assert_eq!(value["id"], 1);
    assert_eq!(value["error"], "no handler for destination");
    assert!(value.get("ok").is_none());
}

#[test]
fn internal_dispatch_details_are_redacted_from_error_frames() {
    let reply = render_reply(
        "secure",
        Some(2),
        Err(WsDispatchError::Inject(
            "SecretProvider<DatabasePassword> failed".to_owned(),
        )),
    )
    .expect("a reply frame");
    let value: WsValue = serde_json::from_str(&reply).expect("valid json reply");

    assert_eq!(value["error"], "internal error");
    assert!(!reply.contains("SecretProvider"));
    assert!(!reply.contains("DatabasePassword"));
}

#[test]
fn inbound_frame_parses_dest_id_and_payload() {
    let inbound: Inbound =
        serde_json::from_str(r#"{"dest":"chat.send","id":9,"payload":{"text":"hi"}}"#)
            .expect("parse");

    assert_eq!(inbound.dest, "chat.send");
    assert_eq!(inbound.id, Some(9));
    assert_eq!(inbound.payload["text"], "hi");
}
