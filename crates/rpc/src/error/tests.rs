use overseerd_transport::PredefinedCode;

use super::Error;
use crate::extract::ResponseError;

fn response_message(error: Error) -> (PredefinedCode, String) {
    let response = error.error_response();
    let message = postcard::from_bytes(&response.body).expect("decode public error body");

    (response.code.predefined(), message)
}

#[test]
fn redacts_internal_error_details() {
    let secret = "/private/config/production.toml";
    let (code, message) = response_message(Error::Transport(overseerd_transport::Error::Io(
        std::io::Error::other(secret),
    )));

    assert_eq!(code, PredefinedCode::Internal);
    assert_eq!(message, "internal server error");
    assert!(!message.contains(secret));
}

#[test]
fn invalid_payload_does_not_expose_decoder_details() {
    let (code, message) = response_message(Error::InvalidPayload(
        "Hit the end of buffer, expected more data".to_string(),
    ));

    assert_eq!(code, PredefinedCode::BadInput);
    assert_eq!(message, "invalid request payload");
}

#[test]
fn route_not_found_does_not_echo_attacker_path() {
    let (code, message) = response_message(Error::RouteNotFound(
        "/probe/internal-service-name".to_string(),
    ));

    assert_eq!(code, PredefinedCode::NotFound);
    assert_eq!(message, "route not found");
}
