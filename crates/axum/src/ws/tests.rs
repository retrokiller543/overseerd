use std::any::{Any, TypeId};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

#[cfg(feature = "tungstenite")]
use axum::extract::ws::Message;
use axum::extract::ws::WebSocket;
#[cfg(feature = "tungstenite")]
use futures::StreamExt;
use overseerd_app::AppRuntime;
use overseerd_core::TypeDescriptor;
use overseerd_di::ScopeContainer;

#[cfg(feature = "tungstenite")]
use super::WsConnectionMeta;
use super::{
    WebsocketProtocol, WsAdmission, WsControllerDescriptor, WsFuture, WsHandlerFn, WsIdle, WsRoute,
    WsShutdown, mount_ws,
};
use crate::AxumAppBuilder as _;

static TEST_PROTOCOL_BUILDS: AtomicUsize = AtomicUsize::new(0);

struct TestProtocol;

impl WebsocketProtocol for TestProtocol {
    type Payload = ();
    type Outcome = ();
    type Options = ();
    type BuildError = std::convert::Infallible;

    fn build(
        _controllers: &[WsControllerDescriptor],
        _runtime: &AppRuntime,
        _options: (),
    ) -> Result<Self, Self::BuildError> {
        TEST_PROTOCOL_BUILDS.fetch_add(1, Ordering::Relaxed);

        Ok(Self)
    }

    async fn serve(
        self: Arc<Self>,
        socket: WebSocket,
        connection: Arc<ScopeContainer>,
        shutdown: WsShutdown,
    ) {
        let _ = (self, socket, connection, shutdown);
    }
}

#[tokio::test]
async fn old_signature_custom_protocol_mounts_without_adapter_methods() {
    let builds_before = TEST_PROTOCOL_BUILDS.load(Ordering::Relaxed);
    let app = crate::App::builder("old-signature-ws-test")
        .config_source(overseerd_config::ConfigManager::<overseerd_config::Toml>::empty())
        .register_ws::<TestProtocol>("/ws")
        .build()
        .await
        .expect("old-signature protocol mounts");

    assert_eq!(
        TEST_PROTOCOL_BUILDS.load(Ordering::Relaxed),
        builds_before + 1
    );
    assert_eq!(app.protocol().ws_endpoints().len(), 1);
    assert_eq!(app.protocol().ws_endpoints()[0].path(), "/ws");
}

#[test]
fn websocket_limits_fail_during_prepare_before_protocol_build() {
    let config = overseerd_config::ConfigManager::<overseerd_config::Toml>::from_str(
        r#"
            [axum]
            max_websocket_message_bytes = 0
        "#,
    )
    .expect("config parses");
    let builds_before = TEST_PROTOCOL_BUILDS.load(Ordering::Relaxed);
    let result = crate::App::builder("invalid-ws-config-test")
        .config_source(config)
        .register_ws::<TestProtocol>("/ws")
        .prepare();

    let error = match result {
        Ok(_) => panic!("zero WebSocket message limit was not rejected during preparation"),
        Err(error) => error,
    };

    assert!(error.to_string().contains("must both be greater than zero"));
    assert_eq!(TEST_PROTOCOL_BUILDS.load(Ordering::Relaxed), builds_before);
}

struct DuplicateProtocol;

static DUPLICATE_PROTOCOL_BUILDS: AtomicUsize = AtomicUsize::new(0);

impl WebsocketProtocol for DuplicateProtocol {
    type Payload = ();
    type Outcome = ();
    type Options = ();
    type BuildError = std::convert::Infallible;

    fn build(
        _: &[WsControllerDescriptor],
        _: &AppRuntime,
        _: (),
    ) -> Result<Self, Self::BuildError> {
        DUPLICATE_PROTOCOL_BUILDS.fetch_add(1, Ordering::Relaxed);

        Ok(Self)
    }

    async fn serve(
        self: Arc<Self>,
        socket: WebSocket,
        connection: Arc<ScopeContainer>,
        shutdown: WsShutdown,
    ) {
        let _ = (self, socket, connection, shutdown);
    }
}

fn duplicate_handler() -> WsHandlerFn<DuplicateProtocol> {
    Arc::new(|(), _scope| -> WsFuture<DuplicateProtocol> { Box::pin(async { Ok(()) }) })
}

fn duplicate_routes(_: &AppRuntime) -> Box<dyn Any + Send> {
    Box::new(vec![WsRoute::<DuplicateProtocol>::new(
        "messages.send",
        duplicate_handler(),
    )])
}

fn duplicate_protocol_id() -> TypeId {
    TypeId::of::<DuplicateProtocol>()
}

fn duplicate_protocol_name() -> &'static str {
    std::any::type_name::<DuplicateProtocol>()
}

fn duplicate_descriptor(id: &'static str) -> WsControllerDescriptor {
    WsControllerDescriptor {
        id,
        name: "DuplicateController",
        ty: TypeDescriptor::of::<DuplicateProtocol>("DuplicateProtocol"),
        protocol: duplicate_protocol_id,
        protocol_name: duplicate_protocol_name,
        routes: duplicate_routes,
    }
}

#[tokio::test]
async fn duplicate_destinations_fail_before_custom_protocol_build() {
    let app = crate::App::builder("duplicate-ws-route-test")
        .config_source(overseerd_config::ConfigManager::<overseerd_config::Toml>::empty())
        .build()
        .await
        .expect("test runtime builds");
    let builds_before = DUPLICATE_PROTOCOL_BUILDS.load(Ordering::Relaxed);
    let result = mount_ws::<DuplicateProtocol>(
        "/ws",
        vec![
            duplicate_descriptor("first"),
            duplicate_descriptor("second"),
        ],
        app.runtime(),
        (),
    );
    let error = result.err().expect("duplicate route must fail");

    assert!(error.to_string().contains("messages.send"));
    assert_eq!(
        DUPLICATE_PROTOCOL_BUILDS.load(Ordering::Relaxed),
        builds_before,
        "route validation must run before the downstream build implementation"
    );
}

#[test]
fn idle_timeout_probes_once_then_closes_unresponsive_peer() {
    let mut idle = WsIdle::new(Some(std::time::Duration::from_secs(10)));

    assert!(!idle.on_timeout(), "first idle interval sends a ping");
    assert!(idle.on_timeout(), "second idle interval closes the peer");

    idle.on_activity();

    assert!(
        !idle.on_timeout(),
        "peer activity clears the outstanding probe"
    );
}

#[test]
fn admission_permit_is_released_when_connection_finishes() {
    let admission = WsAdmission::new(1).expect("valid admission limit");
    let first = admission
        .try_acquire()
        .expect("admission is open")
        .expect("limit is enabled");

    assert!(admission.try_acquire().is_err(), "second peer is rejected");

    drop(first);

    assert!(
        admission
            .try_acquire()
            .expect("permit was released")
            .is_some(),
        "a later peer is admitted"
    );
}

#[test]
fn admission_accepts_tokio_boundary_and_rejects_oversized_config() {
    WsAdmission::new(tokio::sync::Semaphore::MAX_PERMITS)
        .expect("Tokio's maximum permit count is valid");

    let oversized = tokio::sync::Semaphore::MAX_PERMITS
        .checked_add(1)
        .expect("Tokio leaves room above MAX_PERMITS");
    let Err(error) = WsAdmission::new(oversized) else {
        panic!("oversized limit must fail app build");
    };

    assert!(error.to_string().contains("max_websocket_connections"));
    assert!(error.to_string().contains(&oversized.to_string()));
}

#[cfg(feature = "tungstenite")]
struct RequiredSubprotocol;

#[cfg(feature = "tungstenite")]
impl WebsocketProtocol for RequiredSubprotocol {
    type Payload = ();
    type Outcome = ();
    type Options = ();
    type BuildError = std::convert::Infallible;

    const SUBPROTOCOLS: &'static [&'static str] = &["test.v1"];
    const REQUIRE_SUBPROTOCOL: bool = true;

    fn build(
        _: &[WsControllerDescriptor],
        _: &AppRuntime,
        _: (),
    ) -> Result<Self, Self::BuildError> {
        Ok(Self)
    }

    async fn serve(
        self: Arc<Self>,
        mut socket: WebSocket,
        connection: Arc<ScopeContainer>,
        shutdown: WsShutdown,
    ) {
        let selected = connection
            .extract::<WsConnectionMeta>()
            .await
            .expect("connection metadata resolves")
            .selected_subprotocol()
            .unwrap_or_default()
            .to_owned();

        let _ = socket.send(Message::Text(selected.into())).await;
        let _ = (self, shutdown);
    }
}

#[cfg(feature = "tungstenite")]
#[tokio::test]
async fn required_subprotocol_is_negotiated_and_seeded() {
    let app = crate::App::builder("ws-subprotocol-test")
        .config_source(overseerd_config::ConfigManager::<overseerd_config::Toml>::empty())
        .register_ws::<RequiredSubprotocol>("/ws")
        .build()
        .await
        .expect("subprotocol app builds");
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .expect("bind test listener");
    let address = listener.local_addr().expect("listener address");
    let shutdown = app.shutdown_handle();
    let server = tokio::spawn(async move { app.serve(listener).await });
    let url = format!("ws://{address}/ws");

    let mut socket = tokio_tungstenite_wasm::connect_with_protocols(&url, &["other", "test.v1"])
        .await
        .expect("accepted subprotocol connects");
    let message = socket
        .next()
        .await
        .expect("selected protocol message")
        .expect("valid selected protocol message");

    assert_eq!(message.into_text().expect("text frame"), "test.v1");
    assert!(
        tokio_tungstenite_wasm::connect(&url).await.is_err(),
        "required protocol rejects a client that offers none"
    );

    shutdown.shutdown();
    server
        .await
        .expect("server task joins")
        .expect("server stops");
}
