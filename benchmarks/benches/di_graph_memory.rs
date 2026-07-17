//! Memory footprint of building DI graphs of increasing size, layered across scopes.
//!
//! Reports **bytes** (via the custom [`AllocBytes`] measurement and the global [`TrackingAllocator`])
//! rather than wall-clock time, so `cargo bench` surfaces how the container's memory cost scales from
//! a small single-scope graph to a large eight-scope one. Two figures, in two groups:
//!
//! - `di_graph_build_traffic` — allocation *traffic*: cumulative bytes allocated to build the graph,
//!   including transients freed during the build (HashMap rehashing, temporary `Vec`s, the
//!   topological sort). This is build-time churn, not steady-state memory.
//! - `di_graph_retained` — *retained* footprint: bytes still live once the graph is built (measured
//!   with `iter_custom`, since Criterion's normal loop drops the graph inside the timed region). This
//!   is what a running application actually holds.
//!
//! Both are measured for two component families: `empty` (zero-sized) isolates pure DI overhead — the
//! `Arc<ArcSwap<Arc<T>>>` reload cell, the erased slot, and the container's index entry — while
//! `payload64` adds a representative 64-byte user payload, so the gap between them is the user-data
//! cost. `Throughput::Elements` is the component count, so each figure also reads per-component,
//! revealing whether per-component overhead grows with graph size (it should not — it converges down
//! as fixed per-scope overhead amortizes).

use std::hint::black_box;

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use futures::executor::block_on;
use overseerd_benchmarks::alloc::{self, TrackingAllocator};
use overseerd_benchmarks::di::{self, Entry};
use overseerd_benchmarks::measure::AllocBytes;

#[global_allocator]
static GLOBAL: TrackingAllocator = TrackingAllocator;

/// (label, components per scope, scope layers)
const GRAPHS: [(&str, usize, usize); 3] = [("small", 4, 1), ("moderate", 8, 4), ("large", 16, 8)];

fn families() -> [(&'static str, Vec<Entry>); 2] {
    [("empty", di::empty_roster()), ("payload64", di::roster())]
}

/// Allocation traffic to build each graph (includes freed transients).
fn graph_build_traffic(c: &mut Criterion<AllocBytes>) {
    let families = families();

    let mut group = c.benchmark_group("di_graph_build_traffic");

    for (family, roster) in &families {
        for (label, width, layers) in GRAPHS {
            let components = di::graph_component_count(width, layers);

            group.throughput(Throughput::Elements(components as u64));
            group.bench_with_input(
                BenchmarkId::new(*family, format!("{label}-{components}")),
                &(width, layers),
                |bencher, &(width, layers)| {
                    bencher.iter(|| black_box(block_on(di::build_graph(roster, width, layers))));
                },
            );
        }
    }

    group.finish();
}

/// Retained footprint of each built graph — the bytes it still holds once built.
fn graph_retained(c: &mut Criterion<AllocBytes>) {
    let families = families();

    let mut group = c.benchmark_group("di_graph_retained");

    for (family, roster) in &families {
        for (label, width, layers) in GRAPHS {
            let components = di::graph_component_count(width, layers);

            group.throughput(Throughput::Elements(components as u64));
            group.bench_with_input(
                BenchmarkId::new(*family, format!("{label}-{components}")),
                &(width, layers),
                |bencher, &(width, layers)| {
                    bencher.iter_custom(|iters| {
                        let mut retained: u64 = 0;

                        for _ in 0..iters {
                            let before = alloc::live_bytes();
                            let graph = block_on(di::build_graph(roster, width, layers));
                            let after = alloc::live_bytes();

                            retained += (after - before).max(0) as u64;

                            drop(graph);
                        }

                        retained
                    });
                },
            );
        }
    }

    group.finish();
}

criterion_group! {
    name = benches;
    config = Criterion::default().with_measurement(AllocBytes).without_plots();
    targets = graph_build_traffic, graph_retained
}
criterion_main!(benches);
