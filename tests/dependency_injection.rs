//! End-to-end tests for the Phase 0 dependency model: `#[default]` local state,
//! optional dependencies (`Option<Arc<T>>`), and runtime-provided dependencies
//! (`Dynamic<Arc<T>>`). Each builds a real daemon and inspects the constructed
//! component out of the container.

use std::sync::Arc;

use overseerd::{App, Dynamic, component};

/// A plain dependency, provided as an instance at build time (manual — no factory).
#[component(default_factory = false)]
struct Settings {
    name: String,
}

/// A target that is never registered — used to prove an absent `Option` edge
/// resolves to `None` rather than failing the build.
struct Absent;

/// Exercises every Phase 0 edge shape in one component:
/// - `counter` is local state built via `Default` (not injected);
/// - `present` is an optional dependency that *is* provided;
/// - `missing` is an optional dependency that is *not* provided;
/// - `settings` is a runtime-provided (`Dynamic`) dependency.
#[component]
struct Widget {
    #[default]
    counter: u32,
    present: Option<Arc<Settings>>,
    missing: Option<Arc<Absent>>,
    settings: Dynamic<Arc<Settings>>,
}

#[tokio::test]
async fn resolves_default_optional_and_dynamic_edges() {
    let daemon = App::builder("di-test")
        .auto_discover()
        .with_component(Settings {
            name: "configured".to_string(),
        })
        .build()
        .await
        .expect("daemon builds");

    let widget = daemon
        .container()
        .get::<Widget>()
        .expect("Widget constructed");

    assert_eq!(widget.counter, 0, "#[default] field uses Default");
    assert!(widget.present.is_some(), "present optional dep resolved");
    assert!(widget.missing.is_none(), "absent optional dep is None");
    assert_eq!(
        widget.settings.name, "configured",
        "Dynamic dep resolved and derefs through to the value"
    );
}
