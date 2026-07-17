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
    transports::stream::{StreamConfig, StreamConnection, StreamResponder},
};

/// TCP transport.
pub struct TcpTransport {
    listener: TcpListener,
    config: StreamConfig,
}

/// An accepted TCP connection.
pub type TcpConnection = StreamConnection<OwnedReadHalf, OwnedWriteHalf>;

/// Responds to one inbound call on a TCP connection.
pub type TcpResponder = StreamResponder<OwnedWriteHalf>;

impl TcpTransport {
    pub async fn bind(addr: impl ToSocketAddrs) -> Result<Self> {
        Self::bind_with_config(addr, StreamConfig::default()).await
    }

    pub async fn bind_with_config(addr: impl ToSocketAddrs, config: StreamConfig) -> Result<Self> {
        let addr = addr
            .to_socket_addrs()
            .map_err(Error::Io)?
            .next()
            .ok_or_else(|| Error::Io(std::io::Error::other("no address resolved")))?;

        let listener = TcpListener::bind(addr).await?;

        debug!(%addr, "TCP transport bound");

        Ok(Self { listener, config })
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

        Ok(StreamConnection::with_config(
            read,
            write,
            peer,
            self.config,
        ))
    }
}
