pub mod codec;

use serde::{Deserialize, Serialize};

use crate::frame::{CallId, CallResult};

/// Top-level wire message. Every frame on the wire is one of these.
///
/// Unary calls use only `Request`/`Response`. Streaming calls reuse the call's
/// `Request` as the opening frame and then exchange `StreamItem`/`StreamEnd`/
/// `StreamError` frames sharing the same `CallId`; `StreamCancel` lets a client
/// abort one in-flight call without dropping the whole connection. All frames
/// are additive, so unary peers are unaffected.
#[derive(Serialize, Deserialize)]
pub enum WireMessage {
    Request(WireRequest),
    Response(WireResponse),
    StreamItem { id: CallId, payload: Vec<u8> },
    StreamEnd { id: CallId },
    StreamError { id: CallId, message: String },
    StreamCancel { id: CallId },
}

/// The request half of the wire protocol, also the opening frame of a stream.
///
/// `streaming_input` is set by the client when the called method consumes a
/// request stream (client- or bidirectional-streaming). The transport cannot
/// infer this from the path alone, so the caller — which knows the method kind
/// — signals it, and the connection allocates an inbound channel only then.
#[derive(Serialize, Deserialize)]
pub struct WireRequest {
    pub id: CallId,
    pub path: String,
    pub payload: Vec<u8>,
    pub streaming_input: bool,
}

/// The response half of the wire protocol.
#[derive(Serialize, Deserialize)]
pub struct WireResponse {
    pub id: CallId,
    pub outcome: WireOutcome,
}

/// Success or failure at the wire level.
#[derive(Serialize, Deserialize)]
pub enum WireOutcome {
    Ok(Vec<u8>),
    Err(String),
}

impl WireResponse {
    /// Builds a wire response for `id` from a transport-level outcome.
    pub fn new(id: CallId, outcome: CallResult) -> Self {
        let outcome = match outcome {
            CallResult::Ok(bytes) => WireOutcome::Ok(bytes),
            CallResult::Err(msg) => WireOutcome::Err(msg),
        };

        Self { id, outcome }
    }
}
