use super::types::TypeDescriptor;
use std::ops::Deref;
use std::{
    any::{Any, TypeId},
    collections::HashMap,
    fmt,
    future::Future,
    pin::Pin,
    sync::Arc,
};

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

/// The remote peer is a framework-seeded, **by-value** connection-scoped
/// injectable: every daemon provides it in each connection scope (and below, via
/// the parent chain), so a connection/request/transient component depends on it
/// directly as `peer: PeerInfo` — no `Arc`. `PeerInfo` is cheap to clone.
impl Injectable for overseer_transport::PeerInfo {
    type Target = overseer_transport::PeerInfo;
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

/// `PeerInfo` is seeded by the framework into every connection scope, so the
/// compile-time checker treats it as always provided. The runtime
/// [`validate_scopes`](crate::registry::DescriptorRegistry) still rejects a
/// *singleton* depending on it (a connection-scoped value outliving the daemon
/// would be a scope violation).
#[cfg(feature = "di-check")]
impl Provide<overseer_transport::PeerInfo> for Wiring {}

/// Marker that all of a component's dependencies are provided. Under `di-check`
/// the macros emit `impl Wired for T where Wiring: Provide<Dep>, ..` carrying
/// *every* single dependency (concrete and trait-object) as a lazy bound. The
/// [`app!`](overseer_macros::daemon) macro asserts `T: Wired` for its listed types,
/// discharging the whole set at the binary — where every `Provide` impl (across
/// crates) is visible, so it catches the cross-crate and trait-object cases the
/// per-component asserts cannot.
pub trait Wired {}

/// Lifetime policy for a component instance.
///
/// Determines where the instance is stored and how long it lives: a `Singleton`
/// in the root container for the daemon's lifetime, a `Connection`/`Request`
/// instance in a per-connection/per-call scope, and a `Transient` built fresh on
/// every resolution. The captive-dependency rule (a longer-lived component may not
/// depend on a shorter-lived one) is enforced against [`rank`](Self::rank).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ComponentScope {
    Singleton,
    Connection,
    Request,
    Transient,
}

impl ComponentScope {
    /// Lifetime rank: longer-lived scopes rank higher. A non-transient component
    /// may depend only on equal-or-higher-ranked non-transient components.
    ///
    /// Defining the lifetime order as a numeric rank (rather than matching on each
    /// variant at every call site) is what lets a future user-defined scope slot in
    /// at its own rank without touching the validation or container logic.
    pub fn rank(self) -> u8 {
        match self {
            ComponentScope::Singleton => 3,
            ComponentScope::Connection => 2,
            ComponentScope::Request => 1,
            ComponentScope::Transient => 0,
        }
    }

    /// Whether this scope rebuilds its instance on every resolution rather than
    /// caching one per scope.
    pub fn is_transient(self) -> bool {
        matches!(self, ComponentScope::Transient)
    }
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
    /// qualifier (`#[qualifier = ".."]`) instead of the primary/sole one. For a
    /// `config` edge it carries the property path (`#[config("..")]`), or `None` for
    /// the sole-binding shorthand.
    pub qualifier: Option<&'static str>,
    /// Whether this edge resolves a `#[derive(ConfigProperties)]` binding (a `Cfg<T>`
    /// keyed by property path) rather than a component or trait provider. Config
    /// edges are validated against the registered config bindings, not the component
    /// graph, so they are exempt from the standard dependency/scope checks.
    pub config: bool,
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
pub(crate) struct ProviderInstance {
    pub(crate) qualifier: &'static str,
    pub(crate) primary: bool,
    pub(crate) value: BoxedComponent,
}

/// One bound configuration value, plus the property path that selects it. The
/// runtime-string path is why configs need their own store rather than reusing the
/// `&'static str`-keyed trait-provider map.
pub(crate) struct ConfigInstance {
    pub(crate) path: String,
    pub(crate) value: BoxedComponent,
}

/// The instances and trait-providers held by one scope.
///
/// Shared by both the under-construction [`ComponentConstructionContext`] and the
/// frozen [`ScopeContainer`](crate::container::ScopeContainer); each owns one
/// `ScopeStore` and layers parent scopes on top for resolution. All lookups here
/// are **scope-local** — walking the parent chain is the caller's job.
#[derive(Default)]
pub(crate) struct ScopeStore {
    pub(crate) components: HashMap<TypeId, BoxedComponent>,
    pub(crate) providers: HashMap<TypeId, Vec<ProviderInstance>>,
    /// Bound config values keyed by their concrete `TypeId`, each carrying the
    /// property path it was bound at. Mirrors `providers` but keyed by a runtime
    /// path string and holding the concrete `Cfg<T>` handle (no trait erasure).
    pub(crate) configs: HashMap<TypeId, Vec<ConfigInstance>>,
}

impl ScopeStore {
    /// Single concrete-or-primary-provider lookup, scope-local. `None` if absent or
    /// ambiguous.
    pub(crate) fn resolve_local<H: Injectable>(&self) -> Option<H> {
        let type_id = TypeId::of::<H::Target>();

        if let Some(component) = self.components.get(&type_id) {
            return component.value.downcast_ref::<H>().cloned();
        }

        let chosen = pick_single(self.providers.get(&type_id)?)?;

        chosen.value.value.downcast_ref::<H>().cloned()
    }

    /// Qualifier-selected single provider, scope-local.
    pub(crate) fn resolve_qualified_local<H: Injectable>(&self, qualifier: &str) -> Option<H> {
        let type_id = TypeId::of::<H::Target>();
        let entry = self
            .providers
            .get(&type_id)?
            .iter()
            .find(|entry| entry.qualifier == qualifier)?;

        entry.value.value.downcast_ref::<H>().cloned()
    }

    /// Every scope-local provider of the trait `H::Target`.
    pub(crate) fn collect_all_local<H: Injectable>(&self) -> Vec<H> {
        let type_id = TypeId::of::<H::Target>();

        self.providers
            .get(&type_id)
            .into_iter()
            .flatten()
            .filter_map(|entry| entry.value.downcast_ref::<H>().cloned())
            .collect()
    }

    /// Every scope-local provider of the trait `H::Target`, keyed by qualifier.
    pub(crate) fn collect_keyed_local<H: Injectable>(&self) -> HashMap<String, H> {
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

    /// Path-selected single config, scope-local.
    pub(crate) fn resolve_config_local<H: Injectable>(&self, path: &str) -> Option<H> {
        let entries = self.configs.get(&TypeId::of::<H::Target>())?;
        let found = entries.iter().find(|entry| entry.path == path)?;

        found.value.value.downcast_ref::<H>().cloned()
    }

    /// The sole config of this type, scope-local — the type-only shorthand. `None`
    /// when zero or more than one binding of the type exists.
    pub(crate) fn resolve_config_sole_local<H: Injectable>(&self) -> Option<H> {
        let entries = self.configs.get(&TypeId::of::<H::Target>())?;

        match entries.as_slice() {
            [only] => only.value.value.downcast_ref::<H>().cloned(),
            _ => None,
        }
    }

    pub(crate) fn insert(&mut self, component: BoxedComponent) {
        let type_id = (component.ty.type_id)();
        self.components.insert(type_id, component);
    }

    pub(crate) fn insert_config(&mut self, path: String, value: BoxedComponent) {
        let type_id = (value.ty.type_id)();

        self.configs
            .entry(type_id)
            .or_default()
            .push(ConfigInstance { path, value });
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

    pub(crate) fn contains(&self, type_id: TypeId) -> bool {
        self.components.contains_key(&type_id)
    }
}

/// Context passed to a component factory during construction.
///
/// Holds the components built so far in this scope plus a link to the parent scope
/// chain, so a factory can resolve dependencies from its own scope or any
/// longer-lived one. Factories call `resolve`/`resolve_all`/`resolve_keyed` to
/// obtain their dependencies before constructing themselves.
///
/// Resolution is `async` even though eagerly-built scopes resolve immediately
/// (the future is ready): a `Transient` dependency is constructed on demand here,
/// and keeping the interface async is what lets connection/request scopes move to
/// lazy construction later without changing generated factory code.
pub struct ComponentConstructionContext {
    scope: ComponentScope,
    store: ScopeStore,
    parent: Option<Arc<crate::container::ScopeContainer>>,
    registry: Arc<crate::container::ScopeRegistry>,
}

impl ComponentConstructionContext {
    pub(crate) fn new(
        scope: ComponentScope,
        parent: Option<Arc<crate::container::ScopeContainer>>,
        registry: Arc<crate::container::ScopeRegistry>,
    ) -> Self {
        Self {
            scope,
            store: ScopeStore::default(),
            parent,
            registry,
        }
    }

    /// Resolves a single dependency by its injectable handle `H`, keyed under
    /// `H::Target`: this scope first, then each longer-lived parent scope, then —
    /// if `H::Target` is a `Transient` component — a freshly constructed instance.
    pub async fn resolve<H: Injectable>(&self) -> Option<H> {
        if let Some(handle) = self.store.resolve_local::<H>() {
            return Some(handle);
        }

        if let Some(parent) = &self.parent
            && let Some(handle) = parent.resolve_built::<H>()
        {
            return Some(handle);
        }

        crate::container::construct_transient::<H>(&self.registry, self.parent.clone()).await
    }

    /// Qualifier-selected single provider, this scope then parents.
    pub async fn resolve_qualified<H: Injectable>(&self, qualifier: &str) -> Option<H> {
        if let Some(handle) = self.store.resolve_qualified_local::<H>(qualifier) {
            return Some(handle);
        }

        self.parent
            .as_ref()
            .and_then(|parent| parent.resolve_qualified_built::<H>(qualifier))
    }

    /// Path-selected config binding, this scope then parents.
    pub async fn resolve_config<H: Injectable>(&self, path: &str) -> Option<H> {
        if let Some(handle) = self.store.resolve_config_local::<H>(path) {
            return Some(handle);
        }

        self.parent.as_ref()?.resolve_config_built::<H>(path)
    }

    /// Sole config binding of `H::Target` (the type-only shorthand), this scope then
    /// parents.
    pub async fn resolve_config_sole<H: Injectable>(&self) -> Option<H> {
        if let Some(handle) = self.store.resolve_config_sole_local::<H>() {
            return Some(handle);
        }

        self.parent.as_ref()?.resolve_config_sole_built::<H>()
    }

    /// Every provider of the trait `H::Target` across this scope and its parents.
    pub async fn resolve_all<H: Injectable>(&self) -> Vec<H> {
        let mut all = self.store.collect_all_local::<H>();

        if let Some(parent) = &self.parent {
            all.extend(parent.collect_all_built::<H>());
        }

        all
    }

    /// Every provider of the trait `H::Target` keyed by qualifier, across this scope
    /// and its parents (a closer scope wins a qualifier collision).
    pub async fn resolve_keyed<H: Injectable>(&self) -> HashMap<String, H> {
        let mut keyed = match &self.parent {
            Some(parent) => parent.collect_keyed_built::<H>(),
            None => HashMap::new(),
        };

        keyed.extend(self.store.collect_keyed_local::<H>());

        keyed
    }

    pub(crate) fn insert(&mut self, component: BoxedComponent) {
        self.store.insert(component);
    }

    pub(crate) fn insert_config(&mut self, path: String, value: BoxedComponent) {
        self.store.insert_config(path, value);
    }

    pub(crate) fn register_provider(&mut self, provider: &ProviderDescriptor) {
        self.store.register_provider(provider);
    }

    pub(crate) fn into_parts(
        self,
    ) -> (
        ComponentScope,
        ScopeStore,
        Option<Arc<crate::container::ScopeContainer>>,
        Arc<crate::container::ScopeRegistry>,
    ) {
        (self.scope, self.store, self.parent, self.registry)
    }

    /// Whether a component of `type_id` has already been constructed or seeded in
    /// this scope.
    pub(crate) fn contains(&self, type_id: TypeId) -> bool {
        self.store.contains(type_id)
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
