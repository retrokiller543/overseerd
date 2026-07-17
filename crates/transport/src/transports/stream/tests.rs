use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
use std::task::{Context, Poll};
use std::time::Duration;
use std::{future::Future, io, pin::Pin};

use tokio::io::{AsyncWrite, AsyncWriteExt, duplex, split};
use tokio::time::timeout;

use super::{CallSlot, INBOUND_END_RECONCILE_INTERVAL, StreamConfig, StreamConnection};
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

#[test]
fn default_inbound_budget_accepts_one_configured_maximum_frame() {
    let frame = FrameConfig::new(128 * 1024 * 1024, Duration::from_secs(1));
    let config = StreamConfig::new(frame, 1, Duration::from_secs(1));

    assert_eq!(config.max_inbound_bytes_per_call(), frame.max_frame_len());
    assert_eq!(
        config.max_inbound_bytes_per_connection(),
        frame.max_frame_len()
    );
}

fn insert_test_slot<R, W>(connection: &mut StreamConnection<R, W>, id: u64) {
    connection.calls.insert(
        id,
        CallSlot {
            inbound: None,
            inbound_sizes: std::collections::VecDeque::new(),
            inbound_bytes: 0,
            inbound_ending: false,
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

struct PartialThenFailWriter {
    bytes_written: Arc<AtomicUsize>,
}

impl AsyncWrite for PartialThenFailWriter {
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
            Poll::Ready(Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "test writer failed after prefix bytes",
            )))
        }
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
async fn inbound_byte_budget_reconciles_other_calls_and_rejects_only_offender() {
    let (server, client) = duplex(64 * 1024);
    let (server_read, server_write) = split(server);
    let (_client_read, mut client_write) = split(client);
    let config = StreamConfig::new(FrameConfig::default(), 4, Duration::from_secs(1))
        .with_inbound_byte_limits(4, 6);
    let mut connection =
        StreamConnection::with_config(server_read, server_write, PeerInfo { addr: None }, config);

    write_message(&mut client_write, &request(1, true))
        .await
        .expect("open first stream");
    let (mut first, _first_responder) = connection
        .recv()
        .await
        .expect("receive first stream")
        .expect("first stream present");
    write_message(
        &mut client_write,
        &WireMessage::StreamItem {
            id: 1,
            payload: vec![1; 4],
        },
    )
    .await
    .expect("buffer first item");
    write_message(&mut client_write, &request(2, true))
        .await
        .expect("open second stream");
    let (second, _second_responder) = connection
        .recv()
        .await
        .expect("receive second stream")
        .expect("second stream present");
    assert_eq!(connection.inbound_bytes, 4);

    assert_eq!(
        first
            .requests
            .as_mut()
            .expect("first request receiver")
            .recv()
            .await,
        Some(vec![1; 4])
    );

    // Although call 1 has sent no new frame, admitting call 2 reconciles its consumed sender
    // capacity and releases the connection-wide byte charge.
    write_message(
        &mut client_write,
        &WireMessage::StreamItem {
            id: 2,
            payload: vec![2; 4],
        },
    )
    .await
    .expect("buffer second item");
    write_message(&mut client_write, &request(3, false))
        .await
        .expect("open third call");
    connection
        .recv()
        .await
        .expect("connection remains healthy")
        .expect("third call present");
    assert_eq!(connection.inbound_bytes, 4);
    assert!(!second.cancel.is_cancelled());

    // A new four-byte item would exceed the six-byte aggregate budget. Only its call is
    // terminated; the independent call and connection continue.
    write_message(
        &mut client_write,
        &WireMessage::StreamItem {
            id: 1,
            payload: vec![3; 4],
        },
    )
    .await
    .expect("write aggregate overflow");
    write_message(&mut client_write, &request(4, false))
        .await
        .expect("open independent call");
    let (fourth, _responder) = connection
        .recv()
        .await
        .expect("connection survives aggregate overflow")
        .expect("fourth call present");

    assert_eq!(fourth.path, "svc.call_4");
    assert!(first.cancel.is_cancelled());
    assert!(!second.cancel.is_cancelled());
    assert_eq!(connection.inbound_bytes, 4);
}

#[tokio::test]
async fn dropped_inbound_receiver_releases_connection_byte_budget() {
    let (server, client) = duplex(4096);
    let (server_read, server_write) = split(server);
    let (_client_read, mut client_write) = split(client);
    let config = StreamConfig::new(FrameConfig::default(), 4, Duration::from_secs(1))
        .with_inbound_byte_limits(4, 6);
    let mut connection =
        StreamConnection::with_config(server_read, server_write, PeerInfo { addr: None }, config);

    write_message(&mut client_write, &request(1, true))
        .await
        .expect("open stream");
    let (mut first, _responder) = connection
        .recv()
        .await
        .expect("receive stream")
        .expect("stream present");
    write_message(
        &mut client_write,
        &WireMessage::StreamItem {
            id: 1,
            payload: vec![1; 4],
        },
    )
    .await
    .expect("buffer item");
    write_message(&mut client_write, &request(2, false))
        .await
        .expect("drive buffered item");
    connection
        .recv()
        .await
        .expect("connection healthy")
        .expect("second call present");
    assert_eq!(connection.inbound_bytes, 4);

    drop(first.requests.take());
    write_message(
        &mut client_write,
        &WireMessage::StreamItem {
            id: 1,
            payload: vec![2],
        },
    )
    .await
    .expect("write after receiver drop");
    write_message(&mut client_write, &request(3, false))
        .await
        .expect("drive closed receiver");
    connection
        .recv()
        .await
        .expect("connection survives closed receiver")
        .expect("third call present");

    assert_eq!(connection.inbound_bytes, 0);
    assert!(connection.calls.get(&1).unwrap().inbound.is_none());
}

#[tokio::test]
async fn stream_end_releases_budget_as_the_handler_consumes_buffered_items() {
    let (server, client) = duplex(4096);
    let (server_read, server_write) = split(server);
    let (_client_read, mut client_write) = split(client);
    let config = StreamConfig::new(FrameConfig::default(), 3, Duration::from_secs(1))
        .with_inbound_byte_limits(4, 6);
    let mut connection =
        StreamConnection::with_config(server_read, server_write, PeerInfo { addr: None }, config);

    write_message(&mut client_write, &request(1, true))
        .await
        .expect("open stream");
    let (mut first, _responder) = connection
        .recv()
        .await
        .expect("receive stream")
        .expect("stream present");
    write_message(
        &mut client_write,
        &WireMessage::StreamItem {
            id: 1,
            payload: vec![1; 4],
        },
    )
    .await
    .expect("buffer item");
    write_message(&mut client_write, &WireMessage::StreamEnd { id: 1 })
        .await
        .expect("end stream");
    write_message(&mut client_write, &request(2, false))
        .await
        .expect("drive stream end");
    connection
        .recv()
        .await
        .expect("connection healthy")
        .expect("second call present");
    assert_eq!(connection.inbound_bytes, 4);
    assert!(connection.calls.get(&1).unwrap().inbound_ending);

    assert_eq!(
        first
            .requests
            .as_mut()
            .expect("request receiver")
            .recv()
            .await,
        Some(vec![1; 4])
    );

    // Drive the connection loop long enough for its bounded end-of-stream reconciler to observe
    // the released channel capacity. No network frame is expected, so the outer timeout wins.
    assert!(
        timeout(Duration::from_millis(30), connection.recv())
            .await
            .is_err()
    );
    assert_eq!(connection.inbound_bytes, 0);
    assert!(connection.calls.get(&1).unwrap().inbound.is_none());
    assert!(
        first
            .requests
            .as_mut()
            .expect("request receiver")
            .recv()
            .await
            .is_none()
    );
}

#[tokio::test]
async fn stream_end_reconciles_before_an_already_ready_request() {
    let (server, client) = duplex(4096);
    let (server_read, server_write) = split(server);
    let (_client_read, mut client_write) = split(client);
    let config = StreamConfig::new(FrameConfig::default(), 3, Duration::from_secs(1))
        .with_inbound_byte_limits(4, 6);
    let mut connection =
        StreamConnection::with_config(server_read, server_write, PeerInfo { addr: None }, config);

    write_message(&mut client_write, &request(1, true))
        .await
        .expect("open stream");
    let (mut first, _responder) = connection
        .recv()
        .await
        .expect("receive stream")
        .expect("stream present");
    write_message(
        &mut client_write,
        &WireMessage::StreamItem {
            id: 1,
            payload: vec![1; 4],
        },
    )
    .await
    .expect("buffer item");
    write_message(&mut client_write, &WireMessage::StreamEnd { id: 1 })
        .await
        .expect("end stream");
    write_message(&mut client_write, &request(2, false))
        .await
        .expect("drive stream end");
    connection
        .recv()
        .await
        .expect("connection healthy")
        .expect("second call present");

    assert_eq!(
        first
            .requests
            .as_mut()
            .expect("request receiver")
            .recv()
            .await,
        Some(vec![1; 4])
    );
    write_message(&mut client_write, &request(3, false))
        .await
        .expect("queue unrelated request");

    connection
        .recv()
        .await
        .expect("connection healthy")
        .expect("third call present");

    assert_eq!(connection.inbound_bytes, 0);
    assert!(connection.calls.get(&1).unwrap().inbound.is_none());
    assert!(
        first
            .requests
            .as_mut()
            .expect("request receiver")
            .recv()
            .await
            .is_none()
    );
}

#[tokio::test(start_paused = true)]
async fn periodic_stream_end_reconciliation_does_not_extend_frame_idle_deadline() {
    let (server, client) = duplex(4096);
    let (server_read, server_write) = split(server);
    let (_client_read, mut client_write) = split(client);
    let idle_timeout = Duration::from_millis(50);
    let config = StreamConfig::new(
        FrameConfig::new(1024, idle_timeout),
        3,
        Duration::from_secs(1),
    );
    let mut connection =
        StreamConnection::with_config(server_read, server_write, PeerInfo { addr: None }, config);

    write_message(&mut client_write, &request(1, true))
        .await
        .expect("open stream");
    let (_first, _responder) = connection
        .recv()
        .await
        .expect("receive stream")
        .expect("stream present");
    write_message(
        &mut client_write,
        &WireMessage::StreamItem {
            id: 1,
            payload: vec![1],
        },
    )
    .await
    .expect("buffer item");
    write_message(&mut client_write, &WireMessage::StreamEnd { id: 1 })
        .await
        .expect("end stream");
    write_message(&mut client_write, &request(2, false))
        .await
        .expect("drive stream end");
    connection
        .recv()
        .await
        .expect("connection healthy")
        .expect("second call present");
    assert!(connection.calls.get(&1).unwrap().inbound_ending);

    // Begin another frame but leave its prefix incomplete. Poll once so both the frame idle
    // deadline and the periodic reconciliation timer are armed at the current paused instant.
    client_write
        .write_all(&[1])
        .await
        .expect("write partial prefix");
    let mut recv = Box::pin(connection.recv());
    std::future::poll_fn(|cx| {
        assert!(recv.as_mut().poll(cx).is_pending());
        Poll::Ready(())
    })
    .await;

    // Advancing past both timers makes the biased maintenance branch run first. The resumed frame
    // read must retain its original deadline and fail immediately instead of receiving a fresh
    // timeout on every 10 ms reconciliation wakeup.
    tokio::time::advance(idle_timeout + INBOUND_END_RECONCILE_INTERVAL).await;
    let result = std::future::poll_fn(|cx| match recv.as_mut().poll(cx) {
        Poll::Ready(result) => Poll::Ready(result),
        Poll::Pending => panic!("periodic maintenance extended the partial frame idle deadline"),
    })
    .await;

    assert!(matches!(
        result,
        Err(Error::ReadTimeout {
            idle_timeout: actual
        }) if actual == idle_timeout
    ));
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
                inbound_sizes: std::collections::VecDeque::new(),
                inbound_bytes: 0,
                inbound_ending: false,
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
async fn control_timeout_after_a_partial_write_poison_connection() {
    let bytes_written = Arc::new(AtomicUsize::new(0));
    let writer = PartialThenPendingWriter {
        bytes_written: Arc::clone(&bytes_written),
    };
    let (server_read, client) = duplex(1);
    let config = StreamConfig::new(FrameConfig::default(), 1, Duration::from_millis(10));
    let mut connection =
        StreamConnection::with_config(server_read, writer, PeerInfo { addr: None }, config);

    insert_test_slot(&mut connection, 1);
    connection
        .reject_inbound_overflow(1)
        .expect("control task admitted");
    let result = timeout(Duration::from_secs(2), connection.recv())
        .await
        .expect("control write deadline");
    match result {
        Err(Error::ControlWriteTimeout { timeout: actual }) => {
            assert_eq!(actual, Duration::from_millis(10));
        }
        Err(Error::Closed) => {}
        Err(error) => panic!("unexpected poisoned connection error: {error}"),
        Ok(None) => panic!("poisoned connection ended without an error"),
        Ok(Some(_)) => panic!("poisoned connection admitted a request"),
    }
    assert_eq!(bytes_written.load(Ordering::Acquire), 2);
    assert!(connection.health.is_closed());
    drop(client);
}

#[tokio::test]
async fn partially_failed_normal_response_poison_connection_and_reaps_call() {
    let bytes_written = Arc::new(AtomicUsize::new(0));
    let (server_read, mut client_write) = duplex(4096);
    let mut connection = StreamConnection::new(
        server_read,
        PartialThenFailWriter {
            bytes_written: Arc::clone(&bytes_written),
        },
        PeerInfo { addr: None },
    );
    write_message(&mut client_write, &request(1, false))
        .await
        .expect("write request");
    let (_call, responder) = connection
        .recv()
        .await
        .expect("receive request")
        .expect("request present");

    assert!(matches!(
        crate::transport::Respond::respond(responder, crate::frame::CallResult::Ok(Vec::new()))
            .await,
        Err(Error::Io(error)) if error.kind() == io::ErrorKind::BrokenPipe
    ));
    assert_eq!(bytes_written.load(Ordering::Acquire), 2);
    assert!(matches!(connection.recv().await, Err(Error::Closed)));
    assert!(connection.calls.is_empty());
}

#[tokio::test]
async fn cancelling_a_partially_written_response_poison_connection() {
    let bytes_written = Arc::new(AtomicUsize::new(0));
    let (server_read, mut client_write) = duplex(4096);
    let mut connection = StreamConnection::new(
        server_read,
        PartialThenPendingWriter {
            bytes_written: Arc::clone(&bytes_written),
        },
        PeerInfo { addr: None },
    );
    write_message(&mut client_write, &request(1, false))
        .await
        .expect("write request");
    let (_call, responder) = connection
        .recv()
        .await
        .expect("receive request")
        .expect("request present");
    let response = tokio::spawn(crate::transport::Respond::respond(
        responder,
        crate::frame::CallResult::Ok(Vec::new()),
    ));

    timeout(Duration::from_secs(2), async {
        while bytes_written.load(Ordering::Acquire) < 2 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("response never reached partial write");
    response.abort();
    assert!(matches!(response.await, Err(error) if error.is_cancelled()));

    assert!(matches!(
        timeout(Duration::from_secs(2), connection.recv())
            .await
            .expect("poisoned connection deadline"),
        Err(Error::Closed)
    ));
    assert_eq!(bytes_written.load(Ordering::Acquire), 2);
}

#[tokio::test]
async fn failed_stream_item_poison_connection_and_reaps_call() {
    let (server_read, mut client_write) = duplex(4096);
    let mut connection = StreamConnection::new(server_read, FailingWriter, PeerInfo { addr: None });
    write_message(&mut client_write, &request(1, false))
        .await
        .expect("write request");
    let (_call, responder) = connection
        .recv()
        .await
        .expect("receive request")
        .expect("request present");
    let mut sink = crate::transport::RespondStream::into_sink(responder);

    assert!(matches!(
        crate::transport::ResponseSink::send(&mut sink, vec![1]).await,
        Err(Error::Io(error)) if error.kind() == io::ErrorKind::BrokenPipe
    ));
    assert!(matches!(connection.recv().await, Err(Error::Closed)));
    assert!(connection.calls.is_empty());
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
        Err(Error::ControlWriteTimeout { timeout: actual })
            if actual == Duration::from_millis(10)
    ));

    drop(write_guard);
    drop(client);
}

#[tokio::test]
async fn stream_cancel_is_terminal_and_stale_items_cannot_reactivate_call() {
    let (server, client) = duplex(4096);
    let (server_read, server_write) = split(server);
    let (_client_read, mut client_write) = split(client);
    let mut connection = StreamConnection::new(server_read, server_write, PeerInfo { addr: None });

    write_message(&mut client_write, &request(1, true))
        .await
        .expect("write streaming request");
    let (mut first, first_responder) = connection
        .recv()
        .await
        .expect("receive first request")
        .expect("first request present");
    let mut first_sink = crate::transport::RespondStream::into_sink(first_responder);

    write_message(&mut client_write, &WireMessage::StreamCancel { id: 1 })
        .await
        .expect("cancel first request");
    write_message(
        &mut client_write,
        &WireMessage::StreamItem {
            id: 1,
            payload: vec![99],
        },
    )
    .await
    .expect("write stale item");
    write_message(&mut client_write, &WireMessage::StreamEnd { id: 1 })
        .await
        .expect("write stale end");
    write_message(&mut client_write, &request(1, false))
        .await
        .expect("reuse terminal call id");

    let (second, _second_responder) = timeout(Duration::from_secs(2), connection.recv())
        .await
        .expect("replacement request deadline")
        .expect("connection remains healthy")
        .expect("replacement request present");

    assert_eq!(second.path, "svc.call_1");
    assert!(first.cancel.is_cancelled());
    assert!(
        first
            .requests
            .take()
            .expect("stream receiver")
            .recv()
            .await
            .is_none()
    );
    assert!(matches!(
        crate::transport::ResponseSink::send(&mut first_sink, vec![1]).await,
        Err(Error::Closed)
    ));
    assert!(connection.calls.contains_key(&1));

    write_message(&mut client_write, &request(2, false))
        .await
        .expect("write call after stale completion");
    let (after_completion, _responder) = connection
        .recv()
        .await
        .expect("stale completion keeps connection healthy")
        .expect("new call present");
    assert_eq!(after_completion.path, "svc.call_2");
    assert!(
        connection.calls.contains_key(&1),
        "old completion must not remove replacement generation"
    );
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
    write_message(&mut client_write, &WireMessage::StreamCancel { id: 1 })
        .await
        .expect("cancel overflow tombstone");
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
