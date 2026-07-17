use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
use std::task::{Context, Poll};
use std::time::Duration;
use std::{io, pin::Pin};

use tokio::io::{AsyncWrite, AsyncWriteExt, duplex, split};
use tokio::time::timeout;

use super::{CallSlot, StreamConfig, StreamConnection};
use crate::error::Error;
use crate::frame::PeerInfo;
use crate::protocol::codec::{FrameConfig, read_message, write_message};
use crate::protocol::{WireMessage, WireRequest};
use crate::transport::Connection;

fn request(id: u64, streaming_input: bool) -> WireMessage {
    WireMessage::Request(WireRequest {
        id,
        path: format!("svc.call_{id}"),
        payload: Vec::new(),
        streaming_input,
    })
}

fn insert_test_slot<R, W>(connection: &mut StreamConnection<R, W>, id: u64) {
    connection.calls.insert(
        id,
        CallSlot {
            inbound: None,
            cancel: tokio_util::sync::CancellationToken::new(),
            active: Arc::new(AtomicBool::new(true)),
            tombstone: false,
        },
    );
}

struct PartialThenPendingWriter {
    bytes_written: Arc<AtomicUsize>,
}

impl AsyncWrite for PartialThenPendingWriter {
    fn poll_write(
        self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        if self.bytes_written.load(Ordering::Acquire) == 0 {
            let written = buf.len().min(2);
            self.bytes_written.store(written, Ordering::Release);

            Poll::Ready(Ok(written))
        } else {
            Poll::Pending
        }
    }

    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}

struct FailingWriter;

impl AsyncWrite for FailingWriter {
    fn poll_write(
        self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        _buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Poll::Ready(Err(io::Error::new(
            io::ErrorKind::BrokenPipe,
            "test writer failed",
        )))
    }

    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}

#[tokio::test]
async fn overflowing_input_terminates_only_that_call() {
    let (server, client) = duplex(64 * 1024);
    let (server_read, server_write) = split(server);
    let (mut client_read, mut client_write) = split(client);
    let mut connection = StreamConnection::new(server_read, server_write, PeerInfo { addr: None });

    write_message(&mut client_write, &request(1, true))
        .await
        .expect("open streaming call");
    let (first, _responder) = connection
        .recv()
        .await
        .expect("receive first request")
        .expect("first request present");

    for value in 0..=32_u8 {
        write_message(
            &mut client_write,
            &WireMessage::StreamItem {
                id: 1,
                payload: vec![value],
            },
        )
        .await
        .expect("write stream item");
    }

    write_message(&mut client_write, &request(2, false))
        .await
        .expect("open independent call");

    let (second, _responder) = timeout(Duration::from_secs(2), connection.recv())
        .await
        .expect("independent call must not be head-of-line blocked")
        .expect("connection remains healthy")
        .expect("second request present");

    assert_eq!(second.path, "svc.call_2");
    assert!(first.cancel.is_cancelled());

    match timeout(Duration::from_secs(2), read_message(&mut client_read))
        .await
        .expect("overflow response deadline")
        .expect("decode overflow response")
    {
        WireMessage::StreamError { id: 1, .. } => {}
        _ => panic!("expected stream error for overflowing call"),
    }
}

#[tokio::test]
async fn enforces_per_connection_call_limit() {
    let (server, client) = duplex(4096);
    let (server_read, server_write) = split(server);
    let (_client_read, mut client_write) = split(client);
    let config = StreamConfig::new(FrameConfig::default(), 1, Duration::from_millis(100));
    let mut connection =
        StreamConnection::with_config(server_read, server_write, PeerInfo { addr: None }, config);

    write_message(&mut client_write, &request(1, false))
        .await
        .expect("write first request");
    let (first, _responder) = connection
        .recv()
        .await
        .expect("receive first request")
        .expect("first request present");

    write_message(&mut client_write, &request(2, false))
        .await
        .expect("write second request");

    assert!(matches!(
        connection.recv().await,
        Err(Error::TooManyCalls { max: 1 })
    ));
    assert!(first.cancel.is_cancelled());
}

#[tokio::test]
async fn duplicate_call_id_cannot_replace_active_slot() {
    let (server, client) = duplex(4096);
    let (server_read, server_write) = split(server);
    let (_client_read, mut client_write) = split(client);
    let mut connection = StreamConnection::new(server_read, server_write, PeerInfo { addr: None });

    write_message(&mut client_write, &request(1, false))
        .await
        .expect("write first request");
    let (first, _responder) = connection
        .recv()
        .await
        .expect("receive first request")
        .expect("first request present");

    write_message(&mut client_write, &request(1, false))
        .await
        .expect("write duplicate request");

    assert!(matches!(
        connection.recv().await,
        Err(Error::DuplicateCallId { id: 1 })
    ));
    assert!(first.cancel.is_cancelled());
}

#[tokio::test]
async fn control_response_tasks_are_bounded_by_connection_limit() {
    let config = StreamConfig::new(FrameConfig::default(), 2, Duration::from_secs(10));
    let mut connection = StreamConnection::with_config(
        tokio::io::empty(),
        tokio::io::sink(),
        PeerInfo { addr: None },
        config,
    );
    let mut cancellations = Vec::new();
    let mut active_calls = Vec::new();

    for id in 1..=3 {
        let cancel = tokio_util::sync::CancellationToken::new();
        let active = Arc::new(AtomicBool::new(true));

        connection.calls.insert(
            id,
            CallSlot {
                inbound: None,
                cancel: cancel.clone(),
                active: Arc::clone(&active),
                tombstone: false,
            },
        );
        let result = connection.reject_inbound_overflow(id);

        if id <= 2 {
            result.expect("control task admitted below limit");
        } else {
            assert!(matches!(
                result,
                Err(Error::ControlTasksSaturated { max: 2 })
            ));
        }
        cancellations.push(cancel);
        active_calls.push(active);
    }

    assert_eq!(connection.control_tasks.len(), 2);
    assert_eq!(connection.calls.len(), 3);
    assert!(connection.calls.values().all(|slot| slot.tombstone));
    assert!(cancellations.iter().all(|cancel| cancel.is_cancelled()));
    assert!(
        active_calls
            .iter()
            .all(|active| !active.load(Ordering::Acquire))
    );

    // The third call was deactivated and tombstoned even though admitting
    // another terminal writer poisoned the connection.
    assert_eq!(connection.control_tasks.len(), config.max_in_flight_calls());
}

#[tokio::test]
async fn control_timeout_never_cancels_a_partially_written_frame() {
    let bytes_written = Arc::new(AtomicUsize::new(0));
    let writer = PartialThenPendingWriter {
        bytes_written: Arc::clone(&bytes_written),
    };
    let config = StreamConfig::new(FrameConfig::default(), 1, Duration::from_millis(10));
    let mut connection =
        StreamConnection::with_config(tokio::io::empty(), writer, PeerInfo { addr: None }, config);

    insert_test_slot(&mut connection, 1);
    connection
        .reject_inbound_overflow(1)
        .expect("control task admitted");
    tokio::time::sleep(Duration::from_millis(30)).await;

    assert_eq!(bytes_written.load(Ordering::Acquire), 2);
    assert_eq!(connection.control_tasks.len(), 1);
    assert!(connection.control_tasks.try_join_next().is_none());
    assert!(connection.calls.get(&1).unwrap().tombstone);
}

#[tokio::test]
async fn failed_terminal_write_poison_connection() {
    let (server_read, client) = duplex(64);
    let config = StreamConfig::new(FrameConfig::default(), 1, Duration::from_secs(1));
    let mut connection =
        StreamConnection::with_config(server_read, FailingWriter, PeerInfo { addr: None }, config);
    insert_test_slot(&mut connection, 1);
    connection
        .reject_inbound_overflow(1)
        .expect("control task admitted");

    assert!(matches!(
        timeout(Duration::from_secs(2), connection.recv())
            .await
            .expect("poison deadline"),
        Err(Error::Io(error)) if error.kind() == io::ErrorKind::BrokenPipe
    ));

    drop(client);
}

#[tokio::test]
async fn terminal_write_lock_timeout_poison_connection() {
    let (server, client) = duplex(64);
    let (server_read, server_write) = split(server);
    let config = StreamConfig::new(FrameConfig::default(), 1, Duration::from_millis(10));
    let mut connection =
        StreamConnection::with_config(server_read, server_write, PeerInfo { addr: None }, config);
    let write = Arc::clone(&connection.write);
    let write_guard = write.lock_owned().await;
    insert_test_slot(&mut connection, 1);
    connection
        .reject_inbound_overflow(1)
        .expect("control task admitted");

    assert!(matches!(
        timeout(Duration::from_secs(2), connection.recv())
            .await
            .expect("lock timeout deadline"),
        Err(Error::ControlWriteLockTimeout { timeout: actual })
            if actual == Duration::from_millis(10)
    ));

    drop(write_guard);
    drop(client);
}

#[tokio::test]
async fn overflow_tombstone_prevents_id_reuse_before_terminal_write() {
    let (server, client) = duplex(64 * 1024);
    let (server_read, server_write) = split(server);
    let (_client_read, mut client_write) = split(client);
    let mut connection = StreamConnection::new(server_read, server_write, PeerInfo { addr: None });

    write_message(&mut client_write, &request(1, true))
        .await
        .expect("open streaming call");
    let (first, _responder) = connection
        .recv()
        .await
        .expect("receive first request")
        .expect("first request present");
    let write = Arc::clone(&connection.write);
    let write_guard = write.lock_owned().await;

    for value in 0..=32_u8 {
        write_message(
            &mut client_write,
            &WireMessage::StreamItem {
                id: 1,
                payload: vec![value],
            },
        )
        .await
        .expect("write stream item");
    }
    write_message(&mut client_write, &request(1, false))
        .await
        .expect("attempt duplicate id");

    assert!(matches!(
        timeout(Duration::from_secs(2), connection.recv())
            .await
            .expect("duplicate deadline"),
        Err(Error::DuplicateCallId { id: 1 })
    ));
    assert!(first.cancel.is_cancelled());

    drop(write_guard);
}

#[tokio::test]
async fn partial_prefix_survives_completion_branch_cancellation() {
    let (server, client) = duplex(4096);
    let (server_read, server_write) = split(server);
    let (mut client_read, mut client_write) = split(client);
    let mut connection = StreamConnection::new(server_read, server_write, PeerInfo { addr: None });

    write_message(&mut client_write, &request(1, false))
        .await
        .expect("write first request");
    let (_first, responder) = connection
        .recv()
        .await
        .expect("receive first request")
        .expect("first request present");

    let encoded = postcard::to_allocvec(&request(2, false)).expect("encode second request");
    let prefix = (encoded.len() as u32).to_le_bytes();
    client_write
        .write_all(&prefix[..2])
        .await
        .expect("write partial prefix");

    {
        let recv = connection.recv();
        tokio::pin!(recv);
        assert!(timeout(Duration::from_millis(20), &mut recv).await.is_err());
    }

    crate::transport::Respond::respond(responder, crate::frame::CallResult::Ok(Vec::new()))
        .await
        .expect("complete first call");
    let _response = read_message(&mut client_read)
        .await
        .expect("read first response");

    client_write
        .write_all(&prefix[2..])
        .await
        .expect("finish prefix");
    client_write
        .write_all(&encoded)
        .await
        .expect("write second payload");

    let (second, _responder) = connection
        .recv()
        .await
        .expect("connection remains synchronized")
        .expect("second request present");
    assert_eq!(second.path, "svc.call_2");
}

#[tokio::test]
async fn closing_peer_does_not_wait_for_idle_timeout() {
    let (server, client) = duplex(64);
    let (server_read, server_write) = split(server);
    let mut connection = StreamConnection::new(server_read, server_write, PeerInfo { addr: None });

    drop(client);

    assert!(
        timeout(Duration::from_millis(100), connection.recv())
            .await
            .expect("disconnect should resolve promptly")
            .expect("orderly disconnect")
            .is_none()
    );
}
