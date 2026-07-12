//! The axum HTTP protocol.
//!
//! [`Axum`] implements the [`Protocol`]/[`Serve`] traits from `overseerd-app`: it owns the
//! assembled [`axum::Router`] (controllers merged, wrapped by the per-request scope layer)
//! and serves it over a [`SocketAddr`] or a pre-bound [`TcpListener`]. The serve envelope
//! (lifecycle hooks, reload triggers, ctrl-c) is run by `App::serve`, so this loop only
//! drives `axum::serve` with the shutdown signal as its graceful-shutdown future.

use std::future::IntoFuture;
use std::net::SocketAddr;

use overseerd_app::{AppRuntime, Protocol, Serve, ShutdownSignal};
use overseerd_config::Cfg;
use tokio::net::TcpListener;
use tracing::info;

/// The axum protocol: a fully-assembled [`axum::Router`] ready to serve. Built by
/// [`AxumPlugin`](crate::AxumPlugin).
pub struct Axum {
    router: axum::Router,
    config: Cfg<crate::AxumConfig>,

    /// The mounted WebSocket endpoints, kept for inspection and to drain their live connections on
    /// graceful shutdown (axum's graceful stop would otherwise wait on long-lived sockets).
    #[cfg(feature = "ws")]
    ws_endpoints: Vec<crate::ws::WebsocketHandler>,
}

impl Axum {
    pub(crate) fn new(router: axum::Router, config: Cfg<crate::AxumConfig>) -> Self {
        Self {
            router,
            config,
            #[cfg(feature = "ws")]
            ws_endpoints: Vec::new(),
        }
    }

    /// Attaches the mounted ws endpoint handles (built by [`AxumPlugin`](crate::AxumPlugin)).
    #[cfg(feature = "ws")]
    pub(crate) fn with_ws_endpoints(mut self, endpoints: Vec<crate::ws::WebsocketHandler>) -> Self {
        self.ws_endpoints = endpoints;

        self
    }

    /// The assembled router (controllers + scope layer), for inspection or testing.
    pub fn router(&self) -> &axum::Router {
        &self.router
    }

    /// A stable snapshot of the listener configuration currently published by the config store.
    pub fn config(&self) -> std::sync::Arc<crate::AxumConfig> {
        self.config.snapshot()
    }

    /// The listener address currently published by the config store.
    pub fn configured_addr(&self) -> SocketAddr {
        self.config.snapshot().socket_addr()
    }

    /// The mounted WebSocket endpoints (path + protocol), for inspection.
    #[cfg(feature = "ws")]
    pub fn ws_endpoints(&self) -> &[crate::ws::WebsocketHandler] {
        &self.ws_endpoints
    }

    /// Drives `axum::serve` over `listener` until the shutdown signal fires.
    async fn run(self, listener: TcpListener, mut shutdown: ShutdownSignal) -> crate::Result<()> {
        let local = listener.local_addr()?;
        let graceful_timeout = self.config.snapshot().graceful_shutdown_timeout_ms;

        info!(target: "overseerd::axum", addr = %local, "serve starting");

        let router = self.router;

        #[cfg(feature = "ws")]
        let ws_endpoints = self.ws_endpoints;

        let shutdown_started = std::sync::Arc::new(tokio::sync::Notify::new());
        let shutdown_notice = std::sync::Arc::clone(&shutdown_started);
        let server = axum::serve(listener, router)
            .with_graceful_shutdown(async move {
                shutdown.wait().await;

                // Drain ws connections so the graceful stop is not blocked on long-lived sockets.
                #[cfg(feature = "ws")]
                for endpoint in &ws_endpoints {
                    endpoint.trigger_shutdown();
                }

                shutdown_notice.notify_one();
            })
            .into_future();
        tokio::pin!(server);

        tokio::select! {
            result = &mut server => result?,

            _ = shutdown_started.notified() => {
                if graceful_timeout == 0 {
                    server.await?;
                } else {
                    match tokio::time::timeout(
                        std::time::Duration::from_millis(graceful_timeout),
                        &mut server,
                    )
                    .await
                    {
                        Ok(result) => result?,

                        Err(_) => {
                            tracing::warn!(
                                target: "overseerd::axum",
                                timeout_ms = graceful_timeout,
                                "graceful shutdown timed out; dropping remaining connections"
                            );
                        }
                    }
                }
            }
        }

        info!(target: "overseerd::axum", addr = %local, "serve stopped");

        Ok(())
    }
}

impl Protocol for Axum {
    type Error = crate::Error;
}

impl Serve<SocketAddr> for Axum {
    async fn serve(
        self,
        _runtime: AppRuntime,
        shutdown: ShutdownSignal,
        addr: SocketAddr,
    ) -> crate::Result<()> {
        let listener = TcpListener::bind(addr).await?;

        self.run(listener, shutdown).await
    }
}

impl Serve<()> for Axum {
    async fn serve(
        self,
        _runtime: AppRuntime,
        shutdown: ShutdownSignal,
        (): (),
    ) -> crate::Result<()> {
        // Snapshot immediately before binding so a config reload accepted by a startup hook is
        // observed. Listener changes after bind naturally apply on the next process start.
        let addr = self.config.snapshot().socket_addr();
        let listener = TcpListener::bind(addr).await?;

        self.run(listener, shutdown).await
    }
}

impl Serve<TcpListener> for Axum {
    async fn serve(
        self,
        _runtime: AppRuntime,
        shutdown: ShutdownSignal,
        listener: TcpListener,
    ) -> crate::Result<()> {
        self.run(listener, shutdown).await
    }
}
