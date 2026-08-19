//! Homeledger's transaction RPC service and its injected runtime dependencies.

use std::sync::Arc;

use crate::audit::AuditSink;
use crate::components::{DatabaseConfig, DatabaseConnection, LedgerConfig};
use crate::operations::OperationsRegistry;
use crate::protocol::AuditPolicyConfig;
use serde::{Deserialize, Serialize};
use upwell::daemon::{Inject, Payload, handlers, service};
use upwell::{Cfg, CfgNext, ConfigReload, Dep, HookOutcome, ServerConfig, ShutdownHandle};

/// A request to record one categorized household transaction.
#[derive(Serialize, Deserialize)]
pub struct RecordTransactionRequest {
    pub amount_minor: i64,
    pub category: String,
}

/// A recorded Homeledger transaction.
#[derive(Serialize, Deserialize)]
pub struct RecordTransactionResponse {
    pub transaction_id: u64,
    pub household: String,
    pub currency: String,
    pub audited_by: Vec<String>,
    pub reader_pool: u16,
    pub writer_pool: u16,
    pub audit_retention_days: u16,
    pub audit_review_required: bool,
}

/// The transaction service demonstrates config, provider, plugin, and framework injection.
#[service(id = "ledger", version = "0.1")]
pub struct LedgerService {
    /// A config value bound at `homeledger.ledger`, injected by property path as `Cfg<T>`.
    #[config("homeledger.ledger")]
    config: Cfg<LedgerConfig>,
    /// The same `DatabaseConfig` type bound at two paths — selected here by path, proving
    /// configs key on the path rather than the type.
    #[config("homeledger.database.reader")]
    reader: Cfg<DatabaseConfig>,
    #[config("homeledger.database.writer")]
    writer: Cfg<DatabaseConfig>,
    /// Every configured audit sink.
    audit_sinks: Vec<Arc<dyn AuditSink>>,
    /// Runtime marker contributed by the statically installed operations plugin.
    operations: Arc<OperationsRegistry>,
    /// Audit policy supplied by the selected protocol-default replacement plugin.
    #[config("homeledger.audit")]
    audit_policy: Cfg<AuditPolicyConfig>,
    /// The framework [`ServerConfig`] builtin, bound explicitly at `homeledger.server`.
    #[config("homeledger.server")]
    server: Cfg<ServerConfig>,
    /// The framework-seeded shutdown handle, injected by value.
    shutdown: ShutdownHandle,
}

#[handlers]
impl LedgerService {
    /// Records a household transaction and sends it to every audit destination.
    #[rpc]
    async fn record_transaction(
        &self,
        Payload(request): Payload<RecordTransactionRequest>,
        Inject(database): Inject<Dep<DatabaseConnection>>,
    ) -> RecordTransactionResponse {
        let transaction_id = database.get().record_transaction();
        let config = self.config.snapshot();
        let reader = self.reader.snapshot();
        let writer = self.writer.snapshot();
        let server = self.server.snapshot();
        let audit_policy = self.audit_policy.snapshot();

        let mut audited_by: Vec<String> = self
            .audit_sinks
            .iter()
            .map(|sink| sink.destination().to_string())
            .collect();

        audited_by.sort_unstable();

        let _ = (
            &self.operations,
            request.amount_minor,
            &request.category,
            &reader.url,
            &writer.url,
            &self.shutdown,
            &server.bind,
            server.port,
        );

        RecordTransactionResponse {
            transaction_id,
            household: config.household.clone(),
            currency: config.currency.clone(),
            audited_by,
            reader_pool: reader.pool_size,
            writer_pool: writer.pool_size,
            audit_retention_days: audit_policy.retention_days,
            audit_review_required: audit_policy.require_review_ticket,
        }
    }

    /// Reacts to a reload of the ledger config: it receives the proposed household
    /// before the swap is committed and reports that it applied cleanly. Fires only when
    /// `homeledger.ledger` actually changes.
    #[hook(ConfigReload)]
    async fn on_ledger_reload(
        &self,
        #[config("homeledger.ledger")] next: CfgNext<LedgerConfig>,
    ) -> upwell::daemon::Result<HookOutcome> {
        tracing::info!(
            target: "homeledger::config",
            household = %next.household,
            currency = %next.currency,
            "ledger config reloaded"
        );

        Ok(HookOutcome::Reloaded)
    }
}
