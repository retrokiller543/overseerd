# overseerd-jobs-macros

> The Overseerd job macros: `#[jobs]` (impl-block) and `#[job]` (method marker).

Part of the [Overseerd](../../README.md) framework — the macro crate for
[`overseerd-jobs`](../jobs/README.md), built on the shared
[`overseerd-macros-core`](../macros-core/README.md) codegen.

## Role

Like the RPC and axum protocol macros, the job macros emit crate-specific paths
(`::overseerd::jobs::*`), so they live in their own proc-macro crate rather than the core
`overseerd-macros`. `#[jobs]` is `MethodArgs<Jobs>` — the base impl macro (`#[methods]`:
`#[init]` + `#[hook]`) plus a `Jobs` extension (via the `ParseItem`/`ParseMethod`/`ToTokens`
seam) that claims each `#[job]` method, generates its erased call (resolving the `&self`
receiver and each parameter through the `RootResolver` on every run), and registers a
`JobDescriptor` into the `JOBS` link-time slice. This keeps `overseerd-macros-core` unaware
that jobs exist — the base only knows core concepts (`#[component]`/`#[hook]`/`#[init]`).

## Usage

You never depend on this crate directly — it is re-exported through
[`overseerd-jobs`](../jobs/README.md) (and the `overseerd` facade's `jobs` module). Use the
attributes on a component impl:

```rust
use overseerd::jobs::{jobs, JobsPlugin};

#[jobs]
impl Reaper {
    #[job(every = "30s")]
    async fn sweep(&self) { /* … */ }
}
```

`#[job]` is a marker consumed and stripped by `#[jobs]`; used on its own it emits a
`compile_error!`.

## Internal role

Built on `overseerd-macros-core` (base impl-macro state machine, `Paths`, the extension seam).
Re-exported by `overseerd-jobs`, which owns the runtime types the generated code names
(`JobDescriptor`, `JOBS`, `ScheduleKind`).

## Feature flags

| Feature | Effect |
|---|---|
| `di-check` | forward the compile-time DI assertions to `macros-core` (a `#[jobs]` block may carry `#[init]`) |
| `facade` | root generated plugin types at `::overseerd::jobs::*` (set by the `overseerd` facade); off = the standalone `::overseerd_jobs::*` |
