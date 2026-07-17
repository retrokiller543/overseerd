use std::pin::Pin;
use std::sync::{Arc, atomic::AtomicBool};
use std::task::{Context, Poll};
use std::time::Duration;

use futures::StreamExt;
use tokio::io::{AsyncRead, AsyncWrite, ReadHalf, WriteHalf, duplex, split};
use tokio::sync::Notify;
use tokio::time::timeout;

use super::{
    ConnectionState, REPLY_BUFFER, REPLY_OVERFLOW_ERROR, RpcResponses, StreamClientTransport,
    WRITE_BUFFER, WriteCommand, Writer, serialize_frame,
};
use overseerd_client::ClientError;
use overseerd_transport::protocol::codec::{MessageReader, write_message};
use overseerd_transport::protocol::{WireMessage, WireRequest, WireResponse};
use overseerd_transport::{PredefinedCode, StatusCode, WireOutcome};

const TEST_TIMEOUT: Duration = Duration::from_secs(2);

type TestIo = tokio::io::DuplexStream;
type TestClient = StreamClientTransport<WriteHalf<TestIo>>;

struct StalledWriter {
    started: Arc<Notify>,
    dropped: Arc<AtomicBool>,
}

impl AsyncWrite for StalledWriter {
    fn poll_write(
        self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        _buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        self.started.notify_one();

        Poll::Pending
    }

    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}

impl Drop for StalledWriter {
    fn drop(&mut self) {
        self.dropped
            .store(true, std::sync::atomic::Ordering::Release);
    }
}

fn client_pair(capacity: usize) -> (TestClient, ReadHalf<TestIo>, WriteHalf<TestIo>) {
    let (client, server) = duplex(capacity);
    let (client_read, client_write) = split(client);
    let (server_read, server_write) = split(server);

    (
        StreamClientTransport::new(client_read, client_write),
        server_read,
        server_write,
    )
}

async fn read_message<R>(reader: &mut MessageReader<R>) -> WireMessage
where
    R: AsyncRead + Unpin,
{
    timeout(TEST_TIMEOUT, reader.read_message())
        .await
        .expect("timed out awaiting frame")
        .expect("read frame")
}

async fn write<R>(writer: &mut R, message: &WireMessage)
where
    R: AsyncWrite + Unpin,
{
    timeout(TEST_TIMEOUT, write_message(writer, message))
        .await
        .expect("timed out writing frame")
        .expect("write frame");
}

async fn open_call(
    client: &TestClient,
    server: &mut MessageReader<ReadHalf<TestIo>>,
) -> super::RpcCall<WriteHalf<TestIo>> {
    let call = timeout(TEST_TIMEOUT, client.open("svc.call", false, Vec::new()))
        .await
        .expect("open timed out")
        .expect("open call");
    let request = server.read_message().await.expect("read opening request");

    assert!(matches!(request, WireMessage::Request(_)));

    call
}

#[tokio::test]
async fn writer_finishes_an_accepted_frame_after_waiter_cancellation() {
    let (client, server) = duplex(8);
    let (_unused_read, client_write) = split(client);
    let (server_read, _server_write) = split(server);
    // Constructing a full transport also verifies the production task wiring. The peer never
    // writes, so only its writer actor is relevant to this test.
    let (client_read, _keep_read_open) = duplex(8);
    let transport = StreamClientTransport::new(client_read, client_write);
    let message = WireMessage::Request(WireRequest {
        id: 7,
        path: "svc.large".into(),
        payload: vec![42; 1024],
        streaming_input: false,
    });
    let frame = serialize_frame(&message).expect("serialize frame");

    assert!(
        timeout(
            Duration::from_millis(20),
            transport.writer.write_frame(frame)
        )
        .await
        .is_err(),
        "the small socket should backpressure and cancel the waiter"
    );

    let mut reader = MessageReader::new(server_read);
    assert!(matches!(
        read_message(&mut reader).await,
        WireMessage::Request(request) if request.id == 7 && request.payload.len() == 1024
    ));
}

#[tokio::test]
async fn cancelling_open_after_request_acceptance_removes_route_and_writes_fifo_cancel() {
    let (client_io, server_io) = duplex(8);
    let (client_read, client_write) = split(client_io);
    let (server_read, _server_write) = split(server_io);
    let client = StreamClientTransport::new(client_read, client_write);
    let opener = client.clone();
    let opening =
        tokio::spawn(async move { opener.open("svc.large", false, vec![42; 1024]).await });

    timeout(TEST_TIMEOUT, async {
        loop {
            if client.calls.lock().unwrap().len() == 1
                && client.writer.queue.capacity() == WRITE_BUFFER
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("writer did not accept opening request");

    opening.abort();
    assert!(matches!(opening.await, Err(error) if error.is_cancelled()));
    assert!(client.calls.lock().unwrap().is_empty());

    let mut server = MessageReader::new(server_read);
    let id = match read_message(&mut server).await {
        WireMessage::Request(request) => request.id,
        _ => panic!("expected opening request"),
    };
    assert!(matches!(
        read_message(&mut server).await,
        WireMessage::StreamCancel { id: actual } if actual == id
    ));
}

#[tokio::test]
async fn final_transport_drop_forces_a_stalled_accepted_write_to_release_its_socket() {
    let started = Arc::new(Notify::new());
    let dropped = Arc::new(AtomicBool::new(false));
    let (client_read, _keep_read_open) = duplex(1);
    let transport = StreamClientTransport::new(
        client_read,
        StalledWriter {
            started: Arc::clone(&started),
            dropped: Arc::clone(&dropped),
        },
    );
    let writer = transport.writer.clone();
    let pending = tokio::spawn(async move { writer.write_frame(vec![1; 64]).await });

    timeout(TEST_TIMEOUT, started.notified())
        .await
        .expect("writer never accepted the frame");
    drop(transport);

    timeout(TEST_TIMEOUT, async {
        while !dropped.load(std::sync::atomic::Ordering::Acquire) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("stalled writer socket outlived the bounded drain");
    assert!(matches!(
        pending.await.expect("write waiter task"),
        Err(ClientError::Transport(overseerd_transport::Error::Closed))
            | Err(ClientError::ConnectionClosed)
    ));
}

#[tokio::test]
async fn read_loop_death_closes_future_opens() {
    let (client, server_read, server_write) = client_pair(4096);
    drop(server_read);
    drop(server_write);

    timeout(TEST_TIMEOUT, client.writer.state.shutdown.cancelled())
        .await
        .expect("read loop did not publish closure");

    assert!(matches!(
        timeout(
            Duration::from_millis(50),
            client.open("svc.call", false, Vec::new())
        )
        .await
        .expect("closed open must resolve immediately"),
        Err(ClientError::ConnectionClosed)
    ));
}

#[test]
fn saturated_control_lane_poison_connection_instead_of_allocating() {
    let (queue, _queue_rx) = tokio::sync::mpsc::channel(WRITE_BUFFER);
    let state = std::sync::Arc::new(ConnectionState::new());
    let writer = Writer {
        queue: queue.clone(),
        state: std::sync::Arc::clone(&state),
    };

    for _ in 0..WRITE_BUFFER {
        queue
            .try_send(WriteCommand::Control(Vec::new()))
            .expect("fill bounded writer queue");
    }

    writer.cancel(Vec::new());

    assert!(state.is_closed());
}

#[tokio::test]
async fn dropping_source_sends_remote_cancellation() {
    let (client, server_read, _server_write) = client_pair(4096);
    let mut server = MessageReader::new(server_read);
    let call = open_call(&client, &mut server).await;
    let id = call.id;
    let (_sink, source) = call.split();

    drop(source);

    assert!(matches!(
        read_message(&mut server).await,
        WireMessage::StreamCancel { id: actual } if actual == id
    ));
}

#[tokio::test]
async fn dropping_responses_sends_remote_cancellation() {
    let (client, server_read, _server_write) = client_pair(4096);
    let mut server = MessageReader::new(server_read);
    let call = open_call(&client, &mut server).await;
    let id = call.id;
    let (_sink, source) = call.split();
    let responses = RpcResponses::<_, u32, ()>::new(client, source);

    drop(responses);

    assert!(matches!(
        read_message(&mut server).await,
        WireMessage::StreamCancel { id: actual } if actual == id
    ));
}

#[tokio::test]
async fn overflowing_replies_reports_local_error_and_cancels_remote_call() {
    let (client, server_read, mut server_write) = client_pair(64 * 1024);
    let mut server = MessageReader::new(server_read);
    let call = open_call(&client, &mut server).await;
    let id = call.id;
    let (_sink, source) = call.split();
    let mut responses = RpcResponses::<_, u32, ()>::new(client, source);

    for value in 0..=REPLY_BUFFER {
        write(
            &mut server_write,
            &WireMessage::StreamItem {
                id,
                payload: postcard::to_allocvec(&(value as u32)).expect("encode item"),
            },
        )
        .await;
    }

    for value in 0..REPLY_BUFFER {
        assert_eq!(
            responses
                .next()
                .await
                .expect("buffered response")
                .expect("decode item"),
            value as u32
        );
    }

    assert!(matches!(
        responses.next().await,
        Some(Err(ClientError::Decode(message))) if message == REPLY_OVERFLOW_ERROR
    ));
    assert!(matches!(
        read_message(&mut server).await,
        WireMessage::StreamCancel { id: actual } if actual == id
    ));
}

#[tokio::test]
async fn full_item_buffer_does_not_block_terminal_or_other_calls() {
    let (client, server_read, mut server_write) = client_pair(64 * 1024);
    let mut server = MessageReader::new(server_read);
    let first = open_call(&client, &mut server).await;
    let first_id = first.id;
    let (_first_sink, mut first_source) = first.split();

    for value in 0..REPLY_BUFFER {
        write(
            &mut server_write,
            &WireMessage::StreamItem {
                id: first_id,
                payload: postcard::to_allocvec(&(value as u32)).expect("encode item"),
            },
        )
        .await;
    }
    write(
        &mut server_write,
        &WireMessage::StreamError {
            id: first_id,
            code: StatusCode::from(PredefinedCode::Internal),
            body: b"terminal".to_vec(),
        },
    )
    .await;

    let second = open_call(&client, &mut server).await;
    let second_id = second.id;
    let (_second_sink, mut second_source) = second.split();
    write(
        &mut server_write,
        &WireMessage::Response(WireResponse {
            id: second_id,
            outcome: WireOutcome::Ok(postcard::to_allocvec(&99_u32).expect("encode response")),
        }),
    )
    .await;

    assert!(matches!(
        timeout(TEST_TIMEOUT, second_source.recv()).await,
        Ok(Some(super::Reply::Response(WireOutcome::Ok(_))))
    ));

    for _ in 0..REPLY_BUFFER {
        assert!(matches!(
            first_source.recv().await,
            Some(super::Reply::Item(_))
        ));
    }
    assert!(matches!(
        first_source.recv().await,
        Some(super::Reply::Error { body, .. }) if body == b"terminal"
    ));
}
