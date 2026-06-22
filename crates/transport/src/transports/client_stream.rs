use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex};

use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::{Mutex, mpsc};
use tokio::task::JoinHandle;
use tracing::{debug, warn};

use crate::error::Error;
use crate::frame::CallId;
use crate::protocol::{
    WireMessage, WireRequest, WireResponse,
    codec::{read_message, write_message},
};

use super::client::{CallSink, CallSource, ClientCall, ClientError, ClientTransport, Reply};

/// Outbound frames buffered per call before the read loop backpressures.
const REPLY_BUFFER: usize = 32;

/// The per-call routing table: maps a `CallId` to the sender feeding that call's
/// reply channel. Shared between `open` (registration) and the read loop
/// (demuxing); a synchronous mutex so it can also be cleared from `Drop`.
type CallTable = Arc<StdMutex<HashMap<CallId, mpsc::Sender<Reply>>>>;

/// A [`ClientTransport`] over any reliable, ordered byte stream (TCP, Unix).
///
/// Mirrors the server-side `StreamConnection`: one background task owns the read
/// half and demuxes `Response`/`StreamItem`/`StreamEnd`/`StreamError` frames by
/// `CallId` into per-call channels, while the write half is shared behind a mutex
/// and locked only for a single frame write.
pub struct StreamClientTransport<W> {
    write: Arc<Mutex<W>>,
    next_id: AtomicU64,
    calls: CallTable,
    _read_task: JoinHandle<()>,
}

impl<W> StreamClientTransport<W>
where
    W: AsyncWrite + Unpin + Send + 'static,
{
    /// Splits ownership: `read` moves into the demux task, `write` stays shared.
    pub fn new<R>(read: R, write: W) -> Self
    where
        R: AsyncRead + Unpin + Send + 'static,
    {
        let calls: CallTable = Arc::new(StdMutex::new(HashMap::new()));
        let read_task = tokio::spawn(read_loop(read, Arc::clone(&calls)));

        Self {
            write: Arc::new(Mutex::new(write)),
            next_id: AtomicU64::new(1),
            calls,
            _read_task: read_task,
        }
    }
}

impl<W> Drop for StreamClientTransport<W> {
    fn drop(&mut self) {
        self._read_task.abort();
    }
}

impl<W> ClientTransport for StreamClientTransport<W>
where
    W: AsyncWrite + Unpin + Send + 'static,
{
    type Call = StreamCall<W>;

    async fn open(
        &self,
        path: &str,
        streaming_input: bool,
        payload: Vec<u8>,
    ) -> Result<Self::Call, ClientError> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = mpsc::channel(REPLY_BUFFER);
        let request = WireMessage::Request(WireRequest {
            id,
            path: path.to_string(),
            payload,
            streaming_input,
        });

        // Register before writing so a fast reply can't race an absent entry.
        self.calls.lock().unwrap().insert(id, tx);

        {
            let mut write = self.write.lock().await;

            if let Err(e) = write_message(&mut *write, &request).await {
                drop(write);
                self.calls.lock().unwrap().remove(&id);

                return Err(e.into());
            }
        }

        Ok(StreamCall {
            id,
            write: Arc::clone(&self.write),
            calls: Arc::clone(&self.calls),
            replies: rx,
        })
    }
}

/// One in-flight call on a byte-stream transport, split on creation into a
/// [`StreamCallSink`] (shared write half) and a [`StreamSource`] (reply receiver) so
/// the two directions run independently.
pub struct StreamCall<W> {
    id: CallId,
    write: Arc<Mutex<W>>,
    calls: CallTable,
    replies: mpsc::Receiver<Reply>,
}

impl<W> ClientCall for StreamCall<W>
where
    W: AsyncWrite + Unpin + Send + 'static,
{
    type Sink = StreamCallSink<W>;
    type Source = StreamSource;

    fn split(self) -> (StreamCallSink<W>, StreamSource) {
        (
            StreamCallSink {
                id: self.id,
                write: self.write,
                calls: Arc::clone(&self.calls),
            },
            StreamSource {
                id: self.id,
                replies: self.replies,
                calls: self.calls,
            },
        )
    }
}

/// The send half: writes inbound frames under the shared write lock.
pub struct StreamCallSink<W> {
    id: CallId,
    write: Arc<Mutex<W>>,
    calls: CallTable,
}

impl<W> StreamCallSink<W>
where
    W: AsyncWrite + Unpin + Send + 'static,
{
    /// Writes one frame under the shared write lock, held only for the write.
    async fn write_frame(&self, msg: &WireMessage) -> Result<(), ClientError> {
        let mut write = self.write.lock().await;

        write_message(&mut *write, msg).await.map_err(Into::into)
    }
}

impl<W> CallSink for StreamCallSink<W>
where
    W: AsyncWrite + Unpin + Send + 'static,
{
    async fn send(&mut self, payload: Vec<u8>) -> Result<(), ClientError> {
        self.write_frame(&WireMessage::StreamItem {
            id: self.id,
            payload,
        })
        .await
    }

    async fn finish(&mut self) -> Result<(), ClientError> {
        self.write_frame(&WireMessage::StreamEnd { id: self.id }).await
    }

    async fn cancel(self) -> Result<(), ClientError> {
        self.write_frame(&WireMessage::StreamCancel { id: self.id })
            .await?;

        self.calls.lock().unwrap().remove(&self.id);

        Ok(())
    }
}

/// The receive half: pulls demuxed replies, and on drop removes the call's entry
/// from the routing table so the read loop stops demuxing into a dead channel.
pub struct StreamSource {
    id: CallId,
    calls: CallTable,
    replies: mpsc::Receiver<Reply>,
}

impl CallSource for StreamSource {
    async fn recv(&mut self) -> Result<Option<Reply>, ClientError> {
        Ok(self.replies.recv().await)
    }
}

impl Drop for StreamSource {
    fn drop(&mut self) {
        if let Ok(mut calls) = self.calls.lock() {
            calls.remove(&self.id);
        }
    }
}

/// Demuxes inbound frames into per-call channels until the stream ends or errors.
/// On exit the call table is cleared, dropping every reply sender so outstanding
/// calls observe a closed channel and resolve to `ConnectionClosed`.
async fn read_loop<R>(mut read: R, calls: CallTable)
where
    R: AsyncRead + Unpin,
{
    loop {
        let message = read_message(&mut read).await;

        match message {
            Ok(WireMessage::Response(WireResponse { id, outcome })) => {
                let sender = calls.lock().unwrap().remove(&id);

                if let Some(tx) = sender {
                    let _ = tx.send(Reply::Response(outcome)).await;
                }
            }

            Ok(WireMessage::StreamItem { id, payload }) => {
                let sender = calls.lock().unwrap().get(&id).cloned();

                if let Some(tx) = sender {
                    let _ = tx.send(Reply::Item(payload)).await;
                }
            }

            Ok(WireMessage::StreamEnd { id }) => {
                let sender = calls.lock().unwrap().remove(&id);

                if let Some(tx) = sender {
                    let _ = tx.send(Reply::End).await;
                }
            }

            Ok(WireMessage::StreamError { id, code, body }) => {
                let sender = calls.lock().unwrap().remove(&id);

                if let Some(tx) = sender {
                    let _ = tx.send(Reply::Error { code, body }).await;
                }
            }

            Ok(WireMessage::Request(_)) | Ok(WireMessage::StreamCancel { .. }) => {
                warn!("unexpected server-bound message on client connection");

                break;
            }

            Err(Error::Io(e)) if is_disconnect(&e) => {
                debug!(error = %e, "server disconnected");

                break;
            }

            Err(e) => {
                warn!(error = %e, "client frame read error");

                break;
            }
        }
    }

    calls.lock().unwrap().clear();
}

/// Distinguishes an orderly server disconnect from a genuine I/O failure.
fn is_disconnect(e: &std::io::Error) -> bool {
    matches!(
        e.kind(),
        std::io::ErrorKind::UnexpectedEof
            | std::io::ErrorKind::ConnectionReset
            | std::io::ErrorKind::ConnectionAborted
            | std::io::ErrorKind::BrokenPipe
    )
}
