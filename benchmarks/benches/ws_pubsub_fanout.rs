//! WebSocket pub/sub fan-out — the three publish paths, across subscriber counts.
//!
//! `Publisher`/`TopicBus` delegate to the protocol-neutral [`SubscriptionRegistry`], which is what
//! this bench drives directly (it is generic over the frame type, so no protocol wiring is needed):
//!
//! - `emit` → `publish_frames`: fire-and-forget, `try_send`, drops for a full/slow subscriber.
//! - `publish` → `deliver_frames::<16>`: backpressured, up to 16 subscribers awaited concurrently.
//! - `publish_to::<128>` → `deliver_frames::<128>`: backpressured, fully concurrent fan-out.
//!
//! Each is measured against 1/16/128 live subscribers whose receivers are continuously drained, so
//! the figures reflect real delivery rather than a filling buffer. The contrast between the sync
//! `try_send` path and the backpressured `send().await` paths is the trade-off `emit` vs `publish`
//! exists to offer.

use std::hint::black_box;
use std::sync::Arc;
use std::time::Duration;

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use overseerd_axum::SubscriptionRegistry;
use tokio::runtime::Runtime;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

const DESTINATION: &str = "/bench/topic";
const FRAME_BYTES: usize = 64;
const SUBSCRIBER_COUNTS: [usize; 3] = [1, 16, 128];

type Frame = Vec<u8>;

/// Stands up a registry with `subscribers` subscriptions to one destination, each with a spawned
/// task continuously draining its receiver so publishes always reach a live consumer. The returned
/// join handles (and thus the receivers) must be kept alive for the duration of the benchmark.
fn setup(
    runtime: &Runtime,
    subscribers: usize,
) -> (Arc<SubscriptionRegistry<Frame>>, Vec<JoinHandle<()>>) {
    let registry = Arc::new(SubscriptionRegistry::<Frame>::new());
    let mut drains = Vec::with_capacity(subscribers);

    for i in 0..subscribers {
        let (tx, mut rx) = mpsc::channel::<Frame>(1024);
        let conn = registry.register();

        registry.subscribe(conn, &format!("sub-{i}"), DESTINATION, tx);
        drains.push(runtime.spawn(async move { while rx.recv().await.is_some() {} }));
    }

    (registry, drains)
}

/// `emit`'s engine: synchronous fire-and-forget fan-out.
fn emit_fanout(c: &mut Criterion) {
    let runtime = Runtime::new().expect("tokio runtime");
    let frame = vec![0u8; FRAME_BYTES];

    let mut group = c.benchmark_group("ws_emit_publish_frames");

    for subscribers in SUBSCRIBER_COUNTS {
        let (registry, _drains) = setup(&runtime, subscribers);

        group.bench_with_input(
            BenchmarkId::from_parameter(subscribers),
            &registry,
            |bencher, registry| {
                bencher.iter(|| {
                    registry
                        .publish_frames(DESTINATION, |_sub_id, _msg_id| black_box(frame.clone()))
                });
            },
        );
    }

    group.finish();
}

/// `publish`'s engine: backpressured fan-out, 16 subscribers awaited concurrently.
fn publish_fanout(c: &mut Criterion) {
    let runtime = Runtime::new().expect("tokio runtime");
    let frame = vec![0u8; FRAME_BYTES];

    let mut group = c.benchmark_group("ws_publish_deliver_16");

    for subscribers in SUBSCRIBER_COUNTS {
        let (registry, _drains) = setup(&runtime, subscribers);

        group.bench_with_input(
            BenchmarkId::from_parameter(subscribers),
            &registry,
            |bencher, registry| {
                bencher.to_async(&runtime).iter(|| async {
                    registry
                        .deliver_frames::<16>(DESTINATION, |_sub_id, _msg_id| {
                            black_box(frame.clone())
                        })
                        .await
                });
            },
        );
    }

    group.finish();
}

/// `publish_to::<128>`'s engine: backpressured fan-out, fully concurrent.
fn publish_to_fanout(c: &mut Criterion) {
    let runtime = Runtime::new().expect("tokio runtime");
    let frame = vec![0u8; FRAME_BYTES];

    let mut group = c.benchmark_group("ws_publish_to_deliver_128");

    for subscribers in SUBSCRIBER_COUNTS {
        let (registry, _drains) = setup(&runtime, subscribers);

        group.bench_with_input(
            BenchmarkId::from_parameter(subscribers),
            &registry,
            |bencher, registry| {
                bencher.to_async(&runtime).iter(|| async {
                    registry
                        .deliver_frames::<128>(DESTINATION, |_sub_id, _msg_id| {
                            black_box(frame.clone())
                        })
                        .await
                });
            },
        );
    }

    group.finish();
}

criterion_group! {
    name = benches;
    config = Criterion::default()
        .warm_up_time(Duration::from_secs(1))
        .measurement_time(Duration::from_secs(4))
        .sample_size(80);
    targets = emit_fanout, publish_fanout, publish_to_fanout
}
criterion_main!(benches);
