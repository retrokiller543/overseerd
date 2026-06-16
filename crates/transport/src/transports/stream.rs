use std::sync::Arc;

use tokio::{
    io::{AsyncRead, AsyncWrite},
    sync::Mutex,
};
use tracing::{debug, instrument, trace, warn};

use crate::{
    error::{Error, Result},
    frame::{CallId, CallResult, IncomingCall, PeerInfo},
    protocol::{
        WireMessage, WireResponse,
        codec::{read_message, write_message},
    },
    transport::{Connection, Respond},
};

/// A connection over any reliable, ordered byte stream (TCP, Unix sockets).
///
/// The write half is shared with each responder behind a mutex so the
/// responder can outlive `recv`. The lock is held only for the duration of one
/// response write and is uncontended while calls are answered sequentially.
pub struct StreamConnection<R, W> {
    read: R,
    write: Arc<Mutex<W>>,
    peer: PeerInfo,
}

/// Responds to one inbound call on a stream connection. Owns the call's wire
/// id and a shared handle to the connection's write half.
pub struct StreamResponder<W> {
    write: Arc<Mutex<W>>,
    id: CallId,
}

impl<R, W> StreamConnection<R, W> {
    pub fn new(read: R, write: W, peer: PeerInfo) -> Self {
        Self {
            read,
            write: Arc::new(Mutex::new(write)),
            peer,
        }
    }
}

impl<R, W> Connection for StreamConnection<R, W>
where
    R: AsyncRead + Unpin + Send + 'static,
    W: AsyncWrite + Unpin + Send + 'static,
{
    type Responder = StreamResponder<W>;

    fn peer(&self) -> &PeerInfo {
        &self.peer
    }

    #[instrument(skip_all, fields(id = tracing::field::Empty, path = tracing::field::Empty))]
    async fn recv(&mut self) -> Result<Option<(IncomingCall, StreamResponder<W>)>> {
        trace!("reading frame from peer");

        match read_message(&mut self.read).await {
            Ok(WireMessage::Request(req)) => {
                let id = req.id;

                tracing::Span::current()
                    .record("id", id)
                    .record("path", tracing::field::display(&req.path));

                debug!("call received");

                let call = IncomingCall::from(req);
                let responder = StreamResponder {
                    write: Arc::clone(&self.write),
                    id,
                };

                Ok(Some((call, responder)))
            }

            Err(Error::Io(e)) if is_disconnect(&e) => {
                debug!(error = %e, "peer disconnected");
                Ok(None)
            }

            Ok(_) => {
                warn!("unexpected message type from peer");
                Err(Error::UnexpectedMessage)
            }

            Err(e) => {
                warn!(error = %e, "frame read error");
                Err(e)
            }
        }
    }
}

impl<W> Respond for StreamResponder<W>
where
    W: AsyncWrite + Unpin + Send + 'static,
{
    #[instrument(skip_all, fields(id = self.id))]
    async fn respond(self, outcome: CallResult) -> Result<()> {
        trace!("writing response");

        let msg = WireMessage::Response(WireResponse::new(self.id, outcome));
        let mut write = self.write.lock().await;

        write_message(&mut *write, &msg).await?;

        trace!("response written");

        Ok(())
    }
}

/// Distinguishes an orderly peer disconnect from a genuine I/O failure.
fn is_disconnect(e: &std::io::Error) -> bool {
    matches!(
        e.kind(),
        std::io::ErrorKind::UnexpectedEof
            | std::io::ErrorKind::ConnectionReset
            | std::io::ErrorKind::ConnectionAborted
            | std::io::ErrorKind::BrokenPipe
    )
}
