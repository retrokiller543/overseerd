use std::{net::ToSocketAddrs, sync::Arc};

use tokio::{
    net::{
        TcpListener,
        tcp::{OwnedReadHalf, OwnedWriteHalf},
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

/// TCP transport. Each accepted connection handles calls sequentially.
pub struct TcpTransport {
    listener: Arc<TcpListener>,
}

impl TcpTransport {
    pub async fn bind(addr: impl ToSocketAddrs) -> Result<Self> {
        let addr = addr
            .to_socket_addrs()
            .map_err(Error::Io)?
            .next()
            .ok_or_else(|| Error::Io(std::io::Error::other("no address resolved")))?;

        let listener = TcpListener::bind(addr).await?;

        debug!(%addr, "TCP transport bound");

        Ok(Self {
            listener: Arc::new(listener),
        })
    }

    pub fn local_addr(&self) -> Result<std::net::SocketAddr> {
        self.listener.local_addr().map_err(Error::Io)
    }
}

impl Transport for TcpTransport {
    type Connection = TcpConnection;

    #[instrument(skip_all, fields(peer = tracing::field::Empty))]
    async fn accept(&mut self) -> Result<TcpConnection> {
        trace!("waiting for TCP connection");

        let listener = Arc::clone(&self.listener);
        let (stream, peer_addr) = listener.accept().await?;

        tracing::Span::current().record("peer", tracing::field::display(peer_addr));
        debug!("TCP connection accepted");

        let peer = PeerInfo { addr: Some(peer_addr) };
        let (read, write) = stream.into_split();
        let (write_tx, write_rx) = mpsc::channel(1);

        write_tx.try_send(write).expect("channel is empty at construction");

        Ok(TcpConnection { read, write_tx, write_rx, peer })
    }
}

/// An accepted TCP connection. Sequential: respond before calling recv again.
pub struct TcpConnection {
    read: OwnedReadHalf,
    write_tx: mpsc::Sender<OwnedWriteHalf>,
    write_rx: mpsc::Receiver<OwnedWriteHalf>,
    peer: PeerInfo,
}

/// Owns the write half for the duration of one call. No lock — single owner.
pub struct TcpResponder {
    write: OwnedWriteHalf,
    write_tx: mpsc::Sender<OwnedWriteHalf>,
}

impl Connection for TcpConnection {
    type Responder = TcpResponder;

    fn peer(&self) -> &PeerInfo {
        &self.peer
    }

    #[instrument(skip_all, fields(id = tracing::field::Empty, path = tracing::field::Empty))]
    async fn recv(&mut self) -> Result<Option<(IncomingCall, TcpResponder)>> {
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
                let responder = TcpResponder {
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

impl Respond for TcpResponder {
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
