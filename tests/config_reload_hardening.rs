//! Regression coverage for reload invalidation and panic recovery.
#![allow(dead_code)]

use std::borrow::Cow;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, RwLock};

use overseerd::config::Toml;
use overseerd::daemon::App;
use overseerd::{
    Cfg, CfgNext, ConfigManager, ConfigProperties, ConfigReload, ConfigReloadError, HookOutcome,
    component, config, methods,
};
use overseerd_config::Resolver;
use serde::{Deserialize, Deserializer};

static NEXT_DIR: AtomicUsize = AtomicUsize::new(0);

fn temp_config(tag: &str, contents: &str) -> (PathBuf, PathBuf) {
    let root = std::env::temp_dir().join(format!(
        "overseerd-reload-hardening-{tag}-{}-{}",
        std::process::id(),
        NEXT_DIR.fetch_add(1, Ordering::Relaxed)
    ));
    let config = root.join("application.toml");

    fs::create_dir_all(&root).expect("create config directory");
    fs::write(&config, contents).expect("write config");

    (root, config)
}

#[config]
#[derive(Deserialize)]
struct ReferencingConfig {
    url: String,
}

#[component]
struct ReferencingConsumer {
    #[config("server")]
    config: Cfg<ReferencingConfig>,
}

#[tokio::test]
async fn cross_path_reference_changes_republish_the_dependent_binding() {
    let (root, file) = temp_config(
        "cross-path",
        "[defaults]\nhost = \"10.0.0.1\"\n[server]\nurl = \"${defaults.host}:8080\"\n",
    );
    let manager = ConfigManager::<Toml>::load_in(&root, &[]).expect("load config");
    let app = App::builder("cross-path-reload")
        .config_source(manager)
        .config::<ReferencingConfig>("server")
        .component::<ReferencingConsumer>()
        .build()
        .await
        .expect("build app");
    let consumer = app
        .container()
        .get::<ReferencingConsumer>()
        .expect("resolve consumer");

    assert_eq!(consumer.config.get().url, "10.0.0.1:8080");
    fs::write(
        &file,
        "[defaults]\nhost = \"10.0.0.2\"\n[server]\nurl = \"${defaults.host}:8080\"\n",
    )
    .expect("update referenced path");

    let report = app.config_reloader().reload().await.expect("reload");

    assert_eq!(report.changed.len(), 1);
    assert_eq!(report.changed[0].path, "server");
    assert_eq!(consumer.config.get().url, "10.0.0.2:8080");

    let _ = fs::remove_dir_all(root);
}

struct MutableResolver(Arc<RwLock<String>>);

impl Resolver for MutableResolver {
    fn resolve(&self, key: &str) -> Option<Cow<'_, str>> {
        (key == "mutable_value").then(|| Cow::Owned(self.0.read().expect("resolver lock").clone()))
    }
}

#[config]
#[derive(Deserialize)]
struct ResolverConfig {
    value: String,
}

#[component]
struct ResolverConsumer {
    #[config("resolved")]
    config: Cfg<ResolverConfig>,
}

#[tokio::test]
async fn resolver_changes_republish_a_binding_without_source_edits() {
    let (root, _) = temp_config("resolver", "[resolved]\nvalue = \"${mutable_value}\"\n");
    let value = Arc::new(RwLock::new("first".to_string()));
    let manager = ConfigManager::<Toml>::load_in(&root, &[])
        .expect("load config")
        .with_resolver(Box::new(MutableResolver(Arc::clone(&value))));
    let app = App::builder("resolver-reload")
        .config_source(manager)
        .config::<ResolverConfig>("resolved")
        .component::<ResolverConsumer>()
        .build()
        .await
        .expect("build app");
    let consumer = app
        .container()
        .get::<ResolverConsumer>()
        .expect("resolve consumer");

    assert_eq!(consumer.config.get().value, "first");
    *value.write().expect("resolver lock") = "second".to_string();

    let report = app.config_reloader().reload().await.expect("reload");

    assert_eq!(report.changed.len(), 1);
    assert_eq!(consumer.config.get().value, "second");

    let _ = fs::remove_dir_all(root);
}

struct AdvancingResolver(Arc<AtomicUsize>);

impl Resolver for AdvancingResolver {
    fn resolve(&self, key: &str) -> Option<Cow<'_, str>> {
        if key != "advancing_value" {
            return None;
        }

        let call = self.0.fetch_add(1, Ordering::SeqCst);
        let value = ["first", "second", "third"]
            .get(call)
            .copied()
            .unwrap_or("third");

        Some(Cow::Borrowed(value))
    }
}

#[tokio::test]
async fn committed_snapshot_comes_from_the_same_resolver_pass_as_the_value() {
    let (root, _) = temp_config("exact-pass", "[resolved]\nvalue = \"${advancing_value}\"\n");
    let calls = Arc::new(AtomicUsize::new(0));
    let manager = ConfigManager::<Toml>::load_in(&root, &[])
        .expect("load config")
        .with_resolver(Box::new(AdvancingResolver(Arc::clone(&calls))));
    let app = App::builder("exact-pass-reload")
        .config_source(manager)
        .config::<ResolverConfig>("resolved")
        .component::<ResolverConsumer>()
        .build()
        .await
        .expect("build app");
    let consumer = app
        .container()
        .get::<ResolverConsumer>()
        .expect("resolve consumer");

    assert_eq!(consumer.config.get().value, "first");

    let first = app.config_reloader().reload().await.expect("first reload");

    assert_eq!(first.changed.len(), 1);
    assert_eq!(consumer.config.get().value, "third");

    let second = app.config_reloader().reload().await.expect("second reload");

    assert!(
        second.changed.is_empty(),
        "the stable third resolver value matches the exact committed pass"
    );
    assert_eq!(consumer.config.get().value, "third");
    assert_eq!(calls.load(Ordering::SeqCst), 4);

    let _ = fs::remove_dir_all(root);
}

struct PanicConfig {
    value: u32,
}

impl<'de> Deserialize<'de> for PanicConfig {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct Raw {
            value: u32,
        }

        let raw = Raw::deserialize(deserializer)?;

        assert_ne!(raw.value, 2, "sensitive user panic payload");

        Ok(Self { value: raw.value })
    }
}

impl ConfigProperties for PanicConfig {
    const NAME: &'static str = "PanicConfig";
}

#[component]
struct PanicConsumer {
    #[config("panic")]
    config: Cfg<PanicConfig>,
}

#[tokio::test]
async fn panicking_deserializer_does_not_poison_future_reloads() {
    let (root, file) = temp_config("panic", "[panic]\nvalue = 1\n");
    let manager = ConfigManager::<Toml>::load_in(&root, &[]).expect("load config");
    let app = App::builder("panic-reload")
        .config_source(manager)
        .config::<PanicConfig>("panic")
        .component::<PanicConsumer>()
        .build()
        .await
        .expect("build app");
    let consumer = app
        .container()
        .get::<PanicConsumer>()
        .expect("resolve consumer");

    fs::write(&file, "[panic]\nvalue = 2\n").expect("write panicking value");
    let error = app.config_reloader().reload().await.unwrap_err();

    assert!(matches!(error, ConfigReloadError::Panicked));
    assert!(!error.to_string().contains("sensitive"));
    assert_eq!(
        consumer.config.get().value,
        1,
        "failed reload did not commit"
    );

    fs::write(&file, "[panic]\nvalue = 3\n").expect("write valid value");
    let report = app
        .config_reloader()
        .reload()
        .await
        .expect("later reload recovers");

    assert_eq!(report.changed.len(), 1);
    assert_eq!(consumer.config.get().value, 3);

    let _ = fs::remove_dir_all(root);
}

#[config]
#[derive(Deserialize)]
struct HookPanicConfig {
    value: u32,
}

#[component]
struct PanicOnceHook {
    #[config("hooked")]
    config: Cfg<HookPanicConfig>,
    #[default]
    calls: AtomicUsize,
}

#[methods]
impl PanicOnceHook {
    #[hook(ConfigReload)]
    async fn on_reload(
        &self,
        #[config("hooked")] _next: CfgNext<HookPanicConfig>,
    ) -> overseerd::daemon::Result<HookOutcome> {
        if self.calls.fetch_add(1, Ordering::SeqCst) == 0 {
            panic!("sensitive hook panic payload");
        }

        Ok(HookOutcome::Reloaded)
    }
}

#[tokio::test]
async fn panicking_reload_hook_does_not_disable_later_reloads() {
    let (root, file) = temp_config("hook-panic", "[hooked]\nvalue = 1\n");
    let manager = ConfigManager::<Toml>::load_in(&root, &[]).expect("load config");
    let app = App::builder("hook-panic-reload")
        .config_source(manager)
        .config::<HookPanicConfig>("hooked")
        .component::<PanicOnceHook>()
        .build()
        .await
        .expect("build app");
    let component = app
        .container()
        .get::<PanicOnceHook>()
        .expect("resolve component");

    fs::write(&file, "[hooked]\nvalue = 2\n").expect("write first update");
    let error = app.config_reloader().reload().await.unwrap_err();

    assert!(matches!(error, ConfigReloadError::Hook { .. }));
    assert!(!error.to_string().contains("sensitive"));
    assert_eq!(
        component.config.get().value,
        1,
        "panicking hook aborted commit"
    );

    fs::write(&file, "[hooked]\nvalue = 3\n").expect("write second update");
    app.config_reloader()
        .reload()
        .await
        .expect("reload subsystem remains usable");

    assert_eq!(component.config.get().value, 3);
    assert_eq!(component.calls.load(Ordering::SeqCst), 2);

    let _ = fs::remove_dir_all(root);
}
