# Performance and liveness testing

Overseerd separates deterministic resource contracts from statistical timing benchmarks.

## Pull-request CI

The normal workspace test suite is the hard gate. Performance-sensitive code should have a
deterministic regression test wherever possible:

- count allocations for operations expected to be allocation-free;
- assert collection, task, channel, and semaphore counts return to their baseline;
- use `Weak` handles or `Drop` counters to prove objects are released;
- use Tokio's paused clock for timeout and idle-task behavior;
- exercise a large fixed workload and assert bounded state rather than elapsed time.

These tests run on every pull request and do not depend on runner speed. Examples:

- `crates/core/tests/resolver_set_performance.rs` proves that cloning a resolver context performs
  no allocations.
- `crates/di/tests/memory_contracts.rs` proves that resolving a component (`get`) allocates
  nothing, that request-time extraction (`extract`) leaks nothing across many calls, and that a
  built DI graph's footprint is bounded per component and fully reclaimed on drop — at several
  sizes and scope depths.

## Criterion benchmarks

Criterion and its dependencies live in the excluded `benchmarks` workspace, so normal workspace
builds, Clippy, and pull-request tests do not compile the heavier benchmark stack.

`.github/workflows/performance.yaml` runs the benches weekly and on demand, one CI job per bench
(a matrix), so a slow or failing bench is isolated and each uploads its own HTML report artifact
for trend comparison. Timing is advisory because GitHub-hosted runners are shared and noisy.

The suite covers the framework's hot paths:

| Bench                | What it measures |
| -------------------- | ---------------- |
| `di_hot_paths`       | Resolver-set clone; `get` across scope depth; `extract` for single/optional/collection shapes |
| `di_graph_memory`    | **Bytes allocated** to build small/moderate/large graphs across 1–8 scopes (custom allocation measurement, not wall-clock) |
| `config_resolution`  | `from_value` templating across tree sizes; `get_config` (defaults + clones) vs plain `get` |
| `serde_abstraction`  | Each generic-over-serde seam (`Responder`, `StreamEncode`/`StreamDecode`, `Json`/`Form`, `StompCodec`) against its raw serde baseline |
| `rpc_dispatch`       | `WireMessage` envelope encode/decode; `RpcRouter::dispatch` across routing-table sizes |
| `ws_pubsub_fanout`   | `emit` vs `publish` vs `publish_to` fan-out across 1/16/128 subscribers |

Run any bench locally with, for example:

```console
cargo bench --manifest-path benchmarks/Cargo.toml --bench di_hot_paths --locked
```

Verify every bench compiles and executes once (no timing) with:

```console
cargo bench --manifest-path benchmarks/Cargo.toml --bench <name> -- --test
```

### Benchmark support library

`benchmarks/src/` is a small support library the benches share:

- `alloc` — a tracking global allocator recording allocation count, cumulative bytes, and live
  bytes. A bench installs it as `#[global_allocator]` to measure heap traffic.
- `measure` — a custom Criterion measurement (`AllocBytes`) that reports allocated bytes instead of
  wall-clock time, so memory cost is a trended metric.
- `di` — builders that stand up DI graphs of a chosen size layered across scopes, over the public
  `overseerd-di` API.

### Guidance

Add benchmarks only for stable, important hot paths. Prefer deterministic PR tests when behavior
can be expressed as an allocation count, bounded queue/task count, or release invariant. When you
add a bench target, add it to both `benchmarks/Cargo.toml` and the CI matrix in
`.github/workflows/performance.yaml`. When you compare an abstraction to a raw baseline, keep both
lines in the same bench so a regression shows up as a widening gap.
