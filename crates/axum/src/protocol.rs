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
}

impl Axum {
    pub(crate) fn new(router: axum::Router) -> Self {
        Self { router }
    }

    /// The assembled router (controllers + scope layer), for inspection or testing.
    pub fn router(&self) -> &axum::Router {
        &self.router
    }

    /// Drives `axum::serve` over `listener` until the shutdown signal fires.
    async fn run(self, listener: TcpListener, mut shutdown: ShutdownSignal) -> crate::Result<()> {
        let local = listener.local_addr()?;

        info!(target: "overseerd::axum", addr = %local, "serve starting");

        axum::serve(listener, self.router)
            .with_graceful_shutdown(async move {
                shutdown.wait().await;
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
