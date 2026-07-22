//! Build-script helper for Overseerd's registration-backend seam.
//!
//! The framework registers per-component metadata into distributed slices. On targets whose object
//! format is scarce in custom linker sections — Apple/Mach-O caps them at ~255 per segment — the
//! macros must emit `inventory` registrations (one shared constructor section) instead of `linkme`
//! ones (one custom section per per-type slice). That choice is gated on the `overseerd_hybrid`
//! cfg, which this helper registers and conditionally activates.
//!
//! Call [`configure`] from the `build.rs` of any framework crate that either *emits* backend-
//! specific code (the macro crate) or *observes* the registrations and may itself branch on the
//! backend. Because each crate's build script runs for that crate's own compilation, the macro
//! crate (host-compiled) sees the host target and the runtime crates (target-compiled) see the real
//! target — correct for native builds; cross-compiling to a Mach-O target needs the `OVERSEERD_HYBRID`
//! force (or the `hybrid-registry` feature the macro reads directly).

use std::env;

/// Registers the `overseerd_hybrid` cfg and decides whether to activate it.
///
/// The `OVERSEERD_HYBRID` environment variable is a **tri-state** override:
/// - unset → *auto*: activate on a linker-section-scarce object format (any Apple/Mach-O target);
/// - a falsey value (`0`, `false`, `off`, `no`, empty) → force **`linkme`** even on Mach-O — the way
///   to opt a macOS build back onto the zero-runtime backend when it stays under the ~255-section cap;
/// - any other value → force **`inventory`** (e.g. cross-compiling to macOS from a non-Apple host,
///   or stress-testing the backend on Linux).
///
/// Registration (`rustc-check-cfg`) always happens, so the cfg is warning-clean whether or not it is
/// active. The macro crate additionally ORs in its own `hybrid-registry` Cargo feature at expansion;
/// that feature only *adds* the inventory backend, so a `linkme` force via this variable holds as
/// long as the feature is left off.
pub fn configure() {
    let mach_o = env::var("CARGO_CFG_TARGET_VENDOR").as_deref() == Ok("apple");
    let enable = match env::var("OVERSEERD_HYBRID").ok().as_deref() {
        Some("0" | "false" | "off" | "no" | "") => false,
        Some(_) => true,
        None => mach_o,
    };

    println!("cargo::rustc-check-cfg=cfg(overseerd_hybrid)");
    println!("cargo::rerun-if-env-changed=OVERSEERD_HYBRID");

    if enable {
        println!("cargo::rustc-cfg=overseerd_hybrid");
    }
}
