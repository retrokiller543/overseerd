use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

/// Opaque identifier correlating a request to its response within a connection.
///
/// Assigned by the client and echoed back unchanged. The daemon never
/// interprets it; correlation lives entirely in the transport layer. For
/// streaming calls every frame of the call shares this id, which is how the
/// connection multiplexes concurrent streams over one ordered byte stream.
pub type CallId = u64;

/// An inbound RPC call received from the transport layer.
///
/// `payload` is the opening frame's body. For calls that stream their input
/// (client- and bidirectional-streaming), `requests` carries the subsequent
/// inbound items fed by the connection from `StreamItem`/`StreamEnd` frames;
/// it is `None` for unary and server-streaming calls. `cancel` fires when the
/// peer cancels the call or the connection drops.
pub struct IncomingCall {
    pub path: String,
    pub payload: Vec<u8>,
    pub requests: Option<mpsc::Receiver<Vec<u8>>>,
    pub cancel: CancellationToken,
}

/// Success or failure of an RPC call at the transport layer.
#[derive(Debug)]
pub enum CallResult {
    Ok(Vec<u8>),
    Err(String),
}

/// Transport-level information about the remote peer.
#[derive(Clone, Debug)]
pub struct PeerInfo {
    pub addr: Option<std::net::SocketAddr>,
}
