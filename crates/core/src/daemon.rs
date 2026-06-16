use crate::{
    container::Container,
    descriptors::{ComponentDescriptor, ServiceDescriptor},
    lifecycle::{ShutdownHandle, ShutdownSignal},
    registry::Registry,
    router::RpcRouter,
};

/// Assembles a Daemon from an explicit set of components and services.
pub struct DaemonBuilder {
    name: String,
    registry: Registry,
}

impl DaemonBuilder {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            registry: Registry::default(),
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

    /// Validates the registry, resolves all components, and builds a ready-to-run Daemon.
    pub async fn build(self) -> crate::Result<Daemon> {
        self.registry.validate()?;

        let container = Container::build(&self.registry).await?;
        let router = RpcRouter::from_registry(&self.registry);
        let shutdown = ShutdownSignal::new();

        Ok(Daemon {
            name: self.name,
            registry: self.registry,
            container,
            router,
            shutdown,
        })
    }
}

/// A fully assembled daemon, ready to accept RPC calls and run until shutdown.
pub struct Daemon {
    pub name: String,
    pub registry: Registry,
    pub container: Container,
    pub router: RpcRouter,
    shutdown: ShutdownSignal,
}

impl Daemon {
    pub fn builder(name: impl Into<String>) -> DaemonBuilder {
        DaemonBuilder::new(name)
    }

    /// Returns a handle that can trigger graceful shutdown from any spawned task.
    pub fn shutdown_handle(&self) -> ShutdownHandle {
        self.shutdown.handle()
    }

    /// Runs the daemon until ctrl-c or an explicit shutdown signal is received.
    pub async fn run(self) -> crate::Result<()> {
        let mut shutdown = self.shutdown;

        tokio::select! {
            _ = tokio::signal::ctrl_c() => {},
            _ = shutdown.wait() => {},
        }

        Ok(())
    }
}
