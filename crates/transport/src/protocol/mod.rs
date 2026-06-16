pub mod codec;

use serde::{Deserialize, Serialize};

use crate::frame::{CallId, CallResult, IncomingCall, OutgoingResponse};

/// Top-level wire message. Every frame on the wire is one of these.
#[derive(Serialize, Deserialize)]
pub enum WireMessage {
    Request(WireRequest),
    Response(WireResponse),
}

/// The request half of the wire protocol.
#[derive(Serialize, Deserialize)]
pub struct WireRequest {
    pub id: CallId,
    pub path: String,
    pub payload: Vec<u8>,
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

impl From<WireRequest> for IncomingCall {
    fn from(req: WireRequest) -> Self {
        Self {
            id: req.id,
            path: req.path,
            payload: req.payload,
        }
    }
}

impl From<OutgoingResponse> for WireResponse {
    fn from(resp: OutgoingResponse) -> Self {
        let outcome = match resp.outcome {
            CallResult::Ok(bytes) => WireOutcome::Ok(bytes),
            CallResult::Err(msg) => WireOutcome::Err(msg),
        };

        Self {
            id: resp.id,
            outcome,
        }
    }
}
