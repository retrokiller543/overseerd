//! Deterministic memory contracts for the DI container hot paths.
//!
//! These are complexity/leak contracts, not wall-clock benchmarks: they assert allocation counts
//! and live-byte deltas, which are independent of runner speed and so run on every pull request
//! (the statistical memory *trend* across graph sizes lives in `benchmarks/di_graph_memory`). The
//! properties proven here:
//!
//! - resolving a component (`get`) allocates nothing — it is an `Arc` bump and a map walk;
//! - request-time extraction (`extract`) leaks nothing — everything it allocates is freed;
//! - a built graph's retained footprint is bounded per component and is fully reclaimed on drop.
//!
//! All checks run inside a single `#[test]` so only one thread touches the global counters.

use std::alloc::{GlobalAlloc, Layout, System};
use std::collections::HashMap;
use std::hint::black_box;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};

use futures::executor::block_on;
use overseerd_core::{ResolverSet, Scope, Singleton, TypeDescriptor};
use overseerd_di::{
    BoxedComponent, Component, ComponentDescriptor, Injectable, ScopeContainer, ScopeRegistry,
};

// --- allocation tracking -------------------------------------------------------------------------

static ALLOCATIONS: AtomicU64 = AtomicU64::new(0);
static LIVE_BYTES: AtomicI64 = AtomicI64::new(0);

struct TrackingAllocator;

unsafe impl GlobalAlloc for TrackingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        LIVE_BYTES.fetch_add(layout.size() as i64, Ordering::Relaxed);

        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        LIVE_BYTES.fetch_add(layout.size() as i64, Ordering::Relaxed);

        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        LIVE_BYTES.fetch_sub(layout.size() as i64, Ordering::Relaxed);

        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        LIVE_BYTES.fetch_add(new_size as i64 - layout.size() as i64, Ordering::Relaxed);

        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static ALLOCATOR: TrackingAllocator = TrackingAllocator;

/// The allocation traffic and net live-byte change across `f`.
struct Delta {
    allocations: u64,
    net_live: i64,
}

fn measure<R>(f: impl FnOnce() -> R) -> (R, Delta) {
    let allocations_before = ALLOCATIONS.load(Ordering::SeqCst);
    let live_before = LIVE_BYTES.load(Ordering::SeqCst);

    let value = f();

    let delta = Delta {
        allocations: ALLOCATIONS.load(Ordering::SeqCst) - allocations_before,
        net_live: LIVE_BYTES.load(Ordering::SeqCst) - live_before,
    };

    (value, delta)
}

// --- a small graph fixture -----------------------------------------------------------------------

const PAYLOAD_BYTES: usize = 64;

struct C<const N: usize> {
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
    const ID: &'static str = "mem-component";
    const NAME: &'static str = "MemComponent";
    type Handle = Arc<Self>;

    fn into_handle(self) -> Arc<Self> {
        Arc::new(self)
    }
}

#[derive(Clone, Copy)]
struct Entry {
    desc: ComponentDescriptor,
    make: fn() -> BoxedComponent,
}

fn boxed<const N: usize>() -> BoxedComponent {
    BoxedComponent {
        ty: TypeDescriptor::of::<C<N>>(<C<N> as Component>::NAME),
        value: Box::new(Injectable::into_stored(<C<N> as Component>::into_handle(
            C::<N>::new(),
        ))),
    }
}

fn desc<const N: usize>() -> ComponentDescriptor {
    ComponentDescriptor::manual(
        <C<N> as Component>::ID,
        <C<N> as Component>::NAME,
        TypeDescriptor::of::<C<N>>(<C<N> as Component>::NAME),
        &Singleton,
    )
}

macro_rules! roster {
    ($($n:literal),* $(,)?) => {
        vec![ $( Entry { desc: desc::<$n>(), make: boxed::<$n> } ),* ]
    };
}

fn roster() -> Vec<Entry> {
    roster!(
        0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24,
        25, 26, 27, 28, 29, 30, 31
    )
}

struct Layer1;
struct Layer2;
struct Layer3;

impl Scope for Layer1 {
    fn rank(&self) -> u8 {
        1
    }

    fn name(&self) -> &'static str {
        "Layer1"
    }
}

impl Scope for Layer2 {
    fn rank(&self) -> u8 {
        2
    }

    fn name(&self) -> &'static str {
        "Layer2"
    }
}

impl Scope for Layer3 {
    fn rank(&self) -> u8 {
        3
    }

    fn name(&self) -> &'static str {
        "Layer3"
    }
}

static LAYER_SCOPES: [&'static dyn Scope; 4] = [&Singleton, &Layer1, &Layer2, &Layer3];

async fn build_graph(entries: &[Entry], width: usize, layers: usize) -> Arc<ScopeContainer> {
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
    .expect("root builds");

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
        .expect("child builds");
    }

    container
}

// --- the contracts -------------------------------------------------------------------------------

/// Generous per-component ceiling for a graph's retained footprint: the 64-byte payload plus the
/// `Arc`, the `Live` cell, and the map slot the container keeps for it. Well below any figure that
/// would indicate the container itself is bloating per component.
const PER_COMPONENT_CEILING: i64 = 2048;

#[test]
fn di_memory_contracts() {
    let entries = roster();
    let container = block_on(build_graph(&entries, 8, 4));
    let arc_container = Arc::clone(&container);

    // Warm up the one-time, lazily-initialized thread-locals (arc_swap's debt slot, the futures
    // block_on parker) so the measured regions see only steady-state allocation behaviour.
    black_box(container.get::<C<0>>());
    block_on(async { black_box(arc_container.extract::<Arc<C<0>>>().await.is_ok()) });

    // Resolving a component allocates nothing: it is a map walk plus an `Arc` snapshot.
    let (_, resolve_delta) = measure(|| {
        for _ in 0..10_000 {
            black_box(container.get::<C<0>>());
        }
    });

    assert_eq!(
        resolve_delta.allocations, 0,
        "ScopeContainer::get allocated on the resolution hot path"
    );

    // Request-time extraction leaks nothing across many calls.
    let (_, extract_delta) = measure(|| {
        block_on(async {
            for _ in 0..10_000 {
                black_box(arc_container.extract::<Arc<C<0>>>().await.is_ok());
            }
        })
    });

    assert_eq!(
        extract_delta.net_live, 0,
        "ScopeContainer::extract leaked {} live bytes over 10k calls",
        extract_delta.net_live
    );

    drop(container);
    drop(arc_container);

    // A built graph's footprint is bounded per component and is fully reclaimed on drop, at every
    // size and scope depth.
    let mut previous_retained = 0;

    for (label, width, layers) in [("small", 2, 1), ("moderate", 4, 2), ("large", 8, 4)] {
        let components = (width * layers) as i64;

        let (graph, build) = measure(|| block_on(build_graph(&entries, width, layers)));
        let retained = build.net_live;

        assert!(
            retained > 0,
            "{label} graph retained no memory — measurement is broken"
        );
        assert!(
            retained / components < PER_COMPONENT_CEILING,
            "{label} graph retained {} bytes/component (ceiling {PER_COMPONENT_CEILING})",
            retained / components
        );
        assert!(
            retained >= previous_retained,
            "{label} graph retained less than a smaller graph — non-monotonic footprint"
        );

        previous_retained = retained;

        let (_, teardown) = measure(|| drop(graph));

        assert_eq!(
            build.net_live + teardown.net_live,
            0,
            "{label} graph leaked {} bytes after drop",
            build.net_live + teardown.net_live
        );
    }
}
