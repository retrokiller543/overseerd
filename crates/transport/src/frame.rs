/// Opaque identifier correlating a request to its response within a connection.
///
/// Assigned by the client and echoed back unchanged. The daemon never
/// interprets it; correlation lives entirely in the transport layer.
pub type CallId = u64;

/// An inbound RPC call received from the transport layer.
pub struct IncomingCall {
    pub path: String,
    pub payload: Vec<u8>,
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
