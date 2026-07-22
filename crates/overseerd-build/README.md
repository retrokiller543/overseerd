# overseerd-build

Build-script helper for the Overseerd registration-backend seam.

## Role

The framework registers per-component metadata (factories, hooks, RPC groups, routes) into
link-time distributed slices. On Apple/Mach-O targets, custom linker sections are capped at ~255 per
segment, so the macros switch from `linkme` (one custom section per per-type slice) to `inventory`
(one shared constructor section) — selected by the `overseerd_hybrid` cfg.

This crate exposes a single `configure()` function that a framework crate calls from its `build.rs`
to register the cfg (`rustc-check-cfg`) and activate it on Mach-O targets, or when forced with
`OVERSEERD_HYBRID=1`.

## Usage

**Internal only** — a build-dependency of the framework crates that emit or observe backend-specific
code (`overseerd-macros-core`, `overseerd-di`, `overseerd-rpc`, `overseerd-axum`). It is not meant
for downstream user crates: user code never references `overseerd_hybrid`, because the proc-macros
resolve the backend at expansion time and emit only the chosen backend's tokens.

```rust
// build.rs
fn main() {
    overseerd_build::configure();
}
```

To force the `inventory` backend on a non-Mach-O target (e.g. testing on Linux, or cross-compiling
to macOS from Linux), set `OVERSEERD_HYBRID=1`, or enable the `hybrid-registry` feature on the
`overseerd` facade (which the macro crate reads directly).
