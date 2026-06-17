use std::net::ToSocketAddrs;

use tokio::net::{
    TcpListener,
    tcp::{OwnedReadHalf, OwnedWriteHalf},
};
use tracing::{debug, instrument, trace};

use crate::{
    error::{Error, Result},
    frame::PeerInfo,
    transport::Transport,
    transports::stream::{StreamConnection, StreamResponder},
};

/// TCP transport. Each accepted connection handles calls sequentially.
pub struct TcpTransport {
    listener: TcpListener,
}

/// An accepted TCP connection.
pub type TcpConnection = StreamConnection<OwnedReadHalf, OwnedWriteHalf>;

/// Responds to one inbound call on a TCP connection.
pub type TcpResponder = StreamResponder<OwnedWriteHalf>;

impl TcpTransport {
    pub async fn bind(addr: impl ToSocketAddrs) -> Result<Self> {
        let addr = addr
            .to_socket_addrs()
            .map_err(Error::Io)?
            .next()
            .ok_or_else(|| Error::Io(std::io::Error::other("no address resolved")))?;

        let listener = TcpListener::bind(addr).await?;

        debug!(%addr, "TCP transport bound");

        Ok(Self { listener })
    }

    pub fn local_addr(&self) -> Result<std::net::SocketAddr> {
        self.listener.local_addr().map_err(Error::Io)
    }
}

impl Transport for TcpTransport {
    type Connection = TcpConnection;

    #[instrument(level = "debug", skip_all, fields(peer = tracing::field::Empty))]
    async fn accept(&mut self) -> Result<TcpConnection> {
        trace!("waiting for TCP connection");

        let (stream, peer_addr) = self.listener.accept().await?;

        tracing::Span::current().record("peer", tracing::field::display(peer_addr));
        debug!("TCP connection accepted");

        let (read, write) = stream.into_split();
        let peer = PeerInfo {
            addr: Some(peer_addr),
        };

        Ok(StreamConnection::new(read, write, peer))
    }
}
