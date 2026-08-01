use std::marker::PhantomData;
use std::net::SocketAddr;

use overseerd_app::{
    App, PreparedProtocol, ProtocolDefinition, ProtocolRuntime, Serve, ShutdownHandle,
};
use tokio::net::TcpListener;

use crate::LoopbackTask;

type Runtime<D> = <<D as ProtocolDefinition>::Prepared as PreparedProtocol>::Runtime;
type ProtocolError<D> = <Runtime<D> as ProtocolRuntime>::Error;

/// Owns a protocol-generic loopback server and aborts it on incomplete cleanup.
pub struct TestServer<D: ProtocolDefinition> {
    shutdown: ShutdownHandle,
    task: LoopbackTask<Result<(), ProtocolError<D>>>,
    _definition: PhantomData<D>,
}

impl<D> TestServer<D>
where
    D: ProtocolDefinition,
    Runtime<D>: Serve<TcpListener>,
    ProtocolError<D>: From<overseerd_app::Error>,
{
    /// Starts an application while retaining an associated fixture guard.
    pub async fn start_with_guard<G>(app: App<D>, guard: G) -> Self
    where
        G: Send + 'static,
    {
        let shutdown = app.shutdown_handle();
        let task = LoopbackTask::spawn("test server", |listener| async move {
            let _guard = guard;

            app.serve(listener).await
        })
        .await;

        Self {
            shutdown,
            task,
            _definition: PhantomData,
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
