use std::sync::Once;

static INSTALL_SANITIZED_HOOK: Once = Once::new();

/// Installs the dedicated-process tooling panic hook exactly once.
///
/// Panic hooks are process-global. This function is only for the exact hidden probe process path:
/// it installs a payload-free hook before target identity parsing or application work and never
/// restores the prior hook because the probe exits immediately. Direct/embedded callers must not
/// call this function; they retain their process hook and use the unwind-to-envelope boundary only.
#[doc(hidden)]
pub fn install_process_probe_panic_hook() {
    INSTALL_SANITIZED_HOOK.call_once(|| {
        std::panic::set_hook(Box::new(|_| {
            eprintln!("overseerd tooling probe panicked; panic payload suppressed");
        }));
    });
}
