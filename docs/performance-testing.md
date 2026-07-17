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

These tests run on every pull request and do not depend on runner speed. For example,
`resolver_set_performance` proves that cloning a resolver context performs no allocations.

## Criterion benchmarks

Criterion and its dependencies live in the excluded `benchmarks` workspace, so normal workspace
builds, Clippy, and pull-request tests do not compile the heavier benchmark stack.

`.github/workflows/performance.yaml` runs Criterion weekly and on demand. HTML reports are retained
as workflow artifacts for trend comparison, but timing is advisory because GitHub-hosted runners
are shared and noisy.

Run the same suite locally with:

```console
cargo bench --manifest-path benchmarks/Cargo.toml --bench di_hot_paths --locked
```

Add benchmarks only for stable, important hot paths. Prefer deterministic PR tests when behavior
can be expressed as an allocation count, bounded queue/task count, or release invariant.
