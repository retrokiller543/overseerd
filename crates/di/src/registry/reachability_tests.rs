use std::{future::Future, pin::Pin};

use overseerd_core::{
    Cardinality, DependencyDescriptor, ResolutionMode, ScopeId, StaticScope, TypeDescriptor,
};

use super::*;
use crate::descriptors::{
    BoxedComponent, ComponentConstructionContext, ComponentFactoryDescriptor,
};

const HTTP_ID: ScopeId = overseerd_core::namespaced_id!(ScopeId, "test/http");
const CONNECTION_ID: ScopeId = overseerd_core::namespaced_id!(ScopeId, "test/connection");
const MESSAGE_ID: ScopeId = overseerd_core::namespaced_id!(ScopeId, "test/message");
const SIBLING_ID: ScopeId = overseerd_core::namespaced_id!(ScopeId, "test/sibling");

/// A standalone request branch with the same display label as [`MessageScope`].
struct HttpScope;

impl StaticScope for HttpScope {
    const ID: ScopeId = HTTP_ID;
    const RANK: u8 = 1;
    const NAME: &'static str = "Request";
}

/// A connection boundary reachable by websocket messages.
struct ConnectionScope;

impl StaticScope for ConnectionScope {
    const ID: ScopeId = CONNECTION_ID;
    const RANK: u8 = 2;
    const NAME: &'static str = "Connection";
}

/// A websocket message boundary sharing a display label with [`HttpScope`].
struct MessageScope;

impl StaticScope for MessageScope {
    const ID: ScopeId = MESSAGE_ID;
    const RANK: u8 = 1;
    const NAME: &'static str = "Request";
}

/// A provider scope with a rank reachable under legacy validation only.
struct SiblingScope;

impl StaticScope for SiblingScope {
    const ID: ScopeId = SIBLING_ID;
    const RANK: u8 = 2;
    const NAME: &'static str = "Sibling";
}

fn fake_factory<'a>(
    _: &'a mut ComponentConstructionContext,
) -> Pin<Box<dyn Future<Output = crate::Result<BoxedComponent>> + Send + 'a>> {
    Box::pin(async { unreachable!("validation does not construct components") })
}

fn no_dependencies() -> Vec<DependencyDescriptor> {
    Vec::new()
}

static NO_DEPENDENCY_FACTORY: [ComponentFactoryDescriptor; 1] = [ComponentFactoryDescriptor {
    construct: fake_factory,
    dependencies: no_dependencies,
    default: false,
}];

fn no_dependency_factory() -> &'static [ComponentFactoryDescriptor] {
    &NO_DEPENDENCY_FACTORY
}

fn dependency_factory() -> &'static [ComponentFactoryDescriptor] {
    static FACTORY: [ComponentFactoryDescriptor; 1] = [ComponentFactoryDescriptor {
        construct: fake_factory,
        dependencies: dependency,
        default: false,
    }];

    &FACTORY
}

fn dependency() -> Vec<DependencyDescriptor> {
    vec![DependencyDescriptor {
        name: "Dependency",
        ty: TypeDescriptor::of::<u16>("Dependency"),
        cardinality: Cardinality::One,
        optional: false,
        dynamic: false,
        qualifier: None,
        config: false,
        resolution: ResolutionMode::Eager,
    }]
}

fn component(
    id: &'static str,
    name: &'static str,
    ty: TypeDescriptor,
    scope: &'static dyn Scope,
    factories: fn() -> &'static [ComponentFactoryDescriptor],
) -> ComponentDescriptor {
    ComponentDescriptor {
        id,
        name,
        ty,
        scope,
        factories,
        hooks: overseerd_hooks::no_hooks,
    }
}

fn reaches(consumer: ScopeId, dependency: ScopeId) -> bool {
    consumer == dependency
        || dependency == Singleton.id()
        || (consumer == MESSAGE_ID && dependency == CONNECTION_ID)
}

#[test]
fn topology_rejects_equal_label_sibling_dependency() {
    let consumer = component(
        "http-consumer",
        "HttpConsumer",
        TypeDescriptor::of::<u8>("HttpConsumer"),
        &HttpScope,
        dependency_factory,
    );
    let dependency = component(
        "message-dependency",
        "Dependency",
        TypeDescriptor::of::<u16>("Dependency"),
        &MessageScope,
        no_dependency_factory,
    );
    let registry = ComponentRegistry {
        components: vec![consumer, dependency],
        providers: Vec::new(),
    };

    assert!(registry.validate().is_ok());
    assert!(matches!(
        registry.validate_with_scope_reachability(reaches),
        Err(Error::ScopeViolation { .. })
    ));
}

#[test]
fn topology_accepts_declared_ancestor_dependency() {
    let consumer = component(
        "message-consumer",
        "MessageConsumer",
        TypeDescriptor::of::<u8>("MessageConsumer"),
        &MessageScope,
        dependency_factory,
    );
    let dependency = component(
        "connection-dependency",
        "Dependency",
        TypeDescriptor::of::<u16>("Dependency"),
        &ConnectionScope,
        no_dependency_factory,
    );
    let registry = ComponentRegistry {
        components: vec![consumer, dependency],
        providers: Vec::new(),
    };

    assert!(registry.validate_with_scope_reachability(reaches).is_ok());
}

#[test]
fn topology_rejects_higher_rank_unreachable_sibling() {
    let consumer = component(
        "http-consumer",
        "HttpConsumer",
        TypeDescriptor::of::<u8>("HttpConsumer"),
        &HttpScope,
        dependency_factory,
    );
    let dependency = component(
        "sibling-dependency",
        "Dependency",
        TypeDescriptor::of::<u16>("Dependency"),
        &SiblingScope,
        no_dependency_factory,
    );
    let registry = ComponentRegistry {
        components: vec![consumer, dependency],
        providers: Vec::new(),
    };

    assert!(registry.validate().is_ok());
    assert!(matches!(
        registry.validate_with_scope_reachability(reaches),
        Err(Error::ScopeViolation { .. })
    ));
}
