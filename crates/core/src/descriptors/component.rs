use std::{
    any::{Any, TypeId},
    collections::HashMap,
    fmt,
    future::Future,
    pin::Pin,
    sync::Arc,
};
use std::ops::Deref;
use super::types::TypeDescriptor;

/// Metadata trait for types registerable as components.
///
/// Supplies the runtime identity used to synthesize a descriptor for a
/// manually-provided instance (`DaemonBuilder::with_component`). Implemented by
/// `#[derive(Component)]`, `#[component]`, and `#[service]`.
pub trait Component: Any + Send + Sync + 'static {
    const ID: &'static str;
    const NAME: &'static str;

    /// The cloneable handle this component is stored in the container as, and
    /// injected by. `Arc<Self>` by default (auto-wrapped); a type that manages
    /// its own sharing — typically because it is internally `Arc` and cheap to
    /// clone — may set this to `Self` via `#[component(by_value)]`, so it is
    /// stored without an extra `Arc`.
    type Handle: Injectable<Target = Self>;

    /// Wraps a freshly constructed instance into its storage handle.
    fn into_handle(self) -> Self::Handle;
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
    /// The type this handle is stored and looked up under. `?Sized` so a trait
    /// object (`dyn Trait + Send + Sync`) can key its providers.
    type Target: ?Sized + 'static;
}

impl<T: ?Sized + Send + Sync + 'static> Injectable for Arc<T> {
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

impl<H: Injectable> Deref for Dynamic<H> {
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

/// Compile-time dependency marker: `Wiring: Provide<T>` holds when some component
/// in scope provides `T`. Under the `di-check` feature, the macros emit
/// `impl Provide<Self> for Wiring` for every component and a bound per concrete
/// dependency, so a missing provider is a `cargo check` error — a type-checked
/// alternative to the source-level [`build.rs` analyzer](overseer_analyze) that
/// catches what source parsing can't (real types, `cfg`, generics).
#[diagnostic::on_unimplemented(
    message = "no component provides `{T}`",
    note = "add a component for `{T}`, or mark the dependency `Dynamic<{T}>` if it is provided at runtime"
)]
pub trait Provide<T: ?Sized> {}

/// The crate-wide anchor that each component's `Provide` impl attaches to. A
/// dependency assertion is the bound `Wiring: Provide<Dep>`.
pub struct Wiring;

/// Marker that all of a component's dependencies are provided. Under `di-check`
/// the macros emit `impl Wired for T where Wiring: Provide<Dep>, ..` carrying
/// *every* single dependency (concrete and trait-object) as a lazy bound. The
/// [`app!`](overseer_macros::daemon) macro asserts `T: Wired` for its listed types,
/// discharging the whole set at the binary — where every `Provide` impl (across
/// crates) is visible, so it catches the cross-crate and trait-object cases the
/// per-component asserts cannot.
pub trait Wired {}

/// Lifetime policy for a component instance.
#[derive(Clone, Copy, Debug)]
pub enum ComponentScope {
    Singleton,
    Transient,
}

/// Cardinality of a dependency edge: how many values satisfy it.
///
/// A `One` edge wants a single value, resolved by its handle's `Target` type —
/// whether that is a concrete type (`Arc<T>`) or a trait object (`Arc<dyn Trait>`).
/// For a trait object the container has already placed the chosen (primary or
/// sole) provider under the trait's `TypeId`, so the dependency resolves through
/// the same path and never sees the `#[primary]` selection itself.
///
/// `Collection` and `Keyed` are *multi-valued* and always satisfiable — zero
/// providers yields an empty `Vec`/`HashMap`, never a missing-dependency error.
/// Only `One` (when not `optional`) requires a value to exist.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cardinality {
    /// A single value, resolved by concrete type or by the chosen trait provider
    /// (`Arc<T>`, a by-value handle, or `Arc<dyn Trait>`).
    One,
    /// Every provider of a trait, as `Vec<Arc<dyn Trait>>`. Empty is valid.
    Collection,
    /// Every provider of a trait keyed by qualifier, as `HashMap<String, Arc<dyn Trait>>`. Empty is valid.
    Keyed,
}

impl Cardinality {
    /// Whether an edge of this cardinality requires at least one value to exist.
    /// Multi-valued edges (`Collection`/`Keyed`) accept zero.
    pub fn requires_provider(self) -> bool {
        matches!(self, Cardinality::One)
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
    /// For a single `Arc<dyn Trait>` edge, selects a specific provider by its
    /// qualifier (`#[qualifier = ".."]`) instead of the primary/sole one.
    pub qualifier: Option<&'static str>,
}

/// Declares that a component **provides** a trait, so it can be injected as a
/// single `Arc<dyn Trait>` (the primary or sole provider), or collected into a
/// `Vec<Arc<dyn Trait>>` / `HashMap<String, Arc<dyn Trait>>` of all providers.
///
/// Emitted by `#[component(provide = ..)]` / `#[service(provide = ..)]`, one per
/// `(component, trait)` pair. `#[primary]` on the component sets `primary` on
/// every trait it provides; the dependency side never names a primary — it asks
/// for `Arc<dyn Trait>` and the container resolves the right one.
#[derive(Clone, Copy)]
pub struct ProviderDescriptor {
    /// The trait-object type provided, keyed by `TypeId::of::<dyn Trait>()`.
    pub trait_ty: TypeDescriptor,
    /// The providing component's concrete type.
    pub concrete_ty: TypeDescriptor,
    /// Qualifier key for `HashMap<String, Arc<dyn Trait>>` injection. Always
    /// present — inferred from the component's id unless set with `qualifier`.
    pub qualifier: &'static str,
    /// Whether this is the primary provider — chosen for a single `Arc<dyn Trait>`
    /// dependency when several providers of the trait exist.
    pub primary: bool,
    /// Re-erases the built concrete instance (`Arc<Concrete>`) as `Arc<dyn Trait>`
    /// for storage under the trait's key. Generated by the macro, which alone
    /// knows that `Concrete: Trait`.
    pub erase: fn(&BoxedComponent) -> BoxedComponent,
}

impl fmt::Debug for ProviderDescriptor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProviderDescriptor")
            .field("trait_ty", &self.trait_ty)
            .field("concrete_ty", &self.concrete_ty)
            .field("qualifier", &self.qualifier)
            .field("primary", &self.primary)
            .finish_non_exhaustive()
    }
}

/// A type-erased instantiated component.
///
/// `value` stores `Arc<T>` inside a `Box<dyn Any + Send + Sync>`, which
/// allows recovery via `value.downcast_ref::<Arc<T>>()`.
pub struct BoxedComponent {
    pub ty: TypeDescriptor,
    pub value: Box<dyn Any + Send + Sync>,
}

impl AsRef<Box<dyn Any + Send + Sync>> for BoxedComponent {
    fn as_ref(&self) -> &Box<dyn Any + Send + Sync> {
        &self.value
    }
}

impl Deref for BoxedComponent {
    type Target = Box<dyn Any + Send + Sync>;

    fn deref(&self) -> &Box<dyn Any + Send + Sync> {
        &self.value
    }
}

impl fmt::Debug for BoxedComponent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BoxedComponent")
            .field("ty", &self.ty)
            .finish_non_exhaustive()
    }
}

/// One trait provider's constructed value, plus the metadata that selects it.
struct ProviderInstance {
    qualifier: &'static str,
    primary: bool,
    value: BoxedComponent,
}

/// Context passed to a component factory during construction.
///
/// Holds all components built so far, in dependency order, plus the per-trait
/// provider instances erased so far. Factories call `resolve`/`resolve_all`/
/// `resolve_keyed` to obtain their dependencies before constructing themselves.
pub struct ComponentConstructionContext {
    components: HashMap<TypeId, BoxedComponent>,
    providers: HashMap<TypeId, Vec<ProviderInstance>>,
}

impl ComponentConstructionContext {
    pub fn new() -> Self {
        Self {
            components: HashMap::new(),
            providers: HashMap::new(),
        }
    }

    /// Resolves a single dependency by its injectable handle `H`, keyed under
    /// `H::Target`. A concrete instance is returned directly; for a trait object
    /// (`Arc<dyn Trait>`) with no concrete of that key, the primary (or sole)
    /// provider is chosen. `None` if absent or ambiguous.
    pub fn resolve<H: Injectable>(&self) -> Option<H> {
        let type_id = TypeId::of::<H::Target>();

        if let Some(component) = self.components.get(&type_id) {
            return component.value.downcast_ref::<H>().cloned();
        }

        let chosen = pick_single(self.providers.get(&type_id)?)?;

        chosen.value.value.downcast_ref::<H>().cloned()
    }

    /// Resolves the single provider of the trait `H::Target` whose qualifier
    /// matches, ignoring the primary/sole rule. `None` if no such provider.
    pub fn resolve_qualified<H: Injectable>(&self, qualifier: &str) -> Option<H> {
        let type_id = TypeId::of::<H::Target>();
        let entry = self
            .providers
            .get(&type_id)?
            .iter()
            .find(|entry| entry.qualifier == qualifier)?;

        entry.value.value.downcast_ref::<H>().cloned()
    }

    /// Resolves every provider of the trait `H::Target` as `Vec<H>` (empty if none).
    pub fn resolve_all<H: Injectable>(&self) -> Vec<H> {
        let type_id = TypeId::of::<H::Target>();

        self.providers
            .get(&type_id)
            .into_iter()
            .flatten()
            .filter_map(|entry| entry.value.downcast_ref::<H>().cloned())
            .collect()
    }

    /// Resolves every provider of the trait `H::Target` as a `HashMap<String, H>`
    /// keyed by qualifier (every provider has one — inferred or explicit).
    pub fn resolve_keyed<H: Injectable>(&self) -> HashMap<String, H> {
        let type_id = TypeId::of::<H::Target>();

        self.providers
            .get(&type_id)
            .into_iter()
            .flatten()
            .filter_map(|entry| {
                let value = entry.value.downcast_ref::<H>().cloned()?;

                Some((entry.qualifier.to_string(), value))
            })
            .collect()
    }

    pub(crate) fn insert(&mut self, component: BoxedComponent) {
        let type_id = (component.ty.type_id)();
        self.components.insert(type_id, component);
    }

    /// Erases an already-constructed concrete component as `Arc<dyn Trait>` and
    /// records it under the trait's key, so trait-object dependencies resolve.
    pub(crate) fn register_provider(&mut self, provider: &ProviderDescriptor) {
        let concrete_id = (provider.concrete_ty.type_id)();

        let Some(concrete) = self.components.get(&concrete_id) else {
            return;
        };

        let erased = (provider.erase)(concrete);

        self.providers
            .entry((provider.trait_ty.type_id)())
            .or_default()
            .push(ProviderInstance {
                qualifier: provider.qualifier,
                primary: provider.primary,
                value: erased,
            });
    }

    pub(crate) fn into_components(self) -> HashMap<TypeId, BoxedComponent> {
        self.components
    }

    /// Whether a component of `type_id` has already been constructed or seeded.
    pub(crate) fn contains(&self, type_id: TypeId) -> bool {
        self.components.contains_key(&type_id)
    }
}

/// Picks the single provider for an `Arc<dyn Trait>` dependency: the sole entry,
/// or the unique `#[primary]` one. Returns `None` when zero or ambiguous.
fn pick_single(entries: &[ProviderInstance]) -> Option<&ProviderInstance> {
    if entries.len() == 1 {
        return entries.first();
    }

    let mut primaries = entries.iter().filter(|entry| entry.primary);
    let first = primaries.next()?;

    match primaries.next() {
        Some(_) => None,
        None => Some(first),
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
