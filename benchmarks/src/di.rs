//! Builders that stand up DI graphs of a chosen size, layered across several scopes, using only the
//! public `overseerd-di` API.
//!
//! Each component is a distinct `C<N>` — const generics give every `N` its own `TypeId`, which is
//! exactly how the container keys components, so one roster of 128 types yields graphs from a
//! handful of components up to a large multi-scope tree. A graph is `layers` nested scopes
//! (root + children), each holding `width` components; resolving a root component from the deepest
//! scope therefore walks the whole parent chain — the worst case the resolver hits in practice.

use std::collections::HashMap;
use std::sync::Arc;

use overseerd_core::{ResolverSet, Scope, Singleton, TypeDescriptor};
use overseerd_di::{
    BoxedComponent, Component, ComponentDescriptor, Injectable, Live, ProviderDescriptor,
    ScopeContainer, ScopeRegistry,
};

/// The heap payload each component carries, so a graph's footprint includes representative
/// per-component user data rather than only the container's bookkeeping.
const PAYLOAD_BYTES: usize = 64;

/// The maximum graph a single [`roster`] can express (`width * layers <= 128`).
pub const ROSTER_SIZE: usize = 128;

/// A benchmark component. Const-generic `N` makes every instantiation a distinct type — and thus a
/// distinct container slot — from one generic definition.
pub struct C<const N: usize> {
    _payload: Box<[u8; PAYLOAD_BYTES]>,
}

impl<const N: usize> C<N> {
    fn new() -> Self {
        Self {
            _payload: Box::new([0u8; PAYLOAD_BYTES]),
        }
    }
}

impl<const N: usize> Component for C<N> {
    const ID: &'static str = "bench-component";
    const NAME: &'static str = "BenchComponent";
    type Handle = Arc<Self>;

    fn into_handle(self) -> Arc<Self> {
        Arc::new(self)
    }
}

/// A trait every `C<N>` provides, so a `Vec<Arc<dyn Svc>>` collection dependency has many providers
/// to gather — the multi-valued resolution path.
pub trait Svc: Send + Sync {}

impl<const N: usize> Svc for C<N> {}

/// One roster slot: a component's descriptor, a constructor for a fresh instance, and the provider
/// aliasing it as `dyn Svc`.
#[derive(Clone, Copy)]
pub struct Entry {
    desc: ComponentDescriptor,
    make: fn() -> BoxedComponent,
    provider: ProviderDescriptor,
}

/// Boxes a fresh `C<N>` into the type-erased slot the container seeds.
fn boxed<const N: usize>() -> BoxedComponent {
    BoxedComponent {
        ty: TypeDescriptor::of::<C<N>>(<C<N> as Component>::NAME),
        value: Box::new(Injectable::into_stored(<C<N> as Component>::into_handle(
            C::<N>::new(),
        ))),
    }
}

/// A manual (factory-less) descriptor for `C<N>`; the instance is seeded directly.
fn desc<const N: usize>() -> ComponentDescriptor {
    ComponentDescriptor::manual(
        <C<N> as Component>::ID,
        <C<N> as Component>::NAME,
        TypeDescriptor::of::<C<N>>(<C<N> as Component>::NAME),
        &Singleton,
    )
}

/// Re-erases an already-built `Arc<C<N>>` as `Arc<dyn Svc>` for storage under the trait's key —
/// the same job the `#[component(provide = ..)]` macro generates.
fn erase<const N: usize>(boxed: &BoxedComponent) -> BoxedComponent {
    let live = boxed
        .value
        .downcast_ref::<Live<C<N>>>()
        .expect("bench component slot has its own stored type");
    let concrete: Arc<C<N>> = live.snapshot();
    let as_trait: Arc<dyn Svc> = concrete;

    BoxedComponent {
        ty: TypeDescriptor::of::<dyn Svc>("dyn Svc"),
        value: Box::new(Injectable::into_stored(as_trait)),
    }
}

/// A provider aliasing `C<N>` under `dyn Svc`.
fn provider<const N: usize>() -> ProviderDescriptor {
    ProviderDescriptor {
        trait_ty: TypeDescriptor::of::<dyn Svc>("dyn Svc"),
        concrete_ty: TypeDescriptor::of::<C<N>>(<C<N> as Component>::NAME),
        qualifier: "bench",
        primary: false,
        erase: erase::<N>,
    }
}

macro_rules! roster_entries {
    ($($n:literal),* $(,)?) => {
        vec![ $( Entry { desc: desc::<$n>(), make: boxed::<$n>, provider: provider::<$n>() } ),* ]
    };
}

/// The full roster of [`ROSTER_SIZE`] distinct component types. Cheap to build (descriptors and
/// function pointers only — no instances yet); slice it to the size a graph needs.
pub fn roster() -> Vec<Entry> {
    roster_entries!(
        0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24,
        25, 26, 27, 28, 29, 30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40, 41, 42, 43, 44, 45, 46, 47,
        48, 49, 50, 51, 52, 53, 54, 55, 56, 57, 58, 59, 60, 61, 62, 63, 64, 65, 66, 67, 68, 69, 70,
        71, 72, 73, 74, 75, 76, 77, 78, 79, 80, 81, 82, 83, 84, 85, 86, 87, 88, 89, 90, 91, 92, 93,
        94, 95, 96, 97, 98, 99, 100, 101, 102, 103, 104, 105, 106, 107, 108, 109, 110, 111, 112,
        113, 114, 115, 116, 117, 118, 119, 120, 121, 122, 123, 124, 125, 126, 127
    )
}

macro_rules! layer_scopes {
    ($($name:ident : $rank:literal),* $(,)?) => {
        $(
            struct $name;

            impl Scope for $name {
                fn rank(&self) -> u8 {
                    $rank
                }

                fn name(&self) -> &'static str {
                    stringify!($name)
                }
            }
        )*
    };
}

layer_scopes!(
    Layer1: 1,
    Layer2: 2,
    Layer3: 3,
    Layer4: 4,
    Layer5: 5,
    Layer6: 6,
    Layer7: 7,
);

/// Scopes for child layers, indexed by layer number. Index `0` is the root (built as `Singleton`),
/// so it is a placeholder never opened as a child.
static LAYER_SCOPES: [&'static dyn Scope; 8] = [
    &Singleton, &Layer1, &Layer2, &Layer3, &Layer4, &Layer5, &Layer6, &Layer7,
];

/// The number of components a graph of `width * layers` holds.
pub fn graph_component_count(width: usize, layers: usize) -> usize {
    width * layers
}

/// Builds a graph of `layers` nested scopes, each seeding `width` distinct components from
/// `entries`. Panics if `width * layers` exceeds the roster.
pub async fn build_graph(entries: &[Entry], width: usize, layers: usize) -> Arc<ScopeContainer> {
    assert!(
        width * layers <= entries.len(),
        "graph needs {} components but roster holds {}",
        width * layers,
        entries.len()
    );
    assert!(layers >= 1 && layers <= LAYER_SCOPES.len(), "1..=8 layers");

    let registry = Arc::new(ScopeRegistry::new(HashMap::new(), Vec::new()));

    let root = &entries[0..width];
    let root_descs: Vec<ComponentDescriptor> = root.iter().map(|entry| entry.desc).collect();
    let root_seeds: Vec<BoxedComponent> = root.iter().map(|entry| (entry.make)()).collect();

    let mut container = ScopeContainer::build_root(
        &root_descs,
        root_seeds,
        ResolverSet::new(),
        Arc::clone(&registry),
    )
    .await
    .expect("root container builds");

    for layer in 1..layers {
        let slice = &entries[layer * width..(layer + 1) * width];
        let descs: Vec<ComponentDescriptor> = slice.iter().map(|entry| entry.desc).collect();
        let seeds: Vec<BoxedComponent> = slice.iter().map(|entry| (entry.make)()).collect();

        container = ScopeContainer::open_child(
            LAYER_SCOPES[layer],
            container,
            Arc::clone(&registry),
            &descs,
            seeds,
        )
        .await
        .expect("child scope builds");
    }

    container
}

/// Builds a single root scope seeding `count` components, each registered as a `dyn Svc` provider,
/// so a `Vec<Arc<dyn Svc>>` collection resolves all of them.
pub async fn build_with_providers(entries: &[Entry], count: usize) -> Arc<ScopeContainer> {
    let slice = &entries[0..count];
    let providers: Vec<ProviderDescriptor> = slice.iter().map(|entry| entry.provider).collect();
    let registry = Arc::new(ScopeRegistry::new(HashMap::new(), providers));

    let descs: Vec<ComponentDescriptor> = slice.iter().map(|entry| entry.desc).collect();
    let seeds: Vec<BoxedComponent> = slice.iter().map(|entry| (entry.make)()).collect();

    ScopeContainer::build_root(&descs, seeds, ResolverSet::new(), registry)
        .await
        .expect("provider root builds")
}

/// Resolves a root-scoped component (`C<0>`) through `container` — from the deepest scope this walks
/// the entire parent chain. Returns whether it resolved, for the bench to black-box.
pub fn resolve_root(container: &ScopeContainer) -> bool {
    container.get::<C<0>>().is_some()
}

/// Extracts a single `Arc<C<0>>` the way an HTTP/RPC handler parameter is injected.
pub async fn extract_single(container: &Arc<ScopeContainer>) -> bool {
    container.extract::<Arc<C<0>>>().await.is_ok()
}

/// Extracts an `Option<Arc<C<0>>>` — the optional-dependency path.
pub async fn extract_optional(container: &Arc<ScopeContainer>) -> bool {
    container.extract::<Option<Arc<C<0>>>>().await.is_ok()
}

/// Extracts every `dyn Svc` provider into a `Vec` — the multi-valued collection path.
pub async fn extract_collection(container: &Arc<ScopeContainer>) -> usize {
    container
        .extract::<Vec<Arc<dyn Svc>>>()
        .await
        .map(|all| all.len())
        .unwrap_or(0)
}
