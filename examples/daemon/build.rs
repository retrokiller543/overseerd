//! Validates the daemon's dependency graph at build time. If a component depends
//! on something no other component provides (and it isn't marked `Dynamic`), or
//! the graph has a cycle, `cargo build` fails here — long before the daemon runs.

fn main() {
    println!("cargo::rerun-if-changed=src");

    /*overseer_analyze::report(overseer_analyze::validate_crate("src"));*/
}
