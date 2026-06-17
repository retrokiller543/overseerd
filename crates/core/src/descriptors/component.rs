use std::{
    any::{Any, TypeId},
    collections::HashMap,
    fmt,
    future::Future,
    pin::Pin,
    sync::Arc,
};

use super::types::TypeDescriptor;

/// Metadata trait for types registerable as components.
///
/// Supplies the runtime identity used to synthesize a descriptor for a
/// manually-provided instance (`DaemonBuilder::with_component`). Implemented by
/// `#[derive(Component)]`, `#[component]`, and `#[service]`.
pub trait Component: Any + Send + Sync + 'static {
    const ID: &'static str;
    const NAME: &'static str;
}

/// A [`Component`] that is also a service, carrying its version in the type.
///
/// Implemented by `#[service]`. Tracking the version on the type (rather than
/// only in the `ServiceDescriptor`) keeps it available generically and opens
/// the door to manual service registration.
pub trait ServiceComponent: Component {
    const VERSION: Option<&'static str>;
}

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
///
/// `value` stores `Arc<T>` inside a `Box<dyn Any + Send + Sync>`, which
/// allows recovery via `value.downcast_ref::<Arc<T>>()`.
pub struct BoxedComponent {
    pub ty: TypeDescriptor,
    pub value: Box<dyn Any + Send + Sync>,
}

impl fmt::Debug for BoxedComponent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BoxedComponent")
            .field("ty", &self.ty)
            .finish_non_exhaustive()
    }
}

/// Context passed to a component factory during construction.
///
/// Holds all components built so far, in dependency order. Factories call
/// `resolve::<T>()` to obtain their dependencies before constructing themselves.
pub struct ComponentConstructionContext {
    components: HashMap<TypeId, BoxedComponent>,
}

impl ComponentConstructionContext {
    pub fn new() -> Self {
        Self {
            components: HashMap::new(),
        }
    }

    /// Returns a cloned `Arc<T>` for a previously constructed component of type `T`.
    pub fn resolve<T: Any + Send + Sync + 'static>(&self) -> Option<Arc<T>> {
        let type_id = TypeId::of::<T>();
        let component = self.components.get(&type_id)?;

        component.value.downcast_ref::<Arc<T>>().cloned()
    }

    pub(crate) fn insert(&mut self, component: BoxedComponent) {
        let type_id = (component.ty.type_id)();
        self.components.insert(type_id, component);
    }

    pub(crate) fn into_components(self) -> HashMap<TypeId, BoxedComponent> {
        self.components
    }

    /// Whether a component of `type_id` has already been constructed or seeded.
    pub(crate) fn contains(&self, type_id: TypeId) -> bool {
        self.components.contains_key(&type_id)
    }
}

impl Default for ComponentConstructionContext {
    fn default() -> Self {
        Self::new()
    }
}

/// Async factory function pointer for constructing a component.
pub type ComponentFactory = for<'a> fn(
    &'a mut ComponentConstructionContext,
) -> Pin<Box<dyn Future<Output = crate::Result<BoxedComponent>> + Send + 'a>>;

/// Static metadata describing a component and how to construct it.
///
/// `Copy` so the registry can own a flat `Vec<ComponentDescriptor>` holding both
/// inventory-collected descriptors and ones synthesized at runtime for
/// manually-provided instances.
#[derive(Clone, Copy)]
pub struct ComponentDescriptor {
    pub id: &'static str,
    pub name: &'static str,
    pub ty: TypeDescriptor,
    pub scope: ComponentScope,
    pub dependencies: &'static [DependencyDescriptor],
    /// `None` for a manually-provided instance: there is nothing to construct,
    /// the value is seeded into the container directly.
    pub factory: Option<ComponentFactory>,
    /// Whether this is a default (field-injection) factory that an explicit
    /// `#[init]` constructor or a manual registration may override. Exactly one
    /// non-default factory is allowed per type.
    pub default_factory: bool,
}

impl fmt::Debug for ComponentDescriptor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ComponentDescriptor")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("ty", &self.ty)
            .field("scope", &self.scope)
            .field("dependencies", &self.dependencies)
            .field("default_factory", &self.default_factory)
            .finish_non_exhaustive()
    }
}
