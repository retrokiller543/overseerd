use std::any::TypeId;
use std::collections::HashMap;
use std::sync::Arc;

use overseerd_core::{ResolverSet, Scope, ScopeId, StaticScope, TypeDescriptor};
use overseerd_di::{BoxedComponent, ScopeContainer, ScopeRegistry};
use overseerd_hooks::HookManager;

use super::*;
use crate::{Error, ScopeBoundary, ScopeParent, ScopeTopology};

const SESSION_ID: ScopeId = overseerd_core::namespaced_id!(ScopeId, "test/session");
const REQUEST_ID: ScopeId = overseerd_core::namespaced_id!(ScopeId, "test/request");
const OTHER_ID: ScopeId = overseerd_core::namespaced_id!(ScopeId, "test/other");

struct Session;
struct Request;

impl StaticScope for Session {
    const ID: ScopeId = SESSION_ID;
    const RANK: u8 = 100;
    const NAME: &'static str = "Session";
}

impl StaticScope for Request {
    const ID: ScopeId = REQUEST_ID;
    const RANK: u8 = 50;
    const NAME: &'static str = "Request";
}

static SESSION: Session = Session;
static REQUEST: Request = Request;
static OTHER: TestScope = TestScope::new(OTHER_ID, "Other", 25);
static REQUEST_ALIAS: TestScope = TestScope::new(REQUEST_ID, "Caller Metadata", 1);
static BOUNDARIES: [ScopeBoundary; 2] = [
    ScopeBoundary::new(&SESSION, ScopeParent::Root),
    ScopeBoundary::new(&REQUEST, ScopeParent::Boundary(SESSION_ID)),
];

/// A scope with test-controlled stable identity and metadata.
#[derive(Debug)]
struct TestScope {
    id: ScopeId,
    name: &'static str,
    rank: u8,
}

impl TestScope {
    const fn new(id: ScopeId, name: &'static str, rank: u8) -> Self {
        Self { id, name, rank }
    }
}

impl Scope for TestScope {
    fn id(&self) -> ScopeId {
        self.id
    }

    fn rank(&self) -> u8 {
        self.rank
    }

    fn name(&self) -> &'static str {
        self.name
    }
}

async fn build_runtime(
    seed_destinations: HashMap<TypeId, SeedDestination>,
) -> (AppRuntime, Arc<ScopeRegistry>) {
    let registry = Arc::new(ScopeRegistry::new(
        HashMap::new(),
        HashMap::new(),
        Vec::new(),
        HashMap::new(),
    ));
    let root =
        ScopeContainer::build_root(&[], Vec::new(), ResolverSet::new(), Arc::clone(&registry))
            .await
            .expect("test root builds");
    let topology = ScopeTopology::new(&BOUNDARIES)
        .prepare()
        .expect("test topology prepares");
    let runtime = AppRuntime::new(
        Arc::from("test"),
        root,
        Arc::clone(&registry),
        RuntimeScopePlan::new(
            Arc::new(topology),
            Arc::new(HashMap::from([
                (SESSION_ID, Vec::new()),
                (REQUEST_ID, Vec::new()),
            ])),
            Arc::new(seed_destinations),
        ),
        Arc::from([]),
        HookManager::new(Vec::new()),
    );

    (runtime, registry)
}

fn seed<T: Send + Sync + 'static>() -> BoxedComponent {
    BoxedComponent {
        ty: TypeDescriptor::of::<T>(std::any::type_name::<T>()),
        value: Box::new(()),
    }
}

fn expect_app_error<T>(result: crate::Result<T>, message: &str) -> Error {
    match result {
        Ok(_) => panic!("{message}"),
        Err(error) => error,
    }
}

#[tokio::test]
async fn open_rejects_undeclared_and_wrong_parent_boundaries() {
    let (runtime, _) = build_runtime(HashMap::new()).await;

    let undeclared = expect_app_error(
        runtime
            .open_scope(&OTHER, Arc::clone(runtime.root()), Vec::new())
            .await,
        "undeclared boundary was accepted",
    );
    let wrong_parent = expect_app_error(
        runtime
            .open_scope(&REQUEST, Arc::clone(runtime.root()), Vec::new())
            .await,
        "request opened without its session parent",
    );

    assert!(matches!(
        undeclared,
        Error::UndeclaredScopeOpen { scope: OTHER_ID }
    ));
    assert!(matches!(
        wrong_parent,
        Error::InvalidScopeParent {
            child: REQUEST_ID,
            expected: SESSION_ID,
            actual: <overseerd_core::Singleton as StaticScope>::ID,
        }
    ));
}

#[tokio::test]
async fn open_rejects_foreign_runtime_and_noncanonical_root_parents() {
    let (runtime, registry) = build_runtime(HashMap::new()).await;
    let (foreign, _) = build_runtime(HashMap::new()).await;
    let alternate_root = ScopeContainer::build_root(&[], Vec::new(), ResolverSet::new(), registry)
        .await
        .expect("alternate root builds");

    let foreign_error = expect_app_error(
        runtime
            .open_scope(&SESSION, Arc::clone(foreign.root()), Vec::new())
            .await,
        "foreign runtime root was accepted",
    );
    let alternate_error = expect_app_error(
        runtime
            .open_scope(&SESSION, alternate_root, Vec::new())
            .await,
        "same-registry noncanonical root was accepted",
    );

    assert!(matches!(
        foreign_error,
        Error::ForeignScopeParent {
            child: SESSION_ID,
            parent: <overseerd_core::Singleton as StaticScope>::ID,
        }
    ));
    assert!(matches!(
        alternate_error,
        Error::InvalidScopeParent {
            child: SESSION_ID,
            expected: <overseerd_core::Singleton as StaticScope>::ID,
            actual: <overseerd_core::Singleton as StaticScope>::ID,
        }
    ));
}

#[tokio::test]
async fn open_uses_declared_scope_metadata_and_retains_empty_boundaries() {
    let (runtime, _) = build_runtime(HashMap::new()).await;
    let session = runtime
        .open_scope(&SESSION, Arc::clone(runtime.root()), Vec::new())
        .await
        .expect("session opens");
    let request = runtime
        .open_scope(&REQUEST_ALIAS, session, Vec::new())
        .await
        .expect("request opens by stable identity");

    assert_eq!(request.scope().id(), REQUEST_ID);
    assert_eq!(request.scope().name(), REQUEST.name());
    assert_eq!(request.scope().rank(), REQUEST.rank());
    assert!(!Arc::ptr_eq(&request, runtime.root()));
}

#[tokio::test]
async fn open_validates_seed_registration_destination_and_uniqueness() {
    /// A seed declared for the session boundary.
    struct SessionSeed;
    /// A seed declared for the request boundary.
    struct RequestSeed;
    /// A seed absent from the application registry.
    struct UnknownSeed;

    let destinations = HashMap::from([
        (
            TypeId::of::<SessionSeed>(),
            SeedDestination {
                scope: SESSION_ID,
                type_name: std::any::type_name::<SessionSeed>(),
            },
        ),
        (
            TypeId::of::<RequestSeed>(),
            SeedDestination {
                scope: REQUEST_ID,
                type_name: std::any::type_name::<RequestSeed>(),
            },
        ),
    ]);
    let (runtime, _) = build_runtime(destinations).await;

    let wrong_destination = expect_app_error(
        runtime
            .open_scope(
                &SESSION,
                Arc::clone(runtime.root()),
                vec![seed::<RequestSeed>()],
            )
            .await,
        "wrong-destination seed was accepted",
    );
    let unregistered = expect_app_error(
        runtime
            .open_scope(
                &SESSION,
                Arc::clone(runtime.root()),
                vec![seed::<UnknownSeed>()],
            )
            .await,
        "unregistered seed was accepted",
    );
    let duplicate = expect_app_error(
        runtime
            .open_scope(
                &SESSION,
                Arc::clone(runtime.root()),
                vec![seed::<SessionSeed>(), seed::<SessionSeed>()],
            )
            .await,
        "duplicate seed type was accepted",
    );

    assert!(matches!(
        wrong_destination,
        Error::InvalidSeedDestination {
            expected: REQUEST_ID,
            actual: SESSION_ID,
            ..
        }
    ));
    assert!(matches!(
        unregistered,
        Error::UnregisteredSeed {
            scope: SESSION_ID,
            ..
        }
    ));
    assert!(matches!(
        duplicate,
        Error::DuplicateSeedType {
            scope: SESSION_ID,
            ..
        }
    ));
}
