use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::{debug, instrument, trace};

use crate::{
    error::{Error, Result},
    frame::{CallResult, IncomingCall, PeerInfo},
    status::StatusCode,
    transport::{Connection, Respond, RespondStream, ResponseSink, Transport},
};

/// Default server→client event buffer per call.
const EVENTS_BUFFER: usize = 64;

/// A server→client event for one call. Unary calls produce a single
/// `Response`; streaming calls produce `Item`s terminated by `End` or `Error`.
#[derive(Debug)]
pub enum ServerEvent {
    Response(CallResult),
    Item(Vec<u8>),
    End,
    Error { code: StatusCode, body: Vec<u8> },
}

/// One inbound call frame handed to the daemon side, carrying the call's
/// private channels so the in-memory transport needs no wire-level demux.
struct Frame {
    path: String,
    payload: Vec<u8>,
    requests: Option<mpsc::Receiver<Vec<u8>>>,
    events_tx: mpsc::Sender<ServerEvent>,
    cancel: CancellationToken,
}

/// A logical in-memory connection. Yielded by `MemoryTransport::accept`.
pub struct MemoryConnection {
    receiver: mpsc::Receiver<Frame>,
    peer: PeerInfo,
}

/// Sends responses back through the call's event channel (unary or streaming).
pub struct MemoryResponder {
    events_tx: mpsc::Sender<ServerEvent>,
}

/// The streaming sink half of a [`MemoryResponder`].
pub struct MemorySink {
    events_tx: mpsc::Sender<ServerEvent>,
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

/// A client-side handle to one in-flight call, covering all four kinds: send
/// inbound items (client/bidi), receive server events, and cancel.
pub struct MemoryCall {
    inbound: Option<mpsc::Sender<Vec<u8>>>,
    events: mpsc::Receiver<ServerEvent>,
    cancel: CancellationToken,
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
    /// Opens a call with the default event buffer.
    pub async fn open(
        &self,
        path: impl AsRef<str>,
        payload: Vec<u8>,
        streaming_input: bool,
    ) -> Result<MemoryCall> {
        self.open_with_capacity(path, payload, streaming_input, EVENTS_BUFFER)
            .await
    }

    /// Opens a call, sizing the server→client event buffer. A small capacity
    /// exercises outbound backpressure (the server's `send` awaits when full).
    #[instrument(level = "trace", skip(self, payload), fields(path = %path.as_ref()))]
    pub async fn open_with_capacity(
        &self,
        path: impl AsRef<str>,
        payload: Vec<u8>,
        streaming_input: bool,
        events_capacity: usize,
    ) -> Result<MemoryCall> {
        let (events_tx, events_rx) = mpsc::channel(events_capacity);
        let cancel = CancellationToken::new();

        let (inbound, requests) = if streaming_input {
            let (tx, rx) = mpsc::channel(32);

            (Some(tx), Some(rx))
        } else {
            (None, None)
        };

        let frame = Frame {
            path: path.as_ref().to_string(),
            payload,
            requests,
            events_tx,
            cancel: cancel.clone(),
        };

        trace!("sending memory call");

        self.request_tx
            .send(frame)
            .await
            .map_err(|_| Error::Closed)?;

        Ok(MemoryCall {
            inbound,
            events: events_rx,
            cancel,
        })
    }

    /// Sends a unary call and waits for its single response.
    pub async fn call(&self, path: impl AsRef<str>, payload: Vec<u8>) -> Result<CallResult> {
        let mut call = self.open(path, payload, false).await?;

        call.response().await
    }
}

impl MemoryCall {
    /// Sends one inbound item (client/bidi streaming). Errors if the call was
    /// not opened with `streaming_input`.
    pub async fn send(&self, item: Vec<u8>) -> Result<()> {
        let tx = self.inbound.as_ref().ok_or(Error::Closed)?;

        tx.send(item).await.map_err(|_| Error::Closed)
    }

    /// Half-closes the inbound stream, ending the server's request stream.
    pub fn end_input(&mut self) {
        self.inbound = None;
    }

    /// Receives the next server event, or `None` once the call is complete.
    pub async fn recv(&mut self) -> Option<ServerEvent> {
        self.events.recv().await
    }

    /// Awaits the single unary response event.
    pub async fn response(&mut self) -> Result<CallResult> {
        match self.events.recv().await {
            Some(ServerEvent::Response(outcome)) => Ok(outcome),
            Some(ServerEvent::Error { code, body }) => Ok(CallResult::Err { code, body }),
            _ => Err(Error::Closed),
        }
    }

    /// Cancels this call, firing the handler's cancellation token.
    pub fn cancel(&self) {
        self.cancel.cancel();
    }
}

impl Connection for MemoryConnection {
    type Responder = MemoryResponder;

    fn peer(&self) -> &PeerInfo {
        &self.peer
    }

    #[instrument(level = "trace", skip_all, fields(path = tracing::field::Empty))]
    async fn recv(&mut self) -> Result<Option<(IncomingCall, MemoryResponder)>> {
        let Some(frame) = self.receiver.recv().await else {
            debug!("memory connection closed by client");

            return Ok(None);
        };

        tracing::Span::current().record("path", tracing::field::display(&frame.path));

        trace!("memory call received");

        let call = IncomingCall {
            path: frame.path,
            payload: frame.payload,
            requests: frame.requests,
            cancel: frame.cancel,
        };
        let responder = MemoryResponder {
            events_tx: frame.events_tx,
        };

        Ok(Some((call, responder)))
    }
}

impl Respond for MemoryResponder {
    #[instrument(level = "trace", skip_all)]
    async fn respond(self, outcome: CallResult) -> Result<()> {
        trace!("sending memory response");

        self.events_tx
            .send(ServerEvent::Response(outcome))
            .await
            .map_err(|_| Error::Closed)
    }
}

impl RespondStream for MemoryResponder {
    type Sink = MemorySink;

    fn into_sink(self) -> MemorySink {
        MemorySink {
            events_tx: self.events_tx,
        }
    }
}

impl ResponseSink for MemorySink {
    async fn send(&mut self, item: Vec<u8>) -> Result<()> {
        self.events_tx
            .send(ServerEvent::Item(item))
            .await
            .map_err(|_| Error::Closed)
    }

    async fn error(self, code: StatusCode, body: Vec<u8>) -> Result<()> {
        self.events_tx
            .send(ServerEvent::Error { code, body })
            .await
            .map_err(|_| Error::Closed)
    }

    async fn finish(self) -> Result<()> {
        self.events_tx
            .send(ServerEvent::End)
            .await
            .map_err(|_| Error::Closed)
    }
}

impl Transport for MemoryTransport {
    type Connection = MemoryConnection;

    #[instrument(level = "debug", skip_all)]
    async fn accept(&mut self) -> Result<MemoryConnection> {
        trace!("waiting for memory connection");

        let conn = self.conn_rx.recv().await.ok_or(Error::Closed)?;

        debug!("memory connection accepted");

        Ok(conn)
    }
}
