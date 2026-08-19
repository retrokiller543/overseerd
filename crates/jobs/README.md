# upwell-jobs

> The Upwell job scheduler: run `async` methods on an interval or cron schedule as
> supervised background tasks, or schedule work dynamically at run time.

Part of the [Upwell](../../README.md) framework — a non-protocol
[`Plugin`](../app/README.md) layered over the protocol-agnostic `upwell-app` core, so it
composes with any protocol (RPC, HTTP) or runs on its own.

## Role

`upwell-jobs` turns methods into scheduled background jobs. It owns:

- the [`Schedule`] model (interval via `humantime`, cron via `croner` incl. `@`-nicknames),
- the `JOBS` link-time slice and [`JobDescriptor`] each `#[job]` registers into,
- the [`JobScheduler`] singleton — an injectable that spawns one supervised loop per job on a
  `Startup` hook and cancels them all on `Drop` — plus its run-time [`JobScheduler::schedule`]
  API and cancellable [`JobHandle`],
- the [`JobsPlugin`] that registers the scheduler into an app.

The `#[job]`/`#[jobs]` macros live in the sibling [`upwell-jobs-macros`](../jobs-macros/README.md)
crate (re-exported here), keeping the codegen out of the runtime crate.

## Usage

Enable the `jobs` feature on the [`upwell`](../../README.md) facade and register
[`JobsPlugin`]. Mark `async` methods in a `#[jobs]` impl block:

```rust
use upwell::jobs::{JobsPlugin, jobs};
use upwell::{component, Dep};

#[component]
struct Reaper { db: Dep<Db> }

#[jobs]
impl Reaper {
    #[job(every = "30s")]                 // fixed interval (humantime)
    async fn sweep(&self) { self.db.snapshot().cleanup().await; }

    #[job(cron = "@hourly")]              // cron expression / nickname
    async fn report(&self, metrics: Dep<Metrics>) { metrics.snapshot().flush().await; }
}

let app = App::builder("worker")
    .auto_discover()
    .register_plugin::<JobsPlugin>()
    .build().await?;
```

Job parameters after `&self` are resolved from the container per run — the same shapes an
`#[init]` constructor takes (`Arc<T>`, `Dep<T>`, `Cfg<T>`, …), no `Inject<_>` wrapper.

**Dynamic jobs** (e.g. loaded from a database) — inject `Arc<JobScheduler>` and schedule at
run time; the returned [`JobHandle`] cancels (un-registers) the job:

```rust
let handle = scheduler.schedule(Schedule::interval("5m")?, || async { poll().await?; Ok(()) });
handle.cancel();
```

**Standalone job runner** — the scheduler needs no network protocol. An app built with
`JobsPlugin` and driven by `App::run()` (never `serve()`) is a dedicated scheduler/worker
process with no request surface. Pair `jobs` with any protocol (its plugin only supplies the
`App` type) and just `run()`; see [`examples/jobs`](../../examples/jobs).

## Internal role

Depends on `upwell-app` (the `Plugin` seam, `Startup` hook), `upwell-di` (the
`RootResolver` each job resolves through), `upwell-hooks`, and `upwell-core`. The
`upwell` facade re-exports it as `upwell::jobs`.

## Feature flags

| Feature | Effect |
|---|---|
| `di-check` | forward the compile-time DI assertions (a `#[jobs]` block may carry `#[init]`) |
| `facade` | root the macros' generated types at `::upwell::jobs::*` (set by the `upwell` facade); off = the standalone `::upwell_jobs::*` |
