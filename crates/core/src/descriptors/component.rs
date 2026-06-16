use std::{any::Any, fmt, future::Future, pin::Pin, sync::Arc};

use super::types::TypeDescriptor;

/// Lifetime policy for a component instance.
#[derive(Clone, Copy, Debug)]
pub enum ComponentScope {
    Singleton,
    Transient,
}

/// Declares that a component requires another component by type.
#[derive(Clone, Copy, Debug)]
pub struct DependencyDescriptor {
    pub name: &'static str,
    pub ty: TypeDescriptor,
    pub optional: bool,
}

/// A type-erased instantiated component.
pub struct BoxedComponent {
    pub ty: TypeDescriptor,
    pub value: Arc<dyn Any + Send + Sync>,
}

/// Context available to a component factory during construction.
///
/// Minimal placeholder; dependency resolution will be added in a later stage.
pub struct ComponentConstructionContext {}

/// Async factory function pointer for constructing a component.
pub type ComponentFactory = for<'a> fn(
    &'a mut ComponentConstructionContext,
) -> Pin<Box<dyn Future<Output = crate::Result<BoxedComponent>> + Send + 'a>>;

/// Static metadata describing a component and how to construct it.
pub struct ComponentDescriptor {
    pub id: &'static str,
    pub name: &'static str,
    pub ty: TypeDescriptor,
    pub scope: ComponentScope,
    pub dependencies: &'static [DependencyDescriptor],
    pub factory: ComponentFactory,
}

impl fmt::Debug for ComponentDescriptor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ComponentDescriptor")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("ty", &self.ty)
            .field("scope", &self.scope)
            .field("dependencies", &self.dependencies)
            .finish_non_exhaustive()
    }
}