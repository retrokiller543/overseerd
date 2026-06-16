use std::sync::Arc;

use tracing::{debug, error, info, instrument, trace, warn};
use overseer_transport::{CallResult, Connection, OutgoingResponse, Respond, Transport};

use crate::{
    connection::{ConnectionHandler, ConnectionInfo},
    container::Container,
    descriptors::{ComponentDescriptor, RpcCallContext, RpcResponse, ServiceDescriptor},
    lifecycle::{ShutdownHandle, ShutdownSignal},
    registry::Registry,
    router::RpcRouter,
};

/// Assembles a Daemon from an explicit set of components and services.
pub struct DaemonBuilder {
    name: String,
    registry: Registry,
    connection_handlers: Vec<Box<dyn ConnectionHandler>>,
}

impl DaemonBuilder {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            registry: Registry::default(),
            connection_handlers: Vec::new(),
        }
    }

    /// Populates the registry from all `inventory::submit!` entries in the binary.
    pub fn auto_discover(mut self) -> Self {
        self.registry = Registry::collect();

        self
    }

    pub fn component(mut self, descriptor: &'static ComponentDescriptor) -> Self {
        self.registry.components.push(descriptor);

        self
    }

    pub fn service(mut self, descriptor: &'static ServiceDescriptor) -> Self {
        self.registry.services.push(descriptor);

        self
    }

    /// Registers a handler called once per accepted connection to populate
    /// connection-scoped context. Handlers run in registration order.
    pub fn connection_handler<H: ConnectionHandler>(mut self, handler: H) -> Self {
        self.connection_handlers.push(Box::new(handler));

        self
    }

    /// Validates the registry, resolves all components, and builds a ready-to-run Daemon.
    pub async fn build(self) -> crate::Result<Daemon> {
        debug!(daemon = %self.name, "building daemon");

        self.registry.validate()?;

        let container = Container::build(&self.registry).await?;
        let router = RpcRouter::from_registry(&self.registry);
        let shutdown = ShutdownSignal::new();

        info!(
            daemon = %self.name,
            components = self.registry.components.len(),
            services = self.registry.services.len(),
            "daemon built"
        );

        Ok(Daemon {
            name: self.name,
            registry: self.registry,
            container,
            router,
            shutdown,
            connection_handlers: self.connection_handlers,
        })
    }
}

/// A fully assembled daemon, ready to accept connections and dispatch RPC calls.
pub struct Daemon {
    pub name: String,
    pub registry: Registry,
    pub container: Container,
    pub router: RpcRouter,
    shutdown: ShutdownSignal,
    connection_handlers: Vec<Box<dyn ConnectionHandler>>,
}

impl Daemon {
    pub fn builder(name: impl Into<String>) -> DaemonBuilder {
        DaemonBuilder::new(name)
    }

    /// Returns a handle that can trigger graceful shutdown from any spawned task.
    pub fn shutdown_handle(&self) -> ShutdownHandle {
        self.shutdown.handle()
    }

    /// Serves RPC calls from `transport` until ctrl-c or a shutdown signal.
    ///
    /// One task is spawned per accepted connection. Within each connection task,
    /// calls are dispatched sequentially — the response is sent before the next
    /// call is read.
    pub async fn serve<T>(self, mut transport: T) -> crate::Result<()>
    where
        T: Transport,
        T::Connection: 'static,
    {
        let transport_name = std::any::type_name::<T>();

        info!(daemon = %self.name, transport = transport_name, "serve starting");

        let router = Arc::new(self.router);
        let handlers = Arc::new(self.connection_handlers);
        let mut shutdown = self.shutdown;

        loop {
            tokio::select! {
                result = transport.accept() => {
                    match result {
                        Ok(conn) => {
                            debug!(peer = ?conn.peer().addr, "connection accepted, spawning task");

                            let router = Arc::clone(&router);
                            let handlers = Arc::clone(&handlers);

                            tokio::spawn(async move {
                                serve_connection(conn, router, handlers).await;
                            });
                        }

                        Err(e) => {
                            error!(error = %e, "transport accept failed");
                            return Err(e.into());
                        }
                    }
                }

                _ = tokio::signal::ctrl_c() => {
                    info!("ctrl-c received, shutting down");
                    break;
                }

                _ = shutdown.wait() => {
                    info!("shutdown signal received");
                    break;
                }
            }
        }

        info!(transport = transport_name, "serve stopped");

        Ok(())
    }

    /// Waits for ctrl-c or a shutdown signal without serving any transport.
    pub async fn run(self) -> crate::Result<()> {
        let mut shutdown = self.shutdown;

        tokio::select! {
            _ = tokio::signal::ctrl_c() => {},
            _ = shutdown.wait() => {},
        }

        Ok(())
    }
}

#[instrument(
    skip_all,
    fields(peer = ?conn.peer().addr),
    name = "connection"
)]
async fn serve_connection<C: Connection>(
    mut conn: C,
    router: Arc<RpcRouter>,
    handlers: Arc<Vec<Box<dyn ConnectionHandler>>>,
) {
    debug!("connection established");

    let mut info = ConnectionInfo::new(conn.peer().clone());

    for (i, handler) in handlers.iter().enumerate() {
        trace!(index = i, "running connection handler");

        if let Err(e) = handler.on_connect(&mut info).await {
            error!(error = %e, "connection handler failed, closing");
            return;
        }
    }

    let info = Arc::new(info);

    debug!("connection ready");

    loop {
        match conn.recv().await {
            Ok(Some((call, responder))) => {
                let call_id = call.id;
                let path = call.path.clone();

                debug!(id = call_id, %path, "dispatching call");

                let ctx = RpcCallContext {
                    id: call.id,
                    payload: call.payload,
                    connection: Arc::clone(&info),
                };

                let outcome = match router.dispatch(&call.path, ctx).await {
                    Ok(RpcResponse { payload }) => {
                        debug!(id = call_id, %path, "call succeeded");
                        CallResult::Ok(payload)
                    }

                    Err(e) => {
                        warn!(id = call_id, %path, error = %e, "call returned error");
                        CallResult::Err(e.to_string())
                    }
                };

                let response = OutgoingResponse { id: call_id, outcome };

                if let Err(e) = responder.respond(response).await {
                    warn!(id = call_id, error = %e, "failed to send response");
                    break;
                }
            }

            Ok(None) => {
                debug!("connection closed by peer");
                break;
            }

            Err(e) => {
                warn!(error = %e, "connection error");
                break;
            }
        }
    }

    debug!("connection ended");
}