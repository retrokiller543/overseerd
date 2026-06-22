//! Several components providing one trait, to demonstrate trait-object injection:
//! a primary, plus qualifier-keyed variants. `Send + Sync` is a supertrait, so
//! the bare `dyn Notifier` is shareable and no use site writes `+ Send + Sync`.

use overseer::component;

/// A channel a notification can be delivered over.
pub trait Notifier: Send + Sync {
    fn channel(&self) -> &'static str;
}

/// The default channel: `primary` wins a single `Arc<dyn Notifier>`. Its
/// qualifier is inferred as the component id, `"email"`.
#[component(provide = dyn Notifier, primary)]
pub struct Email;

impl Notifier for Email {
    fn channel(&self) -> &'static str {
        "email"
    }
}

/// An explicit qualifier overrides the inferred id.
#[component(provide = dyn Notifier, qualifier = "sms")]
pub struct Sms;

impl Notifier for Sms {
    fn channel(&self) -> &'static str {
        "sms"
    }
}

#[component(provide = dyn Notifier, qualifier = "push")]
pub struct Push;

impl Notifier for Push {
    fn channel(&self) -> &'static str {
        "push"
    }
}
