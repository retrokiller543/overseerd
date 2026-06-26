//! A service tying the DI surface together: a runtime config (`Dynamic`), a
//! by-value pool, the primary notifier, every notifier, and the notifiers keyed
//! by channel — then an RPC that uses them.

use std::collections::HashMap;
use std::sync::Arc;

use crate::components::{Config, DbConfig, DbConnection};
use crate::notifiers::Notifier;
use overseerd::{
    Cfg, CfgNext, ConfigReload, Dep, HookOutcome, Inject, Payload, ServerConfig, ShutdownHandle,
    handlers, service,
};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
pub struct NotifyRequest {
    pub message: String,
}

#[derive(Serialize, Deserialize)]
pub struct NotifyResponse {
    pub greeting: String,
    pub delivered_to: Vec<String>,
    pub query_count: u64,
    pub reader_pool: u16,
    pub writer_pool: u16,
}

/// Each field shows a different injection shape resolved from the container.
#[service(id = "notifications", version = "0.1")]
pub struct Notifications {
    /// A config value bound at `app.greet`, injected by property path as `Cfg<T>`.
    #[config("app.greet")]
    config: Cfg<Config>,
    /// The same `DbConfig` type bound at two paths — selected here by path, proving
    /// configs key on the path rather than the type.
    #[config("app.db.reader")]
    reader: Cfg<DbConfig>,
    #[config("app.db.writer")]
    writer: Cfg<DbConfig>,
    /// The primary `dyn Notifier` (`Email`).
    default: Arc<dyn Notifier>,
    /// Every provider of `dyn Notifier`.
    all: Vec<Arc<dyn Notifier>>,
    /// Providers keyed by qualifier (`"email"`, `"sms"`, `"push"`).
    by_channel: HashMap<String, Arc<dyn Notifier>>,
    /// The framework [`ServerConfig`] builtin, bound explicitly at `app.server`.
    #[config("app.server")]
    server: Cfg<ServerConfig>,
    /// The framework-seeded shutdown handle, injected by value.
    shutdown: ShutdownHandle,
}

#[handlers]
impl Notifications {
    /// Broadcasts to every channel, stamping the configured greeting and the
    /// running query count from the shared by-value pool.
    #[rpc]
    async fn notify(
        &self,
        Payload(req): Payload<NotifyRequest>,
        Inject(db): Inject<Dep<DbConnection>>,
    ) -> NotifyResponse {
        let count = db.get().record_query();
        let config = self.config.snapshot();
        let reader = self.reader.snapshot();
        let writer = self.writer.snapshot();
        let server = self.server.snapshot();

        let mut delivered: Vec<String> = self.all.iter().map(|n| n.channel().to_string()).collect();
        delivered.sort_unstable();

        let _ = (
            &self.default,
            &self.by_channel,
            &req.message,
            &reader.url,
            &writer.url,
            &self.shutdown,
            &server.bind,
            server.port,
        );

        NotifyResponse {
            greeting: config.greeting.clone(),
            delivered_to: delivered,
            query_count: count,
            reader_pool: reader.pool_size,
            writer_pool: writer.pool_size,
        }
    }

    /// Reacts to a reload of the `app.greet` config: it receives the proposed greeting
    /// before the swap is committed and reports that it applied cleanly. Fires only when
    /// `app.greet` actually changes.
    #[hook(ConfigReload)]
    async fn on_greet_reload(
        &self,
        #[config("app.greet")] next: CfgNext<Config>,
    ) -> overseerd::Result<HookOutcome> {
        tracing::info!(
            target: "overseerd::example",
            greeting = %next.greeting,
            "greeting config reloaded"
        );

        Ok(HookOutcome::Reloaded)
    }
}
