//! The framework-seeded [`ShutdownHandle`] singleton injectable.
//!
//! [`ShutdownHandle`] is created by the daemon (from its [`ShutdownSignal`]) and
//! seeded into the root scope as a by-value singleton, so any component or handler
//! can inject it to trigger graceful shutdown. The receiving half
//! ([`ShutdownSignal`]) is consumed by `serve`/`run` and is therefore not exposed
//! through DI.
//!
//! [`ShutdownSignal`]: crate::lifecycle::ShutdownSignal

use crate::lifecycle::ShutdownHandle;
use overseerd_di::{Component, Injectable};

/// The stable component id of the seeded [`ShutdownHandle`] singleton.
pub const SHUTDOWN_HANDLE_ID: &str = "overseerd:shutdown-handle";

/// The display name of the seeded [`ShutdownHandle`] singleton.
pub const SHUTDOWN_HANDLE_NAME: &str = "ShutdownHandle";

impl Component for ShutdownHandle {
    const ID: &'static str = SHUTDOWN_HANDLE_ID;
    const NAME: &'static str = SHUTDOWN_HANDLE_NAME;
    type Handle = ShutdownHandle;

    fn into_handle(self) -> Self::Handle {
        self
    }
}

impl Injectable for ShutdownHandle {
    type Target = ShutdownHandle;
    type Stored = Self;

    fn into_stored(self) -> Self {
        self
    }

    fn from_stored(stored: &Self) -> Self {
        stored.clone()
    }
}

/// Under `di-check`, the handle is framework-seeded, so it is always provided.
#[cfg(feature = "di-check")]
impl overseerd_di::Provide<ShutdownHandle> for overseerd_di::Wiring {}
