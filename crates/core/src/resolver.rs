//! The resolver abstraction: a type-keyed set of dependency sources.
//!
//! Resolution in Overseerd used to be a zoo of methods on the container — one each for
//! single, qualified, config-by-path, config-by-type, collection, and keyed lookups,
//! every one duplicated across the construction-time and request-time paths. The
//! container also *owned* config values, coupling the DI engine to the config layer.
//!
//! Instead, a resolution context is a **typemap of [`Resolver`]s**. Each source of
//! values — the component container, the config store, any future source — is a
//! resolver, retrieved from the context by its own concrete type. An extractor asks the
//! context for the source it needs (`ctx.get::<ComponentSource>()`,
//! `ctx.get::<ConfigStore>()`) and resolves through it. Adding a new kind of dependency
//! source is a new `Resolver` impl registered into the context — no change to the
//! context type, and no leakage of one layer's concerns into another. In particular the
//! component container stays unaware that config (or anything beyond raw components)
//! exists.

use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::sync::Arc;

/// A source of resolved values of one kind (components, config, …).
///
/// A resolver is stored in, and fetched from, a [`ResolverCtx`] by its own concrete
/// type. The trait itself is just the marker that makes a type eligible; the actual
/// resolution methods are inherent to each concrete resolver, so they stay generic over
/// the handle type without forcing object safety on this trait.
pub trait Resolver: Any + Send + Sync {}

/// A type-keyed set of [`Resolver`]s — the context an extractor resolves against.
///
/// Both the construction-time and request-time contexts implement this. It is
/// object-safe (keyed by [`TypeId`], returning `&dyn Any`), so it can be passed as
/// `&dyn ResolverCtx` — which is how a hook's erased call reaches the component source
/// without the hook layer naming the container.
pub trait ResolverCtx {
    /// The registered resolver whose concrete type has the given [`TypeId`], if any.
    fn resolver(&self, kind: TypeId) -> Option<&dyn Any>;
}

/// Ergonomic, generic accessor over a [`ResolverCtx`]. Blanket-implemented, so any
/// context (or `&dyn ResolverCtx`) gets `ctx.get_resolver::<R>()` for free.
///
/// Named `get_resolver` rather than `get` so it never shadows (or is shadowed by) an
/// inherent `get` on a concrete context — e.g. `ScopeContainer::get::<T: Component>`.
pub trait ResolverCtxExt: ResolverCtx {
    /// The registered resolver of concrete type `R`, if present.
    fn get_resolver<R: Resolver>(&self) -> Option<&R> {
        self.resolver(TypeId::of::<R>())?.downcast_ref::<R>()
    }
}

impl<T: ResolverCtx + ?Sized> ResolverCtxExt for T {}

/// A type-keyed bag of [`Resolver`]s — the concrete backing a context delegates its
/// [`ResolverCtx`] impl to.
///
/// Each resolver is stored as `Arc<dyn Any + Send + Sync>` (a plain unsizing coercion
/// from `Arc<R>`, needing no trait upcasting) keyed by its concrete [`TypeId`]. The map
/// itself is copy-on-write behind an [`Arc`], so cloning a set on a request hot path is
/// allocation-free while mutation still gives each scope its own resolver namespace.
/// Every clone observes the same underlying resolver instances, so a config store swapped
/// in place by a reload is seen everywhere it was threaded.
#[derive(Default, Clone)]
pub struct ResolverSet {
    map: Arc<HashMap<TypeId, Arc<dyn Any + Send + Sync>>>,
}

impl ResolverSet {
    /// An empty set.
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers `resolver` under its concrete type `R`, replacing any previous one.
    pub fn insert<R: Resolver>(&mut self, resolver: Arc<R>) {
        Arc::make_mut(&mut self.map).insert(TypeId::of::<R>(), resolver);
    }

    /// The registered resolver of concrete type `R`, as a shared handle, if present.
    pub fn get_arc<R: Resolver>(&self) -> Option<Arc<R>> {
        self.map
            .get(&TypeId::of::<R>())
            .cloned()
            .and_then(|any| any.downcast::<R>().ok())
    }
}

impl ResolverCtx for ResolverSet {
    fn resolver(&self, kind: TypeId) -> Option<&dyn Any> {
        self.map.get(&kind).map(|arc| arc.as_ref() as &dyn Any)
    }
}

#[cfg(test)]
mod tests;
