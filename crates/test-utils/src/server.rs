use std::marker::PhantomData;
use std::net::SocketAddr;

use overseerd_app::{App, Protocol, ProtocolPlugin, Serve, ShutdownHandle};
use tokio::net::TcpListener;

use crate::LoopbackTask;

type ProtocolError<P> = <<P as ProtocolPlugin>::Protocol as Protocol>::Error;

/// Owns a protocol-generic loopback server and aborts it on incomplete cleanup.
pub struct TestServer<P: ProtocolPlugin, G = ()> {
    shutdown: ShutdownHandle,
    task: LoopbackTask<Result<(), ProtocolError<P>>>,
    _guard: G,
    _plugin: PhantomData<P>,
}

impl<P> TestServer<P>
where
    P: ProtocolPlugin + 'static,
    P::Protocol: Serve<TcpListener>,
    ProtocolError<P>: From<overseerd_app::Error>,
{
    /// Starts an application on an ephemeral loopback listener.
    pub async fn start(app: App<P>) -> Self {
        Self::start_with_guard(app, ()).await
    }
}

impl<P, G> TestServer<P, G>
where
    P: ProtocolPlugin + 'static,
    P::Protocol: Serve<TcpListener>,
    ProtocolError<P>: From<overseerd_app::Error>,
{
    /// Starts an application while retaining an associated fixture guard.
    pub async fn start_with_guard(app: App<P>, guard: G) -> Self {
        let shutdown = app.shutdown_handle();
        let task = LoopbackTask::spawn("test server", |listener| app.serve(listener)).await;

        Self {
            shutdown,
            task,
            _guard: guard,
            _plugin: PhantomData,
        }
    }

    /// Returns the bound loopback address.
    pub fn address(&self) -> SocketAddr {
        self.task.address()
    }

    /// Requests graceful shutdown and propagates task or server failure.
    pub async fn shutdown(mut self) {
        self.shutdown.shutdown();
        self.task
            .join()
            .await
            .unwrap_or_else(|error| panic!("test server failed: {error}"));
    }
}
