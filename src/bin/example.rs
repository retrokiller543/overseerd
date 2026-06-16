use std::{future::Future, pin::Pin};

use overseer_core::{
    BoxedComponent, ComponentConstructionContext, ComponentDescriptor, ComponentScope,
    DependencyDescriptor, OperationKind, ParameterDescriptor, ParameterKind, RpcCallContext,
    RpcDescriptor, RpcResponse, Registry, ServiceDescriptor, TypeDescriptor,
};

fn unimplemented_factory<'a>(
    _: &'a mut ComponentConstructionContext,
) -> Pin<Box<dyn Future<Output = overseer_core::Result<BoxedComponent>> + Send + 'a>> {
    Box::pin(async { todo!("factory not implemented in example") })
}

fn unimplemented_handler(
    _: RpcCallContext,
) -> Pin<Box<dyn Future<Output = overseer_core::Result<RpcResponse>> + Send>> {
    Box::pin(async { todo!("handler not implemented in example") })
}

// Stand-in types — macros will use the real application types here.
struct Config;
struct DatabasePool;
struct BackupRepository;
struct BackupService;
struct StartBackupInput;
struct JobId;
struct BackupStatus;
struct BackupSummary;

static CONFIG: ComponentDescriptor = ComponentDescriptor {
    id: "config",
    name: "Config",
    ty: TypeDescriptor::of::<Config>("Config"),
    scope: ComponentScope::Singleton,
    dependencies: &[],
    factory: unimplemented_factory,
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
    factory: unimplemented_factory,
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
    factory: unimplemented_factory,
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
        handler: unimplemented_handler,
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
        handler: unimplemented_handler,
    },
    RpcDescriptor {
        name: "list_backups",
        operation: OperationKind::Query,
        parameters: &[],
        output: TypeDescriptor::of::<BackupSummary>("Vec<BackupSummary>"),
        handler: unimplemented_handler,
    },
];

static BACKUP_SERVICE_DESC: ServiceDescriptor = ServiceDescriptor {
    id: "backup_service",
    name: "BackupService",
    ty: TypeDescriptor::of::<BackupService>("BackupService"),
    version: Some("0.1"),
    rpcs: &BACKUP_SERVICE_RPCS,
};

fn main() {
    let registry = Registry {
        components: vec![&CONFIG, &DATABASE_POOL, &BACKUP_REPO],
        services: vec![&BACKUP_SERVICE_DESC],
    };

    match registry.validate() {
        Ok(()) => println!("Registry validation passed.\n"),
        Err(e) => {
            eprintln!("Registry validation failed: {e}");
            std::process::exit(1);
        }
    }

    println!("=== describe ===\n{}", registry.describe());
    println!("=== debug ===\n{:#?}", registry);
}