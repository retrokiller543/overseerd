/// Opaque identifier correlating a request to its response within a connection.
pub type CallId = u64;

/// An inbound RPC call received from the transport layer.
pub struct IncomingCall {
    pub id: CallId,
    pub path: String,
    pub payload: Vec<u8>,
}

/// The response to send back for an inbound call.
pub struct OutgoingResponse {
    pub id: CallId,
    pub outcome: CallResult,
}

/// Success or failure of an RPC call at the transport layer.
pub enum CallResult {
    Ok(Vec<u8>),
    Err(String),
}

/// Transport-level information about the remote peer.
#[derive(Clone, Debug)]
pub struct PeerInfo {
    pub addr: Option<std::net::SocketAddr>,
}
