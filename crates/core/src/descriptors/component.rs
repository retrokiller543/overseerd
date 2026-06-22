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

/// How a resolved dependency is stored in, and handed out of, the container.
///
/// Every injectable *handle* implements this. `Target` is the type the instance
/// is keyed under; cloning a handle happens on every resolution, so it must be
/// cheap. The blanket impl covers the common case — every `Arc<T>` is injectable
/// and keyed by `T`, so existing `Arc<T>` dependencies need no change. A type
/// that is itself cheaply shareable (internally `Arc`, e.g. a pool) may instead
/// implement `Injectable` for itself, so it is injected by value with no outer
/// `Arc`.
pub trait Injectable: Clone + Send + Sync + 'static {
    /// The type this handle is stored and looked up under.
    type Target: 'static;
}

impl<T: Send + Sync + 'static> Injectable for Arc<T> {
    type Target = T;
}

/// Field wrapper marking a dependency as **runtime-provided**.
///
/// A `Dynamic<H>` dependency is satisfied by a provider registered at runtime
/// (e.g. via `DaemonBuilder::with_component`) rather than one discovered at
/// build, so the edge is exempt from static dependency validation. It wraps the
/// resolved handle `H` and derefs to it for transparent access.
#[derive(Clone)]
pub struct Dynamic<H: Injectable>(pub H);

impl<H: Injectable> std::ops::Deref for Dynamic<H> {
    type Target = H;

    fn deref(&self) -> &H {
        &self.0
    }
}

impl<H: Injectable> AsRef<H> for Dynamic<H> {
    fn as_ref(&self) -> &H {
        &self.0
    }
}

/// Lifetime policy for a component instance.
#[derive(Clone, Copy, Debug)]
pub enum ComponentScope {
    Singleton,
    Transient,
}

/// Cardinality of a dependency edge: how many providers satisfy it.
///
/// `Collection` and `Keyed` are *multi-valued* and always satisfiable — zero
/// providers yields an empty `Vec`/`HashMap`, never a missing-dependency error.
/// Only `One` and `Primary` (when not `optional`) require a provider to exist.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cardinality {
    /// Exactly one provider, resolved by concrete type (`Arc<T>` / by-value handle).
    One,
    /// Every provider of a trait, as `Vec<Arc<dyn Trait>>`. Empty is valid.
    Collection,
    /// Every provider of a trait keyed by qualifier, as `HashMap<String, Arc<dyn Trait>>`. Empty is valid.
    Keyed,
    /// The primary (or sole) provider of a trait, as `Arc<dyn Trait>`.
    Primary,
}

impl Cardinality {
    /// Whether an edge of this cardinality requires at least one provider to
    /// exist. Multi-valued edges (`Collection`/`Keyed`) accept zero.
    pub fn requires_provider(self) -> bool {
        matches!(self, Cardinality::One | Cardinality::Primary)
    }
}

/// Declares that a component requires another component.
///
/// The edge's shape is described by three orthogonal axes: `cardinality` (how
/// many providers satisfy it), `optional` (whether absence is tolerated), and
/// `dynamic` (whether the provider is registered at runtime rather than
/// discovered — which exempts the edge from static dependency validation, since
/// nothing visible at build time is expected to satisfy it).
#[derive(Clone, Copy, Debug)]
pub struct DependencyDescriptor {
    pub name: &'static str,
    pub ty: TypeDescriptor,
    pub cardinality: Cardinality,
    pub optional: bool,
    pub dynamic: bool,
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

    /// Resolves a previously constructed dependency by its injectable handle `H`,
    /// keyed under `H::Target`. For the common case `H = Arc<T>`, this returns a
    /// cloned `Arc<T>` keyed by `T`, exactly as before.
    pub fn resolve<H: Injectable>(&self) -> Option<H> {
        let type_id = TypeId::of::<H::Target>();
        let component = self.components.get(&type_id)?;

        component.value.downcast_ref::<H>().cloned()
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
pub type ComponentFactory =
    for<'a> fn(
        &'a mut ComponentConstructionContext,
    ) -> Pin<Box<dyn Future<Output = crate::Result<BoxedComponent>> + Send + 'a>>;

/// Static metadata describing a component and how to construct it.
///
/// `Copy` so the registry can own a flat `Vec<ComponentDescriptor>` holding both
/// link-time-collected descriptors and ones synthesized at runtime for
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

impl ComponentDescriptor {
    pub const fn of<T: Component>() -> Self {
        Self {
            id: T::ID,
            name: T::NAME,
            ty: TypeDescriptor::of::<T>(T::NAME),
            scope: ComponentScope::Singleton,
            dependencies: &[],
            factory: None,
            default_factory: false,
        }
    }
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
