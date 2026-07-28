use std::any::TypeId;
use std::future::Future;
use std::pin::Pin;

use overseerd_core::{
    Cardinality, DependencyDescriptor, ResolutionMode, Scope, ScopeId, StaticScope, TypeDescriptor,
};
use overseerd_di::{
    BoxedComponent, ComponentConstructionContext, ComponentFactoryDescriptor, Singleton,
};

use super::*;
use crate::{Error, ScopeBoundary, ScopeParent, ScopeTopology};

const PARENT_ID: ScopeId = overseerd_core::namespaced_id!(ScopeId, "test/parent");
const CHILD_ID: ScopeId = overseerd_core::namespaced_id!(ScopeId, "test/child");
const SIBLING_ID: ScopeId = overseerd_core::namespaced_id!(ScopeId, "test/sibling");
const MISSING_ID: ScopeId = overseerd_core::namespaced_id!(ScopeId, "test/missing");

struct Parent;
struct Child;
struct Sibling;
struct Missing;

macro_rules! test_scope {
    ($type:ty, $id:expr, $name:literal, $rank:expr) => {
        impl StaticScope for $type {
            const ID: ScopeId = $id;
            const RANK: u8 = $rank;
            const NAME: &'static str = $name;
        }
    };
}

test_scope!(Parent, PARENT_ID, "Parent", 100);
test_scope!(Child, CHILD_ID, "Child", 50);
test_scope!(Sibling, SIBLING_ID, "Sibling", 50);
test_scope!(Missing, MISSING_ID, "Missing", 25);

static PARENT: Parent = Parent;
static CHILD: Child = Child;
static SIBLING: Sibling = Sibling;
static MISSING: Missing = Missing;
static BOUNDARIES: [ScopeBoundary; 3] = [
    ScopeBoundary::new(&PARENT, ScopeParent::Root),
    ScopeBoundary::new(&CHILD, ScopeParent::Boundary(PARENT_ID)),
    ScopeBoundary::new(&SIBLING, ScopeParent::Root),
];

/// A root-scoped test component.
struct Root;
/// A factory-less seed declared at the parent boundary.
struct ParentSeed;
/// A factory-less seed declared at a sibling boundary.
struct SiblingSeed;
/// A factory-backed child component.
struct ChildFactory;
/// A factory-less seed assigned to an undeclared boundary.
struct MissingSeed;
/// First independent singleton in declaration order.
struct FirstSingleton;
/// Second independent singleton in declaration order.
struct SecondSingleton;
/// First independent component at a protocol-owned scope.
struct FirstScoped;
/// Second independent component at a protocol-owned scope.
struct SecondScoped;

fn construct(
    _: &mut ComponentConstructionContext,
) -> Pin<Box<dyn Future<Output = overseerd_di::Result<BoxedComponent>> + Send + '_>> {
    Box::pin(async { unreachable!("planning does not invoke factories") })
}

fn no_dependencies() -> Vec<DependencyDescriptor> {
    Vec::new()
}

fn parent_dependency() -> Vec<DependencyDescriptor> {
    vec![DependencyDescriptor {
        name: "parent",
        ty: TypeDescriptor::of::<ParentSeed>("ParentSeed"),
        cardinality: Cardinality::One,
        optional: false,
        dynamic: false,
        qualifier: None,
        config: false,
        resolution: ResolutionMode::Eager,
    }]
}

fn sibling_dependency() -> Vec<DependencyDescriptor> {
    vec![DependencyDescriptor {
        name: "sibling",
        ty: TypeDescriptor::of::<SiblingSeed>("SiblingSeed"),
        cardinality: Cardinality::One,
        optional: false,
        dynamic: false,
        qualifier: None,
        config: false,
        resolution: ResolutionMode::Eager,
    }]
}

static EMPTY_FACTORY: [ComponentFactoryDescriptor; 1] = [ComponentFactoryDescriptor {
    construct,
    dependencies: no_dependencies,
    default: false,
}];
static PARENT_FACTORY: [ComponentFactoryDescriptor; 1] = [ComponentFactoryDescriptor {
    construct,
    dependencies: parent_dependency,
    default: false,
}];
static SIBLING_FACTORY: [ComponentFactoryDescriptor; 1] = [ComponentFactoryDescriptor {
    construct,
    dependencies: sibling_dependency,
    default: false,
}];

fn empty_factory() -> &'static [ComponentFactoryDescriptor] {
    &EMPTY_FACTORY
}

fn parent_factory() -> &'static [ComponentFactoryDescriptor] {
    &PARENT_FACTORY
}

fn sibling_factory() -> &'static [ComponentFactoryDescriptor] {
    &SIBLING_FACTORY
}

fn no_factory() -> &'static [ComponentFactoryDescriptor] {
    &[]
}

fn descriptor<T: 'static>(
    id: &'static str,
    name: &'static str,
    scope: &'static dyn Scope,
    factories: fn() -> &'static [ComponentFactoryDescriptor],
) -> ComponentDescriptor {
    ComponentDescriptor {
        id,
        name,
        ty: TypeDescriptor::of::<T>(name),
        scope,
        factories,
        hooks: overseerd_hooks::no_hooks,
    }
}

fn topology() -> PreparedScopeTopology {
    ScopeTopology::new(&BOUNDARIES)
        .prepare()
        .expect("test topology prepares")
}

#[test]
fn factoryless_descriptor_in_undeclared_scope_is_rejected() {
    let descriptors = [descriptor::<MissingSeed>(
        "missing_seed",
        "MissingSeed",
        &MISSING,
        no_factory,
    )];

    let error = ScopePlan::partition(&descriptors, &[], &topology())
        .expect_err("factory-less descriptors require declared scopes");

    assert!(matches!(
        error,
        Error::UndeclaredScope {
            component,
            scope: MISSING_ID,
        } if component.ends_with("MissingSeed")
    ));
}

#[test]
fn partition_preserves_independent_singleton_declaration_order() {
    let descriptors = [
        descriptor::<SecondSingleton>("z-second", "Second", &Singleton, empty_factory),
        descriptor::<FirstSingleton>("a-first", "First", &Singleton, empty_factory),
    ];
    let plan = ScopePlan::partition(&descriptors, &[], &topology()).expect("plan succeeds");

    assert_eq!(plan.singletons[0].id, descriptors[0].id);
    assert_eq!(plan.singletons[1].id, descriptors[1].id);
}

#[test]
fn partition_preserves_independent_scoped_declaration_order() {
    let descriptors = [
        descriptor::<SecondScoped>("z-second", "Second", &CHILD, empty_factory),
        descriptor::<FirstScoped>("a-first", "First", &CHILD, empty_factory),
    ];
    let plan = ScopePlan::partition(&descriptors, &[], &topology()).expect("plan succeeds");

    assert_eq!(plan.orders[&CHILD_ID][0].id, descriptors[0].id);
    assert_eq!(plan.orders[&CHILD_ID][1].id, descriptors[1].id);
}

#[test]
fn child_order_accepts_root_ancestor_and_local_factoryless_descriptors() {
    let descriptors = [
        descriptor::<Root>("root", "Root", &Singleton, empty_factory),
        descriptor::<ParentSeed>("parent_seed", "ParentSeed", &PARENT, no_factory),
        descriptor::<ChildFactory>("child", "ChildFactory", &CHILD, parent_factory),
    ];

    let plan = ScopePlan::partition(&descriptors, &[], &topology()).expect("plan succeeds");

    assert_eq!(plan.orders[&CHILD_ID].len(), 1);
    assert_eq!(
        plan.orders[&CHILD_ID][0].ty.type_id,
        TypeId::of::<ChildFactory>()
    );
    assert_eq!(
        plan.seed_destinations[&TypeId::of::<ParentSeed>()].scope,
        PARENT_ID
    );
}

#[test]
fn child_order_does_not_treat_sibling_factoryless_descriptors_as_prebuilt() {
    let descriptors = [
        descriptor::<SiblingSeed>("sibling_seed", "SiblingSeed", &SIBLING, no_factory),
        descriptor::<ChildFactory>("child", "ChildFactory", &CHILD, sibling_factory),
    ];

    let error = ScopePlan::partition(&descriptors, &[], &topology())
        .expect_err("sibling seed is not reachable while planning child");

    assert!(matches!(
        error,
        Error::Di(overseerd_di::Error::DependencyCycle(_))
    ));
}

#[test]
fn topology_aware_registry_validation_rejects_sibling_dependencies() {
    let registry = crate::AppRegistry {
        components: vec![
            descriptor::<SiblingSeed>("sibling_seed", "SiblingSeed", &SIBLING, no_factory),
            descriptor::<ChildFactory>("child", "ChildFactory", &CHILD, sibling_factory),
        ],
        providers: Vec::new(),
        config_bindings: Vec::new(),
    };

    let error = registry
        .validate_with_scope_topology(&topology())
        .expect_err("sibling dependency must be unreachable");

    assert!(matches!(
        error,
        Error::Di(overseerd_di::Error::ScopeViolation { .. })
    ));
}
