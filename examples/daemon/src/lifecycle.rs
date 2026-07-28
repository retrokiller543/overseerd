//! Homeledger work performed around generated host construction.

use crate::components::Database;
use crate::operations::OperationsArgs;
use crate::protocol::{ComplianceAuditArgs, HomeledgerRpc};
use overseerd::{App, BootstrapContext, DiError, resolve_host_dependency};

#[cfg(test)]
use overseerd::{ColorChoice, LogFormat};

#[cfg(test)]
use std::sync::Mutex;

#[cfg(test)]
static TEST_BOOTSTRAP: Mutex<Option<TestBootstrapState>> = Mutex::new(None);

/// Resolved bootstrap values captured by generated-host integration tests.
#[cfg(test)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TestBootstrapState {
    pub(crate) config_path: String,
    pub(crate) profiles: Vec<String>,
    pub(crate) log: String,
    pub(crate) log_format: LogFormat,
    pub(crate) color: ColorChoice,
}

#[cfg(test)]
pub(crate) fn take_test_bootstrap() -> TestBootstrapState {
    TEST_BOOTSTRAP
        .lock()
        .expect("test bootstrap capture lock is available")
        .take()
        .expect("generated host captured bootstrap state")
}

/// Bootstrap provenance retained for diagnostics throughout the host lifecycle.
pub struct StartupProvenance {
    pub(crate) config_path: String,
    pub(crate) profiles: Vec<String>,
    pub(crate) operator: String,
    pub(crate) audit_review_ticket: String,
}

/// Database readiness information produced after application construction.
pub struct BuildReadiness {
    pub(crate) schema_version: u64,
}

/// Captures generated bootstrap choices before tracing is finalized or a builder exists.
pub async fn setup(
    mut context: BootstrapContext,
) -> Result<BootstrapContext, std::convert::Infallible> {
    #[cfg(test)]
    if context.mode().is_run()
        && let Some(bootstrap) = context.bootstrap()
    {
        let snapshot = TestBootstrapState {
            config_path: bootstrap.config_path().display().to_string(),
            profiles: bootstrap.profiles().to_vec(),
            log: bootstrap.logging().level.clone(),
            log_format: bootstrap.logging().format,
            color: bootstrap.color(),
        };

        *TEST_BOOTSTRAP
            .lock()
            .expect("test bootstrap capture lock is available") = Some(snapshot);
    }

    let config_path = context
        .bootstrap()
        .map(|bootstrap| bootstrap.config_path().display().to_string())
        .unwrap_or_else(|| String::from("<direct-host>"));
    let profiles = context
        .bootstrap()
        .map(|bootstrap| bootstrap.profiles().to_vec())
        .unwrap_or_default();
    let operator = context
        .get::<OperationsArgs>()
        .map(|arguments| arguments.operator.clone())
        .unwrap_or_else(|| String::from("direct-host"));
    let audit_review_ticket = context
        .get::<ComplianceAuditArgs>()
        .map(|arguments| arguments.audit_review_ticket.clone())
        .unwrap_or_else(|| String::from("direct-host"));

    let provenance = StartupProvenance {
        config_path,
        profiles,
        operator,
        audit_review_ticket,
    };

    tracing::debug!(
        target: "homeledger::lifecycle",
        config_path = %provenance.config_path,
        profiles = ?provenance.profiles,
        operator = %provenance.operator,
        audit_review_ticket = %provenance.audit_review_ticket,
        "Homeledger bootstrap choices captured"
    );
    context.insert(provenance);

    Ok(context)
}

/// Verifies database readiness after DI and the RPC runtime have been constructed.
pub async fn after_build(
    context: &mut BootstrapContext,
    app: App<HomeledgerRpc>,
) -> Result<App<HomeledgerRpc>, DiError> {
    let database =
        resolve_host_dependency::<HomeledgerRpc, Database>(&app, "Homeledger after_build").await?;
    let schema_version = database.verify_schema();

    let readiness = BuildReadiness { schema_version };

    tracing::info!(
        target: "homeledger::lifecycle",
        schema_version = readiness.schema_version,
        "Homeledger database schema verified"
    );
    context.insert(readiness);

    Ok(app)
}
