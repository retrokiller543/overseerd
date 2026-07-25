use overseerd_core::{Scope, ScopeId, Singleton, StaticScope, Transient};

use super::*;

const HTTP_ID: ScopeId = overseerd_core::namespaced_id!(ScopeId, "test/http-request");
const CONNECTION_ID: ScopeId = overseerd_core::namespaced_id!(ScopeId, "test/websocket-connection");
const MESSAGE_ID: ScopeId = overseerd_core::namespaced_id!(ScopeId, "test/websocket-message");
const SIBLING_ID: ScopeId = overseerd_core::namespaced_id!(ScopeId, "test/sibling");
const MISSING_ID: ScopeId = overseerd_core::namespaced_id!(ScopeId, "test/missing");
const CYCLE_A_ID: ScopeId = overseerd_core::namespaced_id!(ScopeId, "test/cycle-a");
const CYCLE_B_ID: ScopeId = overseerd_core::namespaced_id!(ScopeId, "test/cycle-b");
const LONG_ID: ScopeId = overseerd_core::namespaced_id!(ScopeId, "test/long");

struct Http;
struct Connection;
struct Message;
struct Sibling;
struct CycleA;
struct CycleB;
struct Long;

macro_rules! test_scope {
    ($type:ty, $id:expr, $name:literal, $rank:expr) => {
        impl StaticScope for $type {
            const ID: ScopeId = $id;
            const RANK: u8 = $rank;
            const NAME: &'static str = $name;
        }
    };
}

test_scope!(Http, HTTP_ID, "Request", 100);
test_scope!(Connection, CONNECTION_ID, "Connection", 150);
test_scope!(Message, MESSAGE_ID, "Request", 50);
test_scope!(Sibling, SIBLING_ID, "Request", 50);
test_scope!(CycleA, CYCLE_A_ID, "Cycle A", 120);
test_scope!(CycleB, CYCLE_B_ID, "Cycle B", 110);
test_scope!(Long, LONG_ID, "Too Long", u8::MAX);

static HTTP: Http = Http;
static CONNECTION: Connection = Connection;
static MESSAGE: Message = Message;
static SIBLING: Sibling = Sibling;
static CYCLE_A: CycleA = CycleA;
static CYCLE_B: CycleB = CycleB;
static LONG: Long = Long;

static AXUM_BOUNDARIES: [ScopeBoundary; 3] = [
    ScopeBoundary::new(&MESSAGE, ScopeParent::Boundary(CONNECTION_ID)),
    ScopeBoundary::new(&HTTP, ScopeParent::Root),
    ScopeBoundary::new(&CONNECTION, ScopeParent::Root),
];

fn prepare(boundaries: &'static [ScopeBoundary]) -> PreparedScopeTopology {
    ScopeTopology::new(boundaries)
        .prepare()
        .expect("valid test topology")
}

#[test]
fn declaration_is_const_and_preparation_canonicalizes_boundaries() {
    let declaration = ScopeTopology::new(&AXUM_BOUNDARIES);
    let topology = declaration.prepare().expect("valid topology");
    let declared_ids: Vec<_> = declaration
        .boundaries()
        .iter()
        .map(|boundary| boundary.id())
        .collect();
    let ids: Vec<_> = topology
        .boundaries()
        .iter()
        .map(|boundary| boundary.id())
        .collect();
    let mut expected = vec![HTTP_ID, CONNECTION_ID, MESSAGE_ID];

    expected.sort();

    assert_eq!(declared_ids, [MESSAGE_ID, HTTP_ID, CONNECTION_ID]);
    assert_eq!(ids, expected);
}

#[test]
fn empty_topology_is_const_constructible_and_default() {
    const EMPTY: ScopeTopology = ScopeTopology::empty();

    assert!(EMPTY.boundaries().is_empty());
    assert!(ScopeTopology::default().boundaries().is_empty());
}

#[test]
fn duplicate_display_names_do_not_alias_stable_ids() {
    static BOUNDARIES: [ScopeBoundary; 2] = [
        ScopeBoundary::new(&HTTP, ScopeParent::Root),
        ScopeBoundary::new(&MESSAGE, ScopeParent::Root),
    ];

    let topology = prepare(&BOUNDARIES);

    assert_eq!(HTTP.name(), MESSAGE.name());
    assert_ne!(HTTP_ID, MESSAGE_ID);
    assert_eq!(
        topology.boundary(&HTTP_ID).map(|boundary| boundary.id()),
        Some(HTTP_ID)
    );
    assert_eq!(
        topology.boundary(&MESSAGE_ID).map(|boundary| boundary.id()),
        Some(MESSAGE_ID)
    );
}

#[test]
fn boundary_lookup_and_parent_queries_include_implicit_root() {
    let topology = prepare(&AXUM_BOUNDARIES);

    assert!(topology.contains(&HTTP_ID));
    assert!(!topology.contains(&Singleton.id()));
    assert_eq!(topology.parent_of(&HTTP_ID), Some(ScopeParent::Root));
    assert_eq!(
        topology.parent_of(&MESSAGE_ID),
        Some(ScopeParent::Boundary(CONNECTION_ID))
    );
    assert_eq!(topology.parent_of(&MISSING_ID), None);
    assert_eq!(
        topology.ancestors(&MESSAGE_ID).collect::<Vec<_>>(),
        [CONNECTION_ID, Singleton.id()]
    );
}

#[test]
fn reachability_follows_ancestry_instead_of_rank() {
    static BOUNDARIES: [ScopeBoundary; 4] = [
        ScopeBoundary::new(&HTTP, ScopeParent::Root),
        ScopeBoundary::new(&CONNECTION, ScopeParent::Root),
        ScopeBoundary::new(&MESSAGE, ScopeParent::Boundary(CONNECTION_ID)),
        ScopeBoundary::new(&SIBLING, ScopeParent::Root),
    ];

    let topology = prepare(&BOUNDARIES);
    let root = Singleton.id();

    assert!(topology.is_ancestor(&CONNECTION_ID, &MESSAGE_ID));
    assert!(topology.is_ancestor(&root, &MESSAGE_ID));
    assert!(!topology.is_ancestor(&MESSAGE_ID, &MESSAGE_ID));
    assert!(topology.is_reachable(&MESSAGE_ID, &MESSAGE_ID));
    assert!(topology.is_reachable(&MESSAGE_ID, &CONNECTION_ID));
    assert!(topology.is_reachable(&MESSAGE_ID, &root));
    assert!(!topology.is_reachable(&HTTP_ID, &CONNECTION_ID));
    assert!(!topology.is_reachable(&MESSAGE_ID, &SIBLING_ID));
    assert!(!topology.is_reachable(&MISSING_ID, &root));
    assert!(!topology.is_reachable(&MESSAGE_ID, &MISSING_ID));
}

#[test]
fn duplicate_ids_are_rejected() {
    static BOUNDARIES: [ScopeBoundary; 2] = [
        ScopeBoundary::new(&HTTP, ScopeParent::Root),
        ScopeBoundary::new(&HTTP, ScopeParent::Root),
    ];

    let error = ScopeTopology::new(&BOUNDARIES)
        .prepare()
        .expect_err("duplicate ids are invalid");

    assert_eq!(error, ScopeTopologyError::DuplicateId { id: HTTP_ID });
}

#[test]
fn root_and_transient_ids_are_reserved() {
    static ROOT_BOUNDARIES: [ScopeBoundary; 1] =
        [ScopeBoundary::new(&Singleton, ScopeParent::Root)];
    static TRANSIENT_BOUNDARIES: [ScopeBoundary; 1] =
        [ScopeBoundary::new(&Transient, ScopeParent::Root)];

    let root = ScopeTopology::new(&ROOT_BOUNDARIES)
        .prepare()
        .expect_err("root cannot be declared");
    let transient = ScopeTopology::new(&TRANSIENT_BOUNDARIES)
        .prepare()
        .expect_err("transient cannot be declared");

    assert_eq!(root, ScopeTopologyError::ReservedId { id: Singleton.id() });
    assert_eq!(
        transient,
        ScopeTopologyError::ReservedId { id: Transient.id() }
    );
}

#[test]
fn missing_and_self_parents_are_rejected() {
    static MISSING_BOUNDARIES: [ScopeBoundary; 1] =
        [ScopeBoundary::new(&HTTP, ScopeParent::Boundary(MISSING_ID))];
    static SELF_BOUNDARIES: [ScopeBoundary; 1] =
        [ScopeBoundary::new(&HTTP, ScopeParent::Boundary(HTTP_ID))];

    let missing = ScopeTopology::new(&MISSING_BOUNDARIES)
        .prepare()
        .expect_err("missing parent is invalid");
    let self_parent = ScopeTopology::new(&SELF_BOUNDARIES)
        .prepare()
        .expect_err("self-parent is invalid");

    assert_eq!(
        missing,
        ScopeTopologyError::MissingParent {
            id: HTTP_ID,
            parent: MISSING_ID,
        }
    );
    assert_eq!(self_parent, ScopeTopologyError::SelfParent { id: HTTP_ID });
}

#[test]
fn cycles_are_reported_before_impossible_rank_ordering() {
    static BOUNDARIES: [ScopeBoundary; 2] = [
        ScopeBoundary::new(&CYCLE_B, ScopeParent::Boundary(CYCLE_A_ID)),
        ScopeBoundary::new(&CYCLE_A, ScopeParent::Boundary(CYCLE_B_ID)),
    ];

    let error = ScopeTopology::new(&BOUNDARIES)
        .prepare()
        .expect_err("parent cycle is invalid");

    assert_eq!(
        error,
        ScopeTopologyError::Cycle {
            members: Box::new([CYCLE_A_ID, CYCLE_B_ID]),
        }
    );
}

#[test]
fn parent_must_have_strictly_greater_lifetime_rank() {
    static EQUAL_RANK_BOUNDARIES: [ScopeBoundary; 2] = [
        ScopeBoundary::new(&MESSAGE, ScopeParent::Boundary(SIBLING_ID)),
        ScopeBoundary::new(&SIBLING, ScopeParent::Root),
    ];
    static ROOT_RANK_BOUNDARIES: [ScopeBoundary; 1] =
        [ScopeBoundary::new(&LONG, ScopeParent::Root)];

    let equal = ScopeTopology::new(&EQUAL_RANK_BOUNDARIES)
        .prepare()
        .expect_err("equal parent rank is invalid");
    let root = ScopeTopology::new(&ROOT_RANK_BOUNDARIES)
        .prepare()
        .expect_err("root rank must be greater than child rank");

    assert_eq!(
        equal,
        ScopeTopologyError::InvalidParentRank {
            child: MESSAGE_ID,
            child_rank: 50,
            parent: SIBLING_ID,
            parent_rank: 50,
        }
    );
    assert_eq!(
        root,
        ScopeTopologyError::InvalidParentRank {
            child: LONG_ID,
            child_rank: u8::MAX,
            parent: Singleton.id(),
            parent_rank: u8::MAX,
        }
    );
}
