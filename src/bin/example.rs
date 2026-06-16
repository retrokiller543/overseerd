use std::{future::Future, pin::Pin, sync::Arc};

use overseer_core::{
    BoxedComponent, ComponentConstructionContext, ComponentDescriptor, ComponentScope,
    Daemon, DependencyDescriptor, OperationKind, ParameterDescriptor, ParameterKind,
    RpcCallContext, RpcDescriptor, RpcResponse, ServiceDescriptor, TypeDescriptor,
};

// ---------------------------------------------------------------------------
// Stand-in domain types — macros will generate these from annotated structs.
// ---------------------------------------------------------------------------

struct Config;
struct DatabasePool;
struct BackupRepository;
struct BackupService;
struct StartBackupInput;
struct JobId;
struct BackupStatus;
struct BackupSummary;

// ---------------------------------------------------------------------------
// Factories — macros will generate one per #[component].
// ---------------------------------------------------------------------------

fn config_factory<'a>(
    _: &'a mut ComponentConstructionContext,
) -> Pin<Box<dyn Future<Output = overseer_core::Result<BoxedComponent>> + Send + 'a>> {
    Box::pin(async {
        Ok(BoxedComponent {
            ty: TypeDescriptor::of::<Config>("Config"),
            value: Box::new(Arc::new(Config)),
        })
    })
}

fn database_pool_factory<'a>(
    ctx: &'a mut ComponentConstructionContext,
) -> Pin<Box<dyn Future<Output = overseer_core::Result<BoxedComponent>> + Send + 'a>> {
    Box::pin(async move {
        let _config = ctx.resolve::<Config>();

        Ok(BoxedComponent {
            ty: TypeDescriptor::of::<DatabasePool>("DatabasePool"),
            value: Box::new(Arc::new(DatabasePool)),
        })
    })
}

fn backup_repository_factory<'a>(
    ctx: &'a mut ComponentConstructionContext,
) -> Pin<Box<dyn Future<Output = overseer_core::Result<BoxedComponent>> + Send + 'a>> {
    Box::pin(async move {
        let _pool = ctx.resolve::<DatabasePool>();

        Ok(BoxedComponent {
            ty: TypeDescriptor::of::<BackupRepository>("BackupRepository"),
            value: Box::new(Arc::new(BackupRepository)),
        })
    })
}

// ---------------------------------------------------------------------------
// RPC handlers — macros will generate these from #[rpc] impl blocks.
// ---------------------------------------------------------------------------

fn start_backup_handler(
    _: RpcCallContext,
) -> Pin<Box<dyn Future<Output = overseer_core::Result<RpcResponse>> + Send>> {
    Box::pin(async { Ok(RpcResponse {}) })
}

fn backup_status_handler(
    _: RpcCallContext,
) -> Pin<Box<dyn Future<Output = overseer_core::Result<RpcResponse>> + Send>> {
    Box::pin(async { Ok(RpcResponse {}) })
}

fn list_backups_handler(
    _: RpcCallContext,
) -> Pin<Box<dyn Future<Output = overseer_core::Result<RpcResponse>> + Send>> {
    Box::pin(async { Ok(RpcResponse {}) })
}

// ---------------------------------------------------------------------------
// Static descriptors — macros will emit these into the binary via inventory::submit!
// ---------------------------------------------------------------------------

static CONFIG: ComponentDescriptor = ComponentDescriptor {
    id: "config",
    name: "Config",
    ty: TypeDescriptor::of::<Config>("Config"),
    scope: ComponentScope::Singleton,
    dependencies: &[],
    factory: config_factory,
};

static DATABASE_POOL_DEPS: [DependencyDescriptor; 1] = [DependencyDescriptor {
    name: "Config",
    ty: TypeDescriptor::of::<Config>("Config"),
    optional: false,
}];

static DATABASE_POOL: ComponentDescriptor = ComponentDescriptor {
    id: "database_pool",
    name: "DatabasePool",
    ty: TypeDescriptor::of::<DatabasePool>("DatabasePool"),
    scope: ComponentScope::Singleton,
    dependencies: &DATABASE_POOL_DEPS,
    factory: database_pool_factory,
};

static BACKUP_REPO_DEPS: [DependencyDescriptor; 1] = [DependencyDescriptor {
    name: "DatabasePool",
    ty: TypeDescriptor::of::<DatabasePool>("DatabasePool"),
    optional: false,
}];

static BACKUP_REPO: ComponentDescriptor = ComponentDescriptor {
    id: "backup_repository",
    name: "BackupRepository",
    ty: TypeDescriptor::of::<BackupRepository>("BackupRepository"),
    scope: ComponentScope::Singleton,
    dependencies: &BACKUP_REPO_DEPS,
    factory: backup_repository_factory,
};

static BACKUP_SERVICE_RPCS: [RpcDescriptor; 3] = [
    RpcDescriptor {
        name: "start_backup",
        operation: OperationKind::Command,
        parameters: &[ParameterDescriptor {
            name: "input",
            kind: ParameterKind::Payload,
            ty: TypeDescriptor::of::<StartBackupInput>("StartBackupInput"),
        }],
        output: TypeDescriptor::of::<JobId>("JobId"),
        handler: start_backup_handler,
    },
    RpcDescriptor {
        name: "backup_status",
        operation: OperationKind::Query,
        parameters: &[ParameterDescriptor {
            name: "job_id",
            kind: ParameterKind::Payload,
            ty: TypeDescriptor::of::<JobId>("JobId"),
        }],
        output: TypeDescriptor::of::<BackupStatus>("BackupStatus"),
        handler: backup_status_handler,
    },
    RpcDescriptor {
        name: "list_backups",
        operation: OperationKind::Query,
        parameters: &[],
        output: TypeDescriptor::of::<BackupSummary>("Vec<BackupSummary>"),
        handler: list_backups_handler,
    },
];

static BACKUP_SERVICE_DESC: ServiceDescriptor = ServiceDescriptor {
    id: "backup_service",
    name: "BackupService",
    ty: TypeDescriptor::of::<BackupService>("BackupService"),
    version: Some("0.1"),
    rpcs: &BACKUP_SERVICE_RPCS,
};

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

#[tokio::main]
async fn main() -> overseer_core::Result<()> {
    let daemon = Daemon::builder("backup-daemon")
        .component(&CONFIG)
        .component(&DATABASE_POOL)
        .component(&BACKUP_REPO)
        .service(&BACKUP_SERVICE_DESC)
        .build()
        .await?;

    println!("{}", daemon.registry);

    println!("Routes ({}):", daemon.router.route_count());
    let mut paths: Vec<&str> = daemon.router.paths().collect();
    paths.sort();
    for path in &paths {
        println!("  {path}");
    }

    println!();
    println!("Container:");
    println!("  Config:           {}", daemon.container.get::<Config>().is_some());
    println!("  DatabasePool:     {}", daemon.container.get::<DatabasePool>().is_some());
    println!("  BackupRepository: {}", daemon.container.get::<BackupRepository>().is_some());

    Ok(())
}
