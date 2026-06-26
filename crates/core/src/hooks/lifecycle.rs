//! Built-in process-lifecycle hook kinds: [`Startup`] and [`Shutdown`].
//!
//! Both run on singleton components (their `&self` resolves on the root container) and are
//! fired by the daemon's `serve`/`run`. They fill a gap no other extension point covers —
//! running code *after the daemon is built but before it serves*, and *on graceful stop* —
//! unlike per-component setup (`#[init]`) or request cross-cutting (middleware/guards). Take
//! their dependencies through `&self`; neither carries inputs.

use super::HookKind;

/// Fired once after the daemon is built and before it begins serving. An `Err` from any
/// `#[hook(Startup)]` aborts startup — the daemon does not begin accepting work.
pub struct Startup;

impl HookKind for Startup {
    const NAME: &'static str = "startup";
    type Output = ();
    type Cx = ();
}

/// Fired once when the daemon begins a graceful shutdown (the serve/run loop has stopped).
/// Errors from a `#[hook(Shutdown)]` are logged; shutdown proceeds regardless.
pub struct Shutdown;

impl HookKind for Shutdown {
    const NAME: &'static str = "shutdown";
    type Output = ();
    type Cx = ();
}
