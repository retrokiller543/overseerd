use std::hint::black_box;
use std::sync::Arc;
use std::time::Duration;

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use overseerd_core::{Resolver, ResolverSet};

macro_rules! resolver_types {
    ($($name:ident),+ $(,)?) => {
        $(
            struct $name;
            impl Resolver for $name {}
        )+
    };
}

resolver_types!(
    R00, R01, R02, R03, R04, R05, R06, R07, R08, R09, R10, R11, R12, R13, R14, R15, R16, R17, R18,
    R19, R20, R21, R22, R23, R24, R25, R26, R27, R28, R29, R30, R31,
);

fn resolver_sets() -> [(usize, ResolverSet); 3] {
    let mut one = ResolverSet::new();
    one.insert(Arc::new(R00));

    let mut eight = one.clone();
    eight.insert(Arc::new(R01));
    eight.insert(Arc::new(R02));
    eight.insert(Arc::new(R03));
    eight.insert(Arc::new(R04));
    eight.insert(Arc::new(R05));
    eight.insert(Arc::new(R06));
    eight.insert(Arc::new(R07));

    let mut thirty_two = eight.clone();
    thirty_two.insert(Arc::new(R08));
    thirty_two.insert(Arc::new(R09));
    thirty_two.insert(Arc::new(R10));
    thirty_two.insert(Arc::new(R11));
    thirty_two.insert(Arc::new(R12));
    thirty_two.insert(Arc::new(R13));
    thirty_two.insert(Arc::new(R14));
    thirty_two.insert(Arc::new(R15));
    thirty_two.insert(Arc::new(R16));
    thirty_two.insert(Arc::new(R17));
    thirty_two.insert(Arc::new(R18));
    thirty_two.insert(Arc::new(R19));
    thirty_two.insert(Arc::new(R20));
    thirty_two.insert(Arc::new(R21));
    thirty_two.insert(Arc::new(R22));
    thirty_two.insert(Arc::new(R23));
    thirty_two.insert(Arc::new(R24));
    thirty_two.insert(Arc::new(R25));
    thirty_two.insert(Arc::new(R26));
    thirty_two.insert(Arc::new(R27));
    thirty_two.insert(Arc::new(R28));
    thirty_two.insert(Arc::new(R29));
    thirty_two.insert(Arc::new(R30));
    thirty_two.insert(Arc::new(R31));

    [(1, one), (8, eight), (32, thirty_two)]
}

fn resolver_set_clone(c: &mut Criterion) {
    let mut group = c.benchmark_group("resolver_set_clone");

    for (resolvers, set) in resolver_sets() {
        group.bench_with_input(
            BenchmarkId::from_parameter(resolvers),
            &set,
            |bencher, set| bencher.iter(|| black_box(set.clone())),
        );
    }

    group.finish();
}

criterion_group! {
    name = benches;
    config = Criterion::default()
        .warm_up_time(Duration::from_secs(2))
        .measurement_time(Duration::from_secs(5))
        .sample_size(60);
    targets = resolver_set_clone
}
criterion_main!(benches);
