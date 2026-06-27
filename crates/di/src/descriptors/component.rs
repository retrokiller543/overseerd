use arc_swap::{ArcSwap, Guard};
use std::marker::PhantomData;
use std::ops::Deref;
use std::{
    any::{Any, TypeId},
    collections::HashMap,
    fmt,
    future::Future,
    pin::Pin,
    sync::Arc,
};

use overseerd_core::{
    ComponentScope, DependencyDescriptor, ResolverCtx, ResolverSet, TypeDescriptor,
};
use overseerd_hooks::{HookDescriptor, no_hooks};

/// Metadata trait for types registerable as components.
///
/// Supplies the runtime identity used to synthesize a descriptor for a
/// manually-provided instance. Implemented by `#[component]` and `#[service]`.
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
/// cheap.
///
/// A scope does not box the handle directly — it boxes [`Stored`](Self::Stored),
/// the *slot* a handle is derived from. For `Arc<T>` and `Dep<T>` the slot is a
/// shared [`Live<T>`]: an `Arc<T>` snapshots it (one fixed instance), a `Dep<T>`
/// clones it (so a later swap is observed), and both find the *same* slot because
/// they share `Target = T`. A by-value handle is its own slot (`Stored = Self`),
/// so the round-trip is a plain clone. The container converts between slot and
/// handle through [`into_stored`](Self::into_stored) (once, at construction) and
/// [`from_stored`](Self::from_stored) (on every resolution).
pub trait Injectable: Clone + Send + Sync + 'static {
    /// The type this handle is stored and looked up under. `?Sized` so a trait
    /// object (`dyn Trait + Send + Sync`) can key its providers.
    type Target: ?Sized + 'static;

    /// The value actually held in a scope's box for this handle. `Arc<T>` and
    /// `Dep<T>` both store a shared `Live<T>`, so they alias one swappable slot;
    /// by-value handles store themselves.
    type Stored: Clone + Send + Sync + 'static;

    /// Wraps a freshly built handle into its stored slot. Called once, when the
    /// instance is first inserted into a scope.
    fn into_stored(self) -> Self::Stored;

    /// Derives this handle from a stored slot. Called on every resolution — a
    /// snapshot for `Arc<T>`, a shared clone for `Dep<T>`, a plain clone otherwise.
    fn from_stored(stored: &Self::Stored) -> Self;
}

/// A shared, swappable cell holding the current `Arc<T>` instance — the interior
/// mutability primitive behind [`Dep<T>`] (and `Cfg<T>` in the config layer).
///
/// Backed by [`arc_swap`]: every clone shares one underlying `ArcSwap`, so a
/// [`replace`](Self::replace) is observed by all clones, while [`snapshot`](Self::snapshot)
/// and [`get`](Self::get) pin the generation current at the call. This is what lets a
/// config reload swap a component (or config value) in place without recreating its
/// consumers.
pub struct Live<T: ?Sized> {
    inner: Arc<ArcSwap<Arc<T>>>,
}

/// A reloadable dependency handle: a stable handle to a component slot whose
/// instance may be swapped at runtime (e.g. by a config reload).
///
/// A `Dep<T>` field keeps working across a swap, observing the new instance on its
/// next read; existing snapshots keep the old instance alive until dropped. It is
/// the same primitive as [`Live<T>`] — an alias chosen at the injection site to opt
/// into live, rather than fixed `Arc<T>`, semantics.
pub type Dep<T> = Live<T>;

/// A guard pinning one generation of a [`Live<T>`], dereferencing to `T`. Holding it
/// keeps observing the generation current when [`get`](Live::get) was called; it
/// borrows the `Live`, discouraging holding it across a long await (which would pin
/// the old instance and delay a reload). Returned by both [`Dep<T>`] and `Cfg<T>`.
pub struct LiveRef<'a, T: ?Sized> {
    guard: Guard<Arc<Arc<T>>>,
    _marker: PhantomData<&'a Live<T>>,
}

impl<T: ?Sized> Deref for LiveRef<'_, T> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.guard
    }
}

impl<T: ?Sized + Send + Sync + 'static> Clone for Live<T> {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

impl<T: ?Sized + Send + Sync + 'static> fmt::Debug for Live<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Live").finish_non_exhaustive()
    }
}

impl<T: ?Sized + Send + Sync + 'static> Live<T> {
    /// Wraps an instance in a fresh slot. Each call creates an independent cell, so
    /// this is the seam where a constructed instance becomes reloadable.
    pub fn new(value: Arc<T>) -> Self {
        Self {
            inner: Arc::new(ArcSwap::new(Arc::new(value))),
        }
    }

    /// Publishes a new instance into the slot. Every clone of this `Live` (every
    /// `Dep<T>` derived from it) observes the new value on its next read — the
    /// swap a config reload performs at commit.
    pub fn replace(&self, value: Arc<T>) {
        self.inner.store(Arc::new(value));
    }

    /// An owned `Arc` snapshot of the current instance — stable once taken. Prefer
    /// this over [`get`](Self::get) for anything held across an `.await`.
    pub fn snapshot(&self) -> Arc<T> {
        self.inner.load_full().as_ref().clone()
    }

    /// A guard pinning the current instance, dereferencing to `T`, for short
    /// synchronous reads.
    pub fn get(&self) -> LiveRef<'_, T> {
        LiveRef {
            guard: self.inner.load(),
            _marker: PhantomData,
        }
    }
}

impl<T: ?Sized + Send + Sync + 'static> Injectable for Dep<T> {
    type Target = T;
    type Stored = Live<T>;

    fn into_stored(self) -> Live<T> {
        self
    }

    fn from_stored(stored: &Live<T>) -> Self {
        stored.clone()
    }
}

impl<T: ?Sized + Send + Sync + 'static> Injectable for Arc<T> {
    type Target = T;
    type Stored = Live<T>;

    fn into_stored(self) -> Live<T> {
        Live::new(self)
    }

    fn from_stored(stored: &Live<T>) -> Self {
        stored.snapshot()
    }
}

/// Field wrapper marking a dependency as **runtime-provided**.
///
/// A `Dynamic<H>` dependency is satisfied by a provider registered at runtime
/// rather than one discovered at build, so the edge is exempt from static
/// dependency validation. It wraps the resolved handle `H` and derefs to it for
/// transparent access.
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
/// dependency, so a missing provider is a `cargo check` error.
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
/// *every* single dependency (concrete and trait-object) as a lazy bound.
pub trait Wired {}

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

/// The instances and trait-providers held by one scope.
///
/// Shared by both the under-construction [`ComponentConstructionContext`] and the
/// frozen [`ScopeContainer`](crate::container::ScopeContainer); each owns one
/// `ScopeStore` and layers parent scopes on top for resolution. All lookups here
/// are **scope-local** — walking the parent chain is the caller's job. Config values
/// are *not* held here: config is an external resolver, not part of the container.
#[derive(Default)]
pub(crate) struct ScopeStore {
    pub(crate) components: HashMap<TypeId, BoxedComponent>,
    pub(crate) providers: HashMap<TypeId, Vec<ProviderInstance>>,
}

/// Recovers handle `H` from a boxed slot: downcasts to its stored representation
/// (`H::Stored`) and derives the handle. This is the single seam through which
/// every resolution converts storage back into a handle — a snapshot for `Arc<T>`,
/// a shared clone for `Dep<T>`, a plain clone for a by-value handle. `None` if the
/// box holds a different stored type.
///
/// Public so external resolvers (e.g. the config store) can recover a handle from a
/// seed they hold without re-implementing the downcast.
pub fn from_boxed<H: Injectable>(boxed: &BoxedComponent) -> Option<H> {
    boxed.value.downcast_ref::<H::Stored>().map(H::from_stored)
}

impl ScopeStore {
    /// Single concrete-or-primary-provider lookup, scope-local. `None` if absent or
    /// ambiguous.
    pub(crate) fn resolve_local<H: Injectable>(&self) -> Option<H> {
        let type_id = TypeId::of::<H::Target>();

        if let Some(component) = self.components.get(&type_id) {
            return from_boxed::<H>(component);
        }

        let chosen = pick_single(self.providers.get(&type_id)?)?;

        from_boxed::<H>(&chosen.value)
    }

    /// Qualifier-selected single provider, scope-local.
    pub(crate) fn resolve_qualified_local<H: Injectable>(&self, qualifier: &str) -> Option<H> {
        let type_id = TypeId::of::<H::Target>();
        let entry = self
            .providers
            .get(&type_id)?
            .iter()
            .find(|entry| entry.qualifier == qualifier)?;

        from_boxed::<H>(&entry.value)
    }

    /// Every scope-local provider of the trait `H::Target`.
    pub(crate) fn collect_all_local<H: Injectable>(&self) -> Vec<H> {
        let type_id = TypeId::of::<H::Target>();

        self.providers
            .get(&type_id)
            .into_iter()
            .flatten()
            .filter_map(|entry| from_boxed::<H>(&entry.value))
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
                let value = from_boxed::<H>(&entry.value)?;

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

    pub(crate) fn contains(&self, type_id: TypeId) -> bool {
        self.components.contains_key(&type_id)
    }
}

/// Context passed to a component factory during construction.
///
/// Holds the components built so far in this scope plus a link to the parent scope
/// chain, so a factory can resolve dependencies from its own scope or any
/// longer-lived one. Factories call `resolve`/`resolve_all`/`resolve_keyed` to
/// obtain their component dependencies; *external* sources (config) are reached
/// through the [`ResolverCtx`] impl (`cx.get::<ConfigStore>()`).
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
    /// External resolvers (e.g. the config store) threaded in by the runtime, so a
    /// factory parameter like `Cfg<T>` resolves through `cx.get::<ConfigStore>()`.
    resolvers: ResolverSet,
}

impl ResolverCtx for ComponentConstructionContext {
    fn resolver(&self, kind: TypeId) -> Option<&dyn Any> {
        self.resolvers.resolver(kind)
    }
}

impl ComponentConstructionContext {
    pub(crate) fn new(
        scope: ComponentScope,
        parent: Option<Arc<crate::container::ScopeContainer>>,
        registry: Arc<crate::container::ScopeRegistry>,
        resolvers: ResolverSet,
    ) -> Self {
        Self {
            scope,
            store: ScopeStore::default(),
            parent,
            registry,
            resolvers,
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
        ResolverSet,
    ) {
        (
            self.scope,
            self.store,
            self.parent,
            self.registry,
            self.resolvers,
        )
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

/// One way to construct a component, carrying the dependencies *that constructor*
/// needs (zero or many).
///
/// A component owns a slice of these (its `{Type}Factories` distributed slice): the
/// `#[component]`/`#[service]` field-injection **default** (`default: true`) plus any
/// explicit factories contributed by an `#[init]` or `factory = path` — each
/// appending its own entry. The effective one is chosen by
/// [`ComponentDescriptor::effective_factory`].
#[derive(Clone, Copy)]
pub struct ComponentFactoryDescriptor {
    pub construct: ComponentFactory,
    /// The factory's dependency edges, reported at runtime. Read only at build.
    pub dependencies: fn() -> Vec<DependencyDescriptor>,
    /// The field-injection default, used only when no explicit factory exists.
    pub default: bool,
}

impl fmt::Debug for ComponentFactoryDescriptor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ComponentFactoryDescriptor")
            .field("dependencies", &(self.dependencies)())
            .field("default", &self.default)
            .finish_non_exhaustive()
    }
}

/// A component type's own construction factories.
///
/// Implemented for each `#[component]`/`#[service]` by the macro to return that
/// type's `{Type}Factories` distributed slice.
pub trait ComponentFactories {
    /// Every factory contributed to this component type.
    fn factories() -> &'static [ComponentFactoryDescriptor];
}

/// Static metadata describing a component and how to construct it.
///
/// `factories` returns the component's `{Type}Factories` slice (the field-injection
/// default plus any `#[init]`/`factory =` contributions); deps live on each factory,
/// not here. There is exactly one `ComponentDescriptor` per type.
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
    pub factories: fn() -> &'static [ComponentFactoryDescriptor],
    /// The component's `{Type}Hooks` slice (its `#[hook]` methods). Empty for a type
    /// that declares none — and for every manually-seeded instance.
    pub hooks: fn() -> &'static [HookDescriptor],
}

/// The empty factory slice for a manually-provided instance: nothing to construct,
/// the value is seeded into the container directly.
fn no_factories() -> &'static [ComponentFactoryDescriptor] {
    &[]
}

impl ComponentDescriptor {
    pub const fn of<T: Component>() -> Self {
        Self {
            id: T::ID,
            name: T::NAME,
            ty: TypeDescriptor::of::<T>(T::NAME),
            scope: ComponentScope::Singleton,
            factories: no_factories,
            hooks: no_hooks,
        }
    }

    /// A descriptor for a manually-provided instance (no factory): the value is
    /// seeded into the container directly. Used for framework-seeded injectables
    /// (the peer, the shutdown handle) that carry a non-singleton or custom identity
    /// that [`of`](Self::of) does not express.
    pub const fn manual(
        id: &'static str,
        name: &'static str,
        ty: TypeDescriptor,
        scope: ComponentScope,
    ) -> Self {
        Self {
            id,
            name,
            ty,
            scope,
            factories: no_factories,
            hooks: no_hooks,
        }
    }

    /// The factory the container should use: an explicit one if present (the default
    /// is its fallback), or `None` for a manually-provided instance (empty slice).
    /// Errors if more than one explicit factory exists for the type.
    pub fn effective_factory(&self) -> crate::Result<Option<&'static ComponentFactoryDescriptor>> {
        let factories = (self.factories)();

        if factories.len() == 1 {
            return Ok(factories.first());
        }

        let mut explicit = factories.iter().filter(|factory| !factory.default);
        let first = explicit.next();

        if first.is_some() && explicit.next().is_some() {
            return Err(crate::Error::AmbiguousFactory(self.name.to_string()));
        }

        match first {
            Some(factory) => Ok(Some(factory)),

            None => Ok(factories.iter().find(|factory| factory.default)),
        }
    }

    /// The dependencies of the effective factory (empty for a manual instance, or if
    /// the factory choice is ambiguous — that is surfaced separately during validation).
    pub fn dependencies(&self) -> Vec<DependencyDescriptor> {
        match self.effective_factory().ok().flatten() {
            Some(factory) => (factory.dependencies)(),

            None => Vec::new(),
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
            .field("dependencies", &self.dependencies())
            .finish_non_exhaustive()
    }
}
