//! Proof that a hook kind defined *outside* the framework works end to end: this test crate
//! declares its own `Startup` kind, a component subscribes with `#[hook(Startup)]`, and the
//! test fires it through the injectable `HookManager` — the path any external crate uses.
//! Also exercises the O(1) `has::<K>()` listener check.
#![allow(dead_code)]

use std::sync::atomic::{AtomicUsize, Ordering};

use overseerd::config::Toml;
use overseerd::{App, ConfigManager, HookKind, component, methods};

/// A user-defined lifecycle kind — no inputs, no output.
struct Startup;

impl HookKind for Startup {
    const NAME: &'static str = "startup";
    type Output = ();
    type Cx = ();
}

/// A kind nobody listens to, to prove `has` is false for it.
struct Unused;

impl HookKind for Unused {
    const NAME: &'static str = "unused";
    type Output = ();
    type Cx = ();
}

/// Subscribes to the custom `Startup` kind. The hook takes `&self` and no inputs.
#[component]
struct Boot {
    #[default]
    started: AtomicUsize,
}

impl Boot {
    fn started(&self) -> usize {
        self.started.load(Ordering::SeqCst)
    }
}

#[methods]
impl Boot {
    #[hook(Startup)]
    async fn on_start(&self) -> overseerd::Result<()> {
        self.started.fetch_add(1, Ordering::SeqCst);

        Ok(())
    }
}

#[tokio::test]
async fn external_hook_kind_fires_through_the_manager() {
    let daemon = App::builder("hook-custom-test")
        .config_source(ConfigManager::<Toml>::empty())
        .auto_discover()
        .build()
        .await
        .expect("daemon builds");

    let boot = daemon.container().get::<Boot>().expect("Boot built");
    let hooks = daemon.hook_manager();

    assert!(hooks.has::<Startup>(), "Startup has a listener");
    assert!(
        !hooks.has::<Unused>(),
        "Unused has no listeners (O(1) miss)"
    );
    assert_eq!(boot.started(), 0, "not started yet");

    let outcomes = hooks.run::<Startup>(&(), |_| true).await;

    assert_eq!(outcomes.len(), 1, "the one Startup hook ran");
    assert!(outcomes[0].1.is_ok(), "the hook succeeded");
    assert_eq!(boot.started(), 1, "the component observed the fire");

    // Firing a kind with no listeners is a no-op that does no work.
    let none = hooks.run::<Unused>(&(), |_| true).await;
    assert!(none.is_empty(), "no listeners -> empty, silently dropped");
}
