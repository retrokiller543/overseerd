use tokio::sync::{mpsc, oneshot};
use tracing::{debug, instrument, trace};

use crate::{
    error::{Error, Result},
    frame::{CallResult, IncomingCall, PeerInfo},
    transport::{Connection, Respond, Transport},
};

struct Frame {
    call: IncomingCall,
    response_tx: oneshot::Sender<CallResult>,
}

/// A logical in-memory connection. Yielded by `MemoryTransport::accept`.
pub struct MemoryConnection {
    receiver: mpsc::Receiver<Frame>,
    peer: PeerInfo,
}

/// Sends a response back through the oneshot for one specific call.
pub struct MemoryResponder {
    response_tx: oneshot::Sender<CallResult>,
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

        Ok(MemoryConnectionHandle { request_tx })
    }
}

impl MemoryConnectionHandle {
    /// Sends a call and waits for the response. The oneshot reply channel
    /// correlates the response to this call, so no wire id is needed.
    #[instrument(skip(self, payload), fields(path = %path.as_ref()))]
    pub async fn call(&self, path: impl AsRef<str>, payload: Vec<u8>) -> Result<CallResult> {
        let (response_tx, response_rx) = oneshot::channel();
        let frame = Frame {
            call: IncomingCall {
                path: path.as_ref().to_string(),
                payload,
            },
            response_tx,
        };

        trace!("sending memory call");

        self.request_tx
            .send(frame)
            .await
            .map_err(|_| Error::Closed)?;

        let result = response_rx.await.map_err(|_| Error::Closed);

        trace!("memory call completed");

        result
    }
}

impl Connection for MemoryConnection {
    type Responder = MemoryResponder;

    fn peer(&self) -> &PeerInfo {
        &self.peer
    }

    #[instrument(skip_all, fields(path = tracing::field::Empty))]
    async fn recv(&mut self) -> Result<Option<(IncomingCall, MemoryResponder)>> {
        let Some(frame) = self.receiver.recv().await else {
            debug!("memory connection closed by client");
            return Ok(None);
        };

        tracing::Span::current().record("path", tracing::field::display(&frame.call.path));

        trace!("memory call received");

        let responder = MemoryResponder {
            response_tx: frame.response_tx,
        };

        Ok(Some((frame.call, responder)))
    }
}

impl Respond for MemoryResponder {
    #[instrument(skip_all)]
    async fn respond(self, outcome: CallResult) -> Result<()> {
        trace!("sending memory response");

        let _ = self.response_tx.send(outcome);

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
