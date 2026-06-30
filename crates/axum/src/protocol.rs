//! The axum HTTP protocol.
//!
//! [`Axum`] implements the [`Protocol`]/[`Serve`] traits from `overseerd-app`: it owns the
//! assembled [`axum::Router`] (controllers merged, wrapped by the per-request scope layer)
//! and serves it over a [`SocketAddr`] or a pre-bound [`TcpListener`]. The serve envelope
//! (lifecycle hooks, reload triggers, ctrl-c) is run by `App::serve`, so this loop only
//! drives `axum::serve` with the shutdown signal as its graceful-shutdown future.

use std::net::SocketAddr;

use overseerd_app::{AppRuntime, Protocol, Serve, ShutdownSignal};
use tokio::net::TcpListener;
use tracing::info;

/// The axum protocol: a fully-assembled [`axum::Router`] ready to serve. Built by
/// [`AxumPlugin`](crate::AxumPlugin).
pub struct Axum {
    router: axum::Router,

    /// The mounted WebSocket endpoints, kept for inspection and to drain their live connections on
    /// graceful shutdown (axum's graceful stop would otherwise wait on long-lived sockets).
    #[cfg(feature = "ws")]
    ws_endpoints: Vec<crate::ws::WebsocketHandler>,
}

impl Axum {
    pub(crate) fn new(router: axum::Router) -> Self {
        Self {
            router,
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

    /// The mounted WebSocket endpoints (path + protocol), for inspection.
    #[cfg(feature = "ws")]
    pub fn ws_endpoints(&self) -> &[crate::ws::WebsocketHandler] {
        &self.ws_endpoints
    }

    /// Drives `axum::serve` over `listener` until the shutdown signal fires.
    async fn run(self, listener: TcpListener, mut shutdown: ShutdownSignal) -> crate::Result<()> {
        let local = listener.local_addr()?;

        info!(target: "overseerd::axum", addr = %local, "serve starting");

        let router = self.router;

        #[cfg(feature = "ws")]
        let ws_endpoints = self.ws_endpoints;

        axum::serve(listener, router)
            .with_graceful_shutdown(async move {
                shutdown.wait().await;

                // Drain ws connections so the graceful stop is not blocked on long-lived sockets.
                #[cfg(feature = "ws")]
                for endpoint in &ws_endpoints {
                    endpoint.trigger_shutdown();
                }
            })
            .await?;

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
