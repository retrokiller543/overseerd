use overseerd_transport::{CodecError, Decodes};

use super::ErrorBody;

struct Decimal;

impl Decodes<u64> for Decimal {
    fn decode(&self, body: Vec<u8>) -> Result<u64, CodecError> {
        std::str::from_utf8(&body)
            .map_err(|error| CodecError::bad_input(error.to_string()))?
            .parse()
            .map_err(|error: std::num::ParseIntError| CodecError::bad_input(error.to_string()))
    }
}

#[test]
fn typed_error_body_uses_the_protocol_decoder() {
    let body = ErrorBody::<_, u64>::new(409_u16, b"42".to_vec());

    assert_eq!(body.deserialize(&Decimal).expect("decimal body"), 42);
    assert_eq!(body.code(), 409);
    assert_eq!(body.raw(), b"42");
}

#[test]
fn failed_decode_preserves_status_and_raw_body() {
    let body = ErrorBody::<_, u64>::new(422_u16, b"not-a-number".to_vec());

    assert!(body.deserialize(&Decimal).is_err());
    assert_eq!(body.code(), 422);
    assert_eq!(body.raw(), b"not-a-number");
}

#[test]
fn consuming_decode_avoids_changing_the_codec_contract() {
    let body = ErrorBody::<_, u64>::new(500_u16, b"7".to_vec());

    assert_eq!(body.into_deserialized(&Decimal).expect("decimal body"), 7);
}
