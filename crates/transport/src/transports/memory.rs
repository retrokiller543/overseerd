use std::sync::atomic::{AtomicU64, Ordering};

use tokio::sync::{mpsc, oneshot};
use tracing::{debug, instrument, trace};

use crate::{
    error::{Error, Result},
    frame::{CallId, IncomingCall, OutgoingResponse, PeerInfo},
    transport::{Connection, Respond, Transport},
};

struct Frame {
    call: IncomingCall,
    response_tx: oneshot::Sender<OutgoingResponse>,
}

/// A logical in-memory connection. Yielded by `MemoryTransport::accept`.
pub struct MemoryConnection {
    receiver: mpsc::Receiver<Frame>,
    peer: PeerInfo,
}

/// Sends a response back through the oneshot for one specific call.
pub struct MemoryResponder {
    response_tx: oneshot::Sender<OutgoingResponse>,
}

/// The daemon side of an in-memory transport. Yields one `MemoryConnection`
/// per `MemoryClient::connect` call.
pub struct MemoryTransport {
    conn_rx: mpsc::Receiver<MemoryConnection>,
}

/// The client side of an in-memory transport. Primarily used in tests.
pub struct MemoryClient {
    conn_tx: mpsc::Sender<MemoryConnection>,
}

/// A handle representing one logical connection on the client side.
/// Dropped when the client is done with the connection.
pub struct MemoryConnectionHandle {
    request_tx: mpsc::Sender<Frame>,
    next_id: AtomicU64,
}

impl MemoryClient {
    /// Creates a paired client and transport.
    pub fn pair() -> (MemoryClient, MemoryTransport) {
        let (conn_tx, conn_rx) = mpsc::channel(16);

        (MemoryClient { conn_tx }, MemoryTransport { conn_rx })
    }

    /// Opens a new logical connection to the daemon.
    pub async fn connect(&self) -> Result<MemoryConnectionHandle> {
        let (request_tx, request_rx) = mpsc::channel(64);
        let conn = MemoryConnection {
            receiver: request_rx,
            peer: PeerInfo { addr: None },
        };

        self.conn_tx.send(conn).await.map_err(|_| Error::Closed)?;

        debug!("memory connection opened");

        Ok(MemoryConnectionHandle {
            request_tx,
            next_id: AtomicU64::new(0),
        })
    }
}

impl MemoryConnectionHandle {
    /// Sends a call and waits for the response.
    #[instrument(skip(self, payload), fields(path = %path.as_ref()))]
    pub async fn call(&self, path: impl AsRef<str>, payload: Vec<u8>) -> Result<OutgoingResponse> {
        let (response_tx, response_rx) = oneshot::channel();
        let id: CallId = self.next_id.fetch_add(1, Ordering::Relaxed);

        trace!(id, "sending memory call");

        let frame = Frame {
            call: IncomingCall {
                id,
                path: path.as_ref().to_string(),
                payload,
            },
            response_tx,
        };

        self.request_tx.send(frame).await.map_err(|_| Error::Closed)?;

        let result = response_rx.await.map_err(|_| Error::Closed);

        trace!(id, "memory call completed");

        result
    }
}

impl Connection for MemoryConnection {
    type Responder = MemoryResponder;

    fn peer(&self) -> &PeerInfo {
        &self.peer
    }

    #[instrument(skip_all, fields(id = tracing::field::Empty, path = tracing::field::Empty))]
    async fn recv(&mut self) -> Result<Option<(IncomingCall, MemoryResponder)>> {
        let Some(frame) = self.receiver.recv().await else {
            debug!("memory connection closed by client");
            return Ok(None);
        };

        tracing::Span::current()
            .record("id", frame.call.id)
            .record("path", tracing::field::display(&frame.call.path));

        trace!("memory call received");

        let responder = MemoryResponder {
            response_tx: frame.response_tx,
        };

        Ok(Some((frame.call, responder)))
    }
}

impl Respond for MemoryResponder {
    #[instrument(skip_all, fields(id = response.id))]
    async fn respond(self, response: OutgoingResponse) -> Result<()> {
        trace!("sending memory response");

        let _ = self.response_tx.send(response);

        Ok(())
    }
}

impl Transport for MemoryTransport {
    type Connection = MemoryConnection;

    #[instrument(skip_all)]
    async fn accept(&mut self) -> Result<MemoryConnection> {
        trace!("waiting for memory connection");

        let conn = self.conn_rx.recv().await.ok_or(Error::Closed)?;

        debug!("memory connection accepted");

        Ok(conn)
    }
}
