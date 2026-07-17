//! Memory footprint of building DI graphs of increasing size, layered across scopes.
//!
//! This bench reports **bytes allocated** (via [`AllocBytes`] and the global
//! [`TrackingAllocator`]) rather than wall-clock time, so `cargo bench` surfaces how the container's
//! memory cost scales from a small single-scope graph to a large eight-scope one. Because the graph
//! is dropped only *after* each measurement ends, the figure is allocation *traffic* to build the
//! graph — the retained-footprint and leak questions are covered by the deterministic contracts in
//! `crates/di/tests/`.
//!
//! `Throughput::Elements` is set to the component count, so the report also shows bytes-per-component
//! — the number that reveals whether per-component overhead grows with graph size (it should not).

use std::hint::black_box;

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use futures::executor::block_on;
use overseerd_benchmarks::alloc::TrackingAllocator;
use overseerd_benchmarks::di;
use overseerd_benchmarks::measure::AllocBytes;

#[global_allocator]
static GLOBAL: TrackingAllocator = TrackingAllocator;

/// (label, components per scope, scope layers)
const GRAPHS: [(&str, usize, usize); 3] = [("small", 4, 1), ("moderate", 8, 4), ("large", 16, 8)];

fn graph_build_memory(c: &mut Criterion<AllocBytes>) {
    let roster = di::roster();

    let mut group = c.benchmark_group("di_graph_build_memory");

    for (label, width, layers) in GRAPHS {
        let components = di::graph_component_count(width, layers);

        group.throughput(Throughput::Elements(components as u64));
        group.bench_with_input(
            BenchmarkId::new(label, components),
            &(width, layers),
            |bencher, &(width, layers)| {
                bencher.iter(|| black_box(block_on(di::build_graph(&roster, width, layers))));
            },
        );
    }

    group.finish();
}

criterion_group! {
    name = benches;
    config = Criterion::default().with_measurement(AllocBytes).without_plots();
    targets = graph_build_memory
}
criterion_main!(benches);
