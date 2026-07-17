use std::any::{Any, TypeId};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use overseerd_core::{DependencyDescriptor, ResolverCtx, ResolverSet, TypeDescriptor};

use super::{Error, HookCall, HookDescriptor, HookKind, HookManager, Startup};

struct PanickingComponent;

fn no_dependencies() -> Vec<DependencyDescriptor> {
    Vec::new()
}

fn startup_type_id() -> TypeId {
    TypeId::of::<Startup>()
}

#[allow(clippy::type_complexity)]
fn panicking_call<'a>(
    _ctx: &'a (dyn ResolverCtx + Send + Sync),
    _cx: &'a (dyn Any + Send + Sync),
) -> Pin<Box<dyn Future<Output = super::Result<Box<dyn Any + Send>>> + Send + 'a>> {
    Box::pin(async move { panic!("sensitive panic payload") })
}

fn panicking_hook() -> HookDescriptor {
    HookDescriptor {
        component_ty: TypeDescriptor::of::<PanickingComponent>("PanickingComponent"),
        kind: Startup::NAME,
        kind_ty: startup_type_id,
        dependencies: no_dependencies,
        call: panicking_call as HookCall,
    }
}

#[test]
fn hook_panics_are_isolated_and_the_manager_remains_usable() {
    let manager = HookManager::new(vec![panicking_hook()]);
    manager.attach(Arc::new(ResolverSet::new()));

    for _ in 0..2 {
        let outcomes = futures::executor::block_on(manager.run::<Startup>(&(), |_| true));

        assert!(matches!(
            outcomes.as_slice(),
            [(
                _,
                Err(Error::Panicked {
                    hook: "startup",
                    component: "PanickingComponent"
                })
            )]
        ));
        assert!(
            !outcomes[0]
                .1
                .as_ref()
                .unwrap_err()
                .to_string()
                .contains("sensitive")
        );
    }
}
