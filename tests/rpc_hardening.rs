use std::future::pending;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use overseerd::daemon::{
    App, Cancel, ErrorHandler, ErrorResponse, Payload, RpcAppBuilder, RpcLimits, handlers, service,
};
use overseerd::{
    CallResult, Connection, MemoryClient, MemoryConnection, MemoryTransport, PeerInfo,
    PredefinedCode, Respond, RespondStream, ResponseSink, StatusCode, Transport,
};
use tokio::time::timeout;

#[service(id = "hardening", version = "0.1")]
struct Hardening;

#[handlers]
impl Hardening {
    #[rpc]
    async fn echo(Payload(value): Payload<u32>) -> u32 {
        value
    }

    #[rpc]
    async fn panics() -> u32 {
        panic!("handler secret must stay server-side")
    }

    #[rpc]
    async fn waits(cancel: Cancel) -> u32 {
        let _guard = ActiveCallGuard::new();
        cancel.0.cancelled().await;
        0
    }
}

static ACTIVE_CALLS: AtomicUsize = AtomicUsize::new(0);

struct ActiveCallGuard;

impl ActiveCallGuard {
    fn new() -> Self {
        ACTIVE_CALLS.fetch_add(1, Ordering::SeqCst);
        Self
    }
}

impl Drop for ActiveCallGuard {
    fn drop(&mut self) {
        ACTIVE_CALLS.fetch_sub(1, Ordering::SeqCst);
    }
}

struct PanickingErrorHandler;

impl ErrorHandler for PanickingErrorHandler {
    fn handle<'a>(
        &'a self,
        _path: &'a str,
        _error: ErrorResponse,
    ) -> Pin<Box<dyn Future<Output = ErrorResponse> + Send + 'a>> {
        Box::pin(async { panic!("global error handler panic") })
    }
}

struct SynchronouslyPanickingErrorHandler;

impl ErrorHandler for SynchronouslyPanickingErrorHandler {
    fn handle<'a>(
        &'a self,
        _path: &'a str,
        _error: ErrorResponse,
    ) -> Pin<Box<dyn Future<Output = ErrorResponse> + Send + 'a>> {
        panic!("synchronous global error handler panic")
    }
}

fn encode<T: serde::Serialize>(value: &T) -> Vec<u8> {
    postcard::to_allocvec(value).expect("encode request")
}

async fn start_memory(
    limits: RpcLimits,
) -> (
    overseerd::MemoryConnectionHandle,
    tokio::task::JoinHandle<overseerd::daemon::Result<()>>,
) {
    let (client, transport) = MemoryClient::pair();
    let app = App::builder("hardening-test")
        .auto_discover()
        .rpc_limits(limits)
        .build()
        .await
        .expect("build app");
    let connection = client.connect().await.expect("connect memory transport");
    let server = tokio::spawn(app.serve(transport));

    (connection, server)
}

#[tokio::test]
async fn handler_panic_returns_redacted_internal_error() {
    let (connection, server) = start_memory(RpcLimits::default()).await;

    let result = timeout(
        Duration::from_secs(2),
        connection.call("Hardening.panics", encode(&())),
    )
    .await
    .expect("panic response deadline")
    .expect("transport response");

    match result {
        CallResult::Err { code, body } => {
            assert_eq!(code.predefined(), PredefinedCode::Internal);
            let message: String = postcard::from_bytes(&body).expect("decode error body");
            assert_eq!(message, "internal server error");
            assert!(!message.contains("handler secret"));
        }
        CallResult::Ok(_) => panic!("panicking handler unexpectedly succeeded"),
    }

    drop(connection);
    timeout(Duration::from_secs(2), server)
        .await
        .expect("server shutdown deadline")
        .expect("server task")
        .expect("clean server shutdown");
}

#[tokio::test]
async fn panicking_global_error_handler_falls_back_to_internal_response() {
    let (client, transport) = MemoryClient::pair();
    let app = App::builder("hardening-test")
        .auto_discover()
        .error_handler(PanickingErrorHandler)
        .build()
        .await
        .expect("build app");
    let connection = client.connect().await.expect("connect memory transport");
    let server = tokio::spawn(app.serve(transport));

    let result = timeout(
        Duration::from_secs(2),
        connection.call("Hardening.panics", encode(&())),
    )
    .await
    .expect("fallback response deadline")
    .expect("transport response");

    match result {
        CallResult::Err { code, body } => {
            assert_eq!(code.predefined(), PredefinedCode::Internal);
            let message: String = postcard::from_bytes(&body).expect("decode error body");
            assert_eq!(message, "internal server error");
        }
        CallResult::Ok(_) => panic!("panicking handler unexpectedly succeeded"),
    }

    drop(connection);
    drop(client);
    timeout(Duration::from_secs(2), server)
        .await
        .expect("server shutdown deadline")
        .expect("server task")
        .expect("clean server shutdown");
}

#[tokio::test]
async fn synchronously_panicking_global_error_handler_falls_back_to_internal_response() {
    let (client, transport) = MemoryClient::pair();
    let app = App::builder("hardening-test")
        .auto_discover()
        .error_handler(SynchronouslyPanickingErrorHandler)
        .build()
        .await
        .expect("build app");
    let connection = client.connect().await.expect("connect memory transport");
    let server = tokio::spawn(app.serve(transport));

    let result = timeout(
        Duration::from_secs(2),
        connection.call("Hardening.panics", encode(&())),
    )
    .await
    .expect("fallback response deadline")
    .expect("transport response");

    match result {
        CallResult::Err { code, body } => {
            assert_eq!(code.predefined(), PredefinedCode::Internal);
            let message: String = postcard::from_bytes(&body).expect("decode error body");
            assert_eq!(message, "internal server error");
        }
        CallResult::Ok(_) => panic!("panicking handler unexpectedly succeeded"),
    }

    drop(connection);
    drop(client);
    timeout(Duration::from_secs(2), server)
        .await
        .expect("server shutdown deadline")
        .expect("server task")
        .expect("clean server shutdown");
}

#[tokio::test]
async fn completed_call_tasks_are_reaped_before_admission_check() {
    let limits = RpcLimits::new(1, 1);
    let (connection, server) = start_memory(limits).await;

    for value in 0..256_u32 {
        let result = connection
            .call("Hardening.echo", encode(&value))
            .await
            .expect("sequential call transport");

        match result {
            CallResult::Ok(body) => {
                assert_eq!(postcard::from_bytes::<u32>(&body).unwrap(), value)
            }
            CallResult::Err { .. } => panic!("sequential call failed"),
        }
    }

    drop(connection);
    timeout(Duration::from_secs(2), server)
        .await
        .expect("server shutdown deadline")
        .expect("server task")
        .expect("clean server shutdown");
}

#[tokio::test]
async fn per_connection_admission_closes_abusive_connection_and_drops_tasks() {
    ACTIVE_CALLS.store(0, Ordering::SeqCst);
    let limits = RpcLimits::new(1, 1);
    let (connection, server) = start_memory(limits).await;
    let mut first = connection
        .open("Hardening.waits", encode(&()), false)
        .await
        .expect("open first call");

    while ACTIVE_CALLS.load(Ordering::SeqCst) == 0 {
        tokio::task::yield_now().await;
    }

    let mut second = connection
        .open("Hardening.echo", encode(&1_u32), false)
        .await
        .expect("enqueue excess call");

    assert!(
        timeout(Duration::from_secs(2), second.recv())
            .await
            .expect("excess call closes promptly")
            .is_none()
    );
    assert!(
        timeout(Duration::from_secs(2), first.recv())
            .await
            .expect("original call closes promptly")
            .is_none()
    );

    while ACTIVE_CALLS.load(Ordering::SeqCst) != 0 {
        tokio::task::yield_now().await;
    }

    drop(connection);
    timeout(Duration::from_secs(2), server)
        .await
        .expect("server shutdown deadline")
        .expect("server task")
        .expect("clean server shutdown");
}

struct FlakyTransport {
    inner: MemoryTransport,
    fail_next: bool,
}

struct PermanentFailTransport;

impl Transport for PermanentFailTransport {
    type Connection = MemoryConnection;

    async fn accept(&mut self) -> overseerd::transport::Result<Self::Connection> {
        Err(overseerd::transport::Error::Io(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "permanent test failure",
        )))
    }
}

impl Transport for FlakyTransport {
    type Connection = MemoryConnection;

    async fn accept(&mut self) -> overseerd::transport::Result<Self::Connection> {
        if std::mem::take(&mut self.fail_next) {
            return Err(overseerd::transport::Error::Io(std::io::Error::new(
                std::io::ErrorKind::ConnectionAborted,
                "transient test failure",
            )));
        }

        self.inner.accept().await
    }
}

#[tokio::test]
async fn transient_accept_error_retries_without_stopping_protocol() {
    let (client, transport) = MemoryClient::pair();
    let limits = RpcLimits::default()
        .with_accept_backoff(Duration::from_millis(1), Duration::from_millis(2));
    let app = App::builder("hardening-test")
        .auto_discover()
        .rpc_limits(limits)
        .build()
        .await
        .expect("build app");
    let server = tokio::spawn(app.serve(FlakyTransport {
        inner: transport,
        fail_next: true,
    }));
    let connection = client
        .connect()
        .await
        .expect("connect after transient error");

    let result = timeout(
        Duration::from_secs(2),
        connection.call("Hardening.echo", encode(&7_u32)),
    )
    .await
    .expect("retry response deadline")
    .expect("retry transport response");
    assert!(matches!(
        result,
        CallResult::Ok(body) if postcard::from_bytes::<u32>(&body).unwrap() == 7
    ));

    drop(connection);
    drop(client);
    timeout(Duration::from_secs(2), server)
        .await
        .expect("server shutdown deadline")
        .expect("server task")
        .expect("clean server shutdown");
}

#[tokio::test]
async fn permanent_accept_error_stops_protocol() {
    let app = App::builder("hardening-test")
        .auto_discover()
        .build()
        .await
        .expect("build app");

    assert!(
        timeout(Duration::from_secs(2), app.serve(PermanentFailTransport))
            .await
            .expect("terminal accept deadline")
            .is_err()
    );
}

#[derive(Clone)]
struct PendingResponder;

struct PendingSink;

impl Respond for PendingResponder {
    async fn respond(self, _outcome: CallResult) -> overseerd::transport::Result<()> {
        Ok(())
    }
}

impl RespondStream for PendingResponder {
    type Sink = PendingSink;

    fn into_sink(self) -> Self::Sink {
        PendingSink
    }
}

impl ResponseSink for PendingSink {
    async fn send(&mut self, _item: Vec<u8>) -> overseerd::transport::Result<()> {
        Ok(())
    }

    async fn error(self, _code: StatusCode, _body: Vec<u8>) -> overseerd::transport::Result<()> {
        Ok(())
    }

    async fn finish(self) -> overseerd::transport::Result<()> {
        Ok(())
    }
}

struct PendingConnection {
    live: Arc<AtomicUsize>,
    peer: PeerInfo,
}

impl Drop for PendingConnection {
    fn drop(&mut self) {
        self.live.fetch_sub(1, Ordering::SeqCst);
    }
}

impl Connection for PendingConnection {
    type Responder = PendingResponder;

    fn peer(&self) -> &PeerInfo {
        &self.peer
    }

    async fn recv(
        &mut self,
    ) -> overseerd::transport::Result<Option<(overseerd::IncomingCall, Self::Responder)>> {
        pending().await
    }
}

struct UnlimitedTransport {
    accepted: Arc<AtomicUsize>,
    live: Arc<AtomicUsize>,
}

impl Transport for UnlimitedTransport {
    type Connection = PendingConnection;

    async fn accept(&mut self) -> overseerd::transport::Result<Self::Connection> {
        self.accepted.fetch_add(1, Ordering::SeqCst);
        self.live.fetch_add(1, Ordering::SeqCst);
        Ok(PendingConnection {
            live: Arc::clone(&self.live),
            peer: PeerInfo { addr: None },
        })
    }
}

#[tokio::test]
async fn connection_admission_is_bounded_and_shutdown_leaves_no_tasks() {
    let accepted = Arc::new(AtomicUsize::new(0));
    let live = Arc::new(AtomicUsize::new(0));
    let app = App::builder("hardening-test")
        .auto_discover()
        .rpc_limits(RpcLimits::new(2, 1))
        .build()
        .await
        .expect("build app");
    let shutdown = app.shutdown_handle();
    let server = tokio::spawn(app.serve(UnlimitedTransport {
        accepted: Arc::clone(&accepted),
        live: Arc::clone(&live),
    }));

    while accepted.load(Ordering::SeqCst) < 2 {
        tokio::task::yield_now().await;
    }

    for _ in 0..16 {
        tokio::task::yield_now().await;
    }
    assert_eq!(accepted.load(Ordering::SeqCst), 2);
    assert_eq!(live.load(Ordering::SeqCst), 2);

    shutdown.shutdown();
    timeout(Duration::from_secs(2), server)
        .await
        .expect("bounded server shutdown deadline")
        .expect("server task")
        .expect("clean server shutdown");
    assert_eq!(live.load(Ordering::SeqCst), 0);
}
