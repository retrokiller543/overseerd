#![cfg(unix)]

use std::{path::PathBuf, sync::Arc};

use tokio::{
    net::{
        UnixListener,
        unix::{OwnedReadHalf, OwnedWriteHalf},
    },
    sync::mpsc,
};
use tracing::{debug, instrument, trace, warn};

use crate::{
    error::{Error, Result},
    frame::{IncomingCall, OutgoingResponse, PeerInfo},
    protocol::{
        WireMessage, WireResponse,
        codec::{read_message, write_message},
    },
    transport::{Connection, Respond, Transport},
};

/// Unix socket transport. Removes the socket file on drop.
pub struct UnixTransport {
    listener: Arc<UnixListener>,
    path: PathBuf,
}

impl UnixTransport {
    pub fn bind(path: impl Into<PathBuf>) -> Result<Self> {
        let path = path.into();
        let listener = UnixListener::bind(&path)?;

        debug!(path = %path.display(), "Unix transport bound");

        Ok(Self {
            listener: Arc::new(listener),
            path,
        })
    }
}

impl Drop for UnixTransport {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
        debug!(path = %self.path.display(), "Unix socket removed");
    }
}

impl Transport for UnixTransport {
    type Connection = UnixConnection;

    #[instrument(skip_all)]
    async fn accept(&mut self) -> Result<UnixConnection> {
        trace!("waiting for Unix connection");

        let listener = Arc::clone(&self.listener);
        let (stream, _) = listener.accept().await?;

        debug!("Unix connection accepted");

        let (read, write) = stream.into_split();
        let (write_tx, write_rx) = mpsc::channel(1);

        write_tx.try_send(write).expect("channel is empty at construction");

        Ok(UnixConnection {
            read,
            write_tx,
            write_rx,
            peer: PeerInfo { addr: None },
        })
    }
}

/// An accepted Unix connection. Sequential: respond before calling recv again.
pub struct UnixConnection {
    read: OwnedReadHalf,
    write_tx: mpsc::Sender<OwnedWriteHalf>,
    write_rx: mpsc::Receiver<OwnedWriteHalf>,
    peer: PeerInfo,
}

/// Owns the write half for the duration of one call. No lock — single owner.
pub struct UnixResponder {
    write: OwnedWriteHalf,
    write_tx: mpsc::Sender<OwnedWriteHalf>,
}

impl Connection for UnixConnection {
    type Responder = UnixResponder;

    fn peer(&self) -> &PeerInfo {
        &self.peer
    }

    #[instrument(skip_all, fields(id = tracing::field::Empty, path = tracing::field::Empty))]
    async fn recv(&mut self) -> Result<Option<(IncomingCall, UnixResponder)>> {
        trace!("acquiring write token");

        let write = self.write_rx.recv().await.ok_or(Error::Closed)?;

        trace!("reading frame from peer");

        match read_message(&mut self.read).await {
            Ok(WireMessage::Request(req)) => {
                tracing::Span::current()
                    .record("id", req.id)
                    .record("path", tracing::field::display(&req.path));

                debug!("call received");

                let call = IncomingCall::from(req);
                let responder = UnixResponder {
                    write,
                    write_tx: self.write_tx.clone(),
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

impl Respond for UnixResponder {
    #[instrument(skip_all, fields(id = response.id))]
    async fn respond(mut self, response: OutgoingResponse) -> Result<()> {
        trace!("writing response");

        let msg = WireMessage::Response(WireResponse::from(response));

        write_message(&mut self.write, &msg).await?;

        trace!("response written, returning write token");

        let _ = self.write_tx.send(self.write).await;

        Ok(())
    }
}

fn is_disconnect(e: &std::io::Error) -> bool {
    matches!(
        e.kind(),
        std::io::ErrorKind::UnexpectedEof
            | std::io::ErrorKind::ConnectionReset
            | std::io::ErrorKind::ConnectionAborted
            | std::io::ErrorKind::BrokenPipe
    )
}
