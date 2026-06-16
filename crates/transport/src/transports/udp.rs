use std::{
    collections::HashMap,
    net::{SocketAddr, ToSocketAddrs},
    sync::Arc,
    time::{Duration, Instant},
};

use tokio::{net::UdpSocket, sync::mpsc, task::JoinHandle};
use tracing::{debug, instrument, trace, warn};

use crate::{
    error::{Error, Result},
    frame::{IncomingCall, OutgoingResponse, PeerInfo},
    protocol::{
        WireMessage, WireResponse,
        codec::{decode, encode},
    },
    transport::{Connection, Respond, Transport},
};

/// Maximum UDP payload (65535 - 20 IP header - 8 UDP header).
const MAX_DATAGRAM: usize = 65507;

/// How long a session can be idle before the router closes it.
const SESSION_IDLE_TIMEOUT: Duration = Duration::from_secs(30);

/// UDP transport with per-peer session demultiplexing.
///
/// A background router task reads all datagrams and routes them to the
/// correct `UdpConnection` by peer address. Sessions that receive no
/// datagram for `SESSION_IDLE_TIMEOUT` are closed automatically.
pub struct UdpTransport {
    conn_rx: mpsc::Receiver<UdpConnection>,
    router: JoinHandle<()>,
}

impl UdpTransport {
    pub async fn bind(addr: impl ToSocketAddrs) -> Result<Self> {
        let addr = addr
            .to_socket_addrs()
            .map_err(Error::Io)?
            .next()
            .ok_or_else(|| Error::Io(std::io::Error::other("no address resolved")))?;

        let socket = Arc::new(UdpSocket::bind(addr).await?);

        debug!(%addr, "UDP transport bound");

        let (conn_tx, conn_rx) = mpsc::channel(16);
        let router = tokio::spawn(run_router(Arc::clone(&socket), conn_tx));

        Ok(Self { conn_rx, router })
    }
}

impl Drop for UdpTransport {
    fn drop(&mut self) {
        self.router.abort();
    }
}

impl Transport for UdpTransport {
    type Connection = UdpConnection;

    #[instrument(skip_all)]
    async fn accept(&mut self) -> Result<UdpConnection> {
        trace!("waiting for new UDP session");

        let conn = self.conn_rx.recv().await.ok_or(Error::Closed)?;

        debug!(peer = ?conn.peer.addr, "UDP session started");

        Ok(conn)
    }
}

/// A logical UDP session scoped to one remote peer address.
///
/// Multiple datagrams from the same peer are delivered sequentially.
/// The session closes when the router idles it out or the transport is dropped.
pub struct UdpConnection {
    socket: Arc<UdpSocket>,
    call_rx: mpsc::Receiver<IncomingCall>,
    peer: PeerInfo,
}

/// Sends a response datagram back to the peer. No lock needed — `send_to`
/// takes `&self` and the OS serialises concurrent sends on the same fd.
pub struct UdpResponder {
    socket: Arc<UdpSocket>,
    peer_addr: SocketAddr,
}

impl Connection for UdpConnection {
    type Responder = UdpResponder;

    fn peer(&self) -> &PeerInfo {
        &self.peer
    }

    #[instrument(skip_all, fields(id = tracing::field::Empty, path = tracing::field::Empty))]
    async fn recv(&mut self) -> Result<Option<(IncomingCall, UdpResponder)>> {
        trace!("waiting for UDP call");

        let Some(call) = self.call_rx.recv().await else {
            debug!("UDP session closed");
            return Ok(None);
        };

        let peer_addr = self.peer.addr.expect("UdpConnection always has a peer address");

        tracing::Span::current()
            .record("id", call.id)
            .record("path", tracing::field::display(&call.path));

        debug!("UDP call received");

        let responder = UdpResponder {
            socket: Arc::clone(&self.socket),
            peer_addr,
        };

        Ok(Some((call, responder)))
    }
}

impl Respond for UdpResponder {
    #[instrument(skip_all, fields(id = response.id, peer = %self.peer_addr))]
    async fn respond(self, response: OutgoingResponse) -> Result<()> {
        trace!("sending UDP response");

        let msg = WireMessage::Response(WireResponse::from(response));
        let bytes = encode(&msg)?;

        self.socket.send_to(&bytes, self.peer_addr).await?;

        trace!("UDP response sent");

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

struct SessionEntry {
    call_tx: mpsc::Sender<IncomingCall>,
    last_seen: Instant,
}

/// Reads all datagrams from the socket and routes them to per-peer channels.
///
/// New peers create a fresh `UdpConnection` sent to the transport's accept queue.
/// Existing peers have their datagram forwarded to the open channel.
/// Sessions idle for longer than `SESSION_IDLE_TIMEOUT` are evicted: dropping
/// `call_tx` closes the channel, which makes `UdpConnection::recv` return
/// `Ok(None)` and exits `serve_connection` cleanly.
async fn run_router(socket: Arc<UdpSocket>, conn_tx: mpsc::Sender<UdpConnection>) {
    let mut sessions: HashMap<SocketAddr, SessionEntry> = HashMap::new();
    let mut buf = vec![0u8; MAX_DATAGRAM];

    // First cleanup fires after a full interval, not immediately.
    let cleanup_start = tokio::time::Instant::now() + SESSION_IDLE_TIMEOUT;
    let mut cleanup = tokio::time::interval_at(cleanup_start, SESSION_IDLE_TIMEOUT);

    loop {
        tokio::select! {
            result = socket.recv_from(&mut buf) => {
                let (len, peer_addr) = match result {
                    Ok(r) => r,
                    Err(e) => {
                        warn!(error = %e, "UDP router socket error");
                        break;
                    }
                };

                let msg = match decode(&buf[..len]) {
                    Ok(m) => m,
                    Err(e) => {
                        warn!(peer = %peer_addr, error = %e, "UDP decode error, dropping datagram");
                        continue;
                    }
                };

                let WireMessage::Request(req) = msg else {
                    warn!(peer = %peer_addr, "unexpected non-request UDP datagram, dropping");
                    continue;
                };

                let call = IncomingCall::from(req);

                if let Some(entry) = sessions.get_mut(&peer_addr) {
                    entry.last_seen = Instant::now();

                    if entry.call_tx.send(call).await.is_err() {
                        sessions.remove(&peer_addr);
                    }
                } else {
                    let (call_tx, call_rx) = mpsc::channel(16);

                    if call_tx.send(call).await.is_err() {
                        continue;
                    }

                    let conn = UdpConnection {
                        socket: Arc::clone(&socket),
                        call_rx,
                        peer: PeerInfo { addr: Some(peer_addr) },
                    };

                    if conn_tx.send(conn).await.is_err() {
                        break;
                    }

                    sessions.insert(peer_addr, SessionEntry {
                        call_tx,
                        last_seen: Instant::now(),
                    });

                    debug!(peer = %peer_addr, "new UDP session created");
                }
            }

            _ = cleanup.tick() => {
                let now = Instant::now();

                sessions.retain(|peer, entry| {
                    if entry.call_tx.is_closed()
                        || now.duration_since(entry.last_seen) >= SESSION_IDLE_TIMEOUT
                    {
                        debug!(
                            %peer,
                            idle_secs = now.duration_since(entry.last_seen).as_secs(),
                            "UDP session evicted"
                        );
                        false
                    } else {
                        true
                    }
                });
            }
        }
    }

    debug!("UDP router stopped");
}
