use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use overseerd_app::App;
use overseerd_config::{ConfigManager, Dynamic};
use overseerd_core::TypeDescriptor;
use overseerd_di::{
    BoxedComponent, Component, ComponentConstructionContext, ComponentDescriptor,
    ComponentFactoryDescriptor, Injectable, Singleton,
};

use super::{RpcAppBuilder, RpcPlugin};
use crate::{Error, ServiceDescriptor};

static FACTORY_CALLS: AtomicUsize = AtomicUsize::new(0);

/// Component proving protocol validation runs before singleton construction.
struct SentinelComponent;

impl Component for SentinelComponent {
    const ID: &'static str = "rpc_validation_sentinel";
    const NAME: &'static str = "RpcValidationSentinel";

    type Handle = Arc<Self>;

    fn into_handle(self) -> Self::Handle {
        Arc::new(self)
    }
}

fn construct_sentinel(
    _context: &mut ComponentConstructionContext,
) -> Pin<Box<dyn Future<Output = overseerd_di::Result<BoxedComponent>> + Send + '_>> {
    Box::pin(async {
        FACTORY_CALLS.fetch_add(1, Ordering::SeqCst);

        Ok(BoxedComponent {
            ty: TypeDescriptor::of::<SentinelComponent>(SentinelComponent::NAME),
            value: Box::new(Injectable::into_stored(Arc::new(SentinelComponent))),
        })
    })
}

fn no_dependencies() -> Vec<overseerd_core::DependencyDescriptor> {
    Vec::new()
}

static SENTINEL_FACTORIES: [ComponentFactoryDescriptor; 1] = [ComponentFactoryDescriptor {
    construct: construct_sentinel,
    dependencies: no_dependencies,
    default: true,
}];

fn sentinel_factories() -> &'static [ComponentFactoryDescriptor] {
    &SENTINEL_FACTORIES
}

static SENTINEL_COMPONENT: ComponentDescriptor = ComponentDescriptor {
    id: SentinelComponent::ID,
    name: SentinelComponent::NAME,
    ty: TypeDescriptor::of::<SentinelComponent>(SentinelComponent::NAME),
    scope: &Singleton,
    factories: sentinel_factories,
    hooks: overseerd_hooks::no_hooks,
};

struct EmptyService;

fn no_rpc_groups() -> &'static [crate::RpcGroup] {
    &[]
}

static EMPTY_SERVICE: ServiceDescriptor = ServiceDescriptor {
    id: "empty",
    name: "EmptyService",
    ty: TypeDescriptor::of::<EmptyService>("EmptyService"),
    version: None,
    rpcs: no_rpc_groups,
};

#[test]
fn empty_service_fails_during_prepare_before_component_construction() {
    FACTORY_CALLS.store(0, Ordering::SeqCst);

    let result = App::<RpcPlugin>::builder("invalid-rpc-test")
        .config_source(ConfigManager::<Dynamic>::empty())
        .component_descriptor(&SENTINEL_COMPONENT)
        .service_descriptor(&EMPTY_SERVICE)
        .prepare();

    let error = match result {
        Ok(_) => panic!("empty service was not rejected during preparation"),
        Err(error) => error,
    };

    assert!(matches!(error, Error::EmptyService(service) if service == "EmptyService"));
    assert_eq!(FACTORY_CALLS.load(Ordering::SeqCst), 0);
}
