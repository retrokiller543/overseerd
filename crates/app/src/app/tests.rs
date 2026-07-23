use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use overseerd_config::{ConfigManager, Dynamic};
use overseerd_core::TypeDescriptor;
use overseerd_di::{
    BoxedComponent, Component, ComponentConstructionContext, ComponentDescriptor,
    ComponentFactoryDescriptor, Injectable, Singleton,
};

use super::App;
use crate::{AppRegistry, AppRuntime, Plugin, PreBuildContext, Protocol, ProtocolPlugin};

static FACTORY_CALLS: AtomicUsize = AtomicUsize::new(0);
static PRE_BUILD_CALLS: AtomicUsize = AtomicUsize::new(0);
static PROTOCOL_BUILD_CALLS: AtomicUsize = AtomicUsize::new(0);

/// Component whose factory records the construction boundary.
struct BoundaryComponent;

impl Component for BoundaryComponent {
    const ID: &'static str = "boundary_component";
    const NAME: &'static str = "BoundaryComponent";

    type Handle = Arc<Self>;

    fn into_handle(self) -> Self::Handle {
        Arc::new(self)
    }
}

fn construct_boundary_component(
    _context: &mut ComponentConstructionContext,
) -> Pin<Box<dyn Future<Output = overseerd_di::Result<BoxedComponent>> + Send + '_>> {
    Box::pin(async {
        FACTORY_CALLS.fetch_add(1, Ordering::SeqCst);

        Ok(BoxedComponent {
            ty: TypeDescriptor::of::<BoundaryComponent>(BoundaryComponent::NAME),
            value: Box::new(Injectable::into_stored(Arc::new(BoundaryComponent))),
        })
    })
}

fn no_dependencies() -> Vec<overseerd_core::DependencyDescriptor> {
    Vec::new()
}

static BOUNDARY_FACTORIES: [ComponentFactoryDescriptor; 1] = [ComponentFactoryDescriptor {
    construct: construct_boundary_component,
    dependencies: no_dependencies,
    default: true,
}];

fn boundary_factories() -> &'static [ComponentFactoryDescriptor] {
    &BOUNDARY_FACTORIES
}

static BOUNDARY_COMPONENT: ComponentDescriptor = ComponentDescriptor {
    id: BoundaryComponent::ID,
    name: BoundaryComponent::NAME,
    ty: TypeDescriptor::of::<BoundaryComponent>(BoundaryComponent::NAME),
    scope: &Singleton,
    factories: boundary_factories,
    hooks: overseerd_hooks::no_hooks,
};

/// Protocol plugin recording validation and construction calls.
#[derive(Default)]
struct BoundaryPlugin;

impl Plugin for BoundaryPlugin {
    fn register(&self, _registry: &mut AppRegistry) {}
}

/// Protocol produced after the component graph is constructed.
struct BoundaryProtocol;

impl Protocol for BoundaryProtocol {
    type Error = crate::Error;
}

impl ProtocolPlugin for BoundaryPlugin {
    type Protocol = BoundaryProtocol;
    type Error = crate::Error;

    const SCOPES: &'static [&'static dyn overseerd_core::Scope] = &[];

    fn pre_build(&mut self, context: &PreBuildContext<'_>) -> Result<(), Self::Error> {
        PRE_BUILD_CALLS.fetch_add(1, Ordering::SeqCst);

        assert_eq!(context.name(), "prepare-boundary-test");
        assert!(
            context
                .resolved_components()
                .iter()
                .any(|component| component.id == BoundaryComponent::ID)
        );

        Ok(())
    }

    fn build(self, _runtime: &AppRuntime) -> Result<Self::Protocol, Self::Error> {
        PROTOCOL_BUILD_CALLS.fetch_add(1, Ordering::SeqCst);

        Ok(BoundaryProtocol)
    }
}

#[tokio::test]
async fn prepare_validates_without_constructing_components_or_protocol() {
    FACTORY_CALLS.store(0, Ordering::SeqCst);
    PRE_BUILD_CALLS.store(0, Ordering::SeqCst);
    PROTOCOL_BUILD_CALLS.store(0, Ordering::SeqCst);

    let prepared = App::<BoundaryPlugin>::builder("prepare-boundary-test")
        .config_source(ConfigManager::<Dynamic>::empty())
        .component_descriptor(&BOUNDARY_COMPONENT)
        .prepare()
        .expect("application prepares");

    assert_eq!(PRE_BUILD_CALLS.load(Ordering::SeqCst), 1);
    assert_eq!(FACTORY_CALLS.load(Ordering::SeqCst), 0);
    assert_eq!(PROTOCOL_BUILD_CALLS.load(Ordering::SeqCst), 0);

    let app = prepared.build().await.expect("prepared application builds");

    assert_eq!(FACTORY_CALLS.load(Ordering::SeqCst), 1);
    assert_eq!(PROTOCOL_BUILD_CALLS.load(Ordering::SeqCst), 1);
    assert!(app.container().get::<BoundaryComponent>().is_some());
}
