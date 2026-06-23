use std::{future::Future, pin::Pin};

/// The outcome of a single health check poll.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HealthStatus {
    Healthy,
    /// The component is operational but impaired; watchdog pings still fire.
    Degraded,
    /// The component is non-functional; watchdog pings are suppressed so the
    /// service manager can restart the process.
    Unhealthy,
}

impl HealthStatus {
    pub fn is_unhealthy(&self) -> bool {
        matches!(self, HealthStatus::Unhealthy)
    }
}

/// A component that reports its operational health to the framework.
///
/// Implement this trait on a `#[component(provide = dyn HealthCheck)]` to
/// register it as a health provider. The framework polls all singleton health
/// checks on the watchdog interval and uses the aggregate result to decide
/// whether to send a watchdog ping to the service manager.
///
/// Only singleton (and transient) components may provide `dyn HealthCheck`
/// — connection- or request-scoped components outlive no watchdog interval and
/// are rejected at build time.
///
/// # Example
///
/// ```rust,ignore
/// #[component(provide = dyn HealthCheck)]
/// struct DatabasePool { pool: Arc<Pool> }
///
/// impl HealthCheck for DatabasePool {
///     fn name(&self) -> &str { "database" }
///
///     fn check(&self) -> overseerd::HealthCheckFuture {
///         let pool = self.pool.clone();
///         Box::pin(async move {
///             if pool.is_alive().await {
///                 HealthStatus::Healthy
///             } else {
///                 HealthStatus::Unhealthy
///             }
///         })
///     }
/// }
/// ```
pub trait HealthCheck: Send + Sync + 'static {
    /// A human-readable name used in STATUS messages to the service manager.
    /// Defaults to the fully-qualified type name.
    fn name(&self) -> &str {
        std::any::type_name::<Self>()
    }

    /// Checks the component's health. Must not block — use async I/O.
    ///
    /// The returned future is `'static` (no borrow of `self`); clone any
    /// internal `Arc` handles before boxing if the future needs data from
    /// `self`.
    fn check(&self) -> Pin<Box<dyn Future<Output = HealthStatus> + Send>>;
}

/// Convenience type alias for the boxed future returned by [`HealthCheck::check`].
pub type HealthCheckFuture = Pin<Box<dyn Future<Output = HealthStatus> + Send>>;
