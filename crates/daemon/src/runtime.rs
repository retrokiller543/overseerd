//! The protocol-facing runtime handle.
//!
//! [`AppRuntime`] is the cheap-clone handle a [`Protocol`](crate::protocol::Protocol)
//! receives to drive requests through the DI container and reach the app's support
//! systems. It owns the *agnostic* runtime state — the built scope containers, the
//! per-scope construction orders, and the hook manager — that the serve loop used to
//! take as a long argument list, and exposes the scope-opening primitives a protocol
//! drives per connection and per request.

use std::collections::HashMap;
use std::sync::Arc;

use overseerd_core::Scope;
use overseerd_di::{BoxedComponent, ComponentDescriptor, ScopeContainer, ScopeRegistry};
use overseerd_hooks::HookManager;

/// Everything a protocol needs to drive requests through DI, cheaply cloneable.
///
/// Agnostic to any particular protocol: it holds the built root scope, the per-scope
/// construction orders keyed by scope name (computed from the protocol's declared
/// scope chain), the resolved component set, and the hook manager. A protocol opens
/// its scopes through [`open_scope`](Self::open_scope), naming each scope from its own
/// chain.
#[derive(Clone)]
pub struct AppRuntime {
    name: Arc<str>,
    root: Arc<ScopeContainer>,
    scopes: Arc<ScopeRegistry>,
    orders: Arc<HashMap<&'static str, Vec<ComponentDescriptor>>>,
    resolved: Arc<[ComponentDescriptor]>,
    hooks: HookManager,
}

impl AppRuntime {
    pub(crate) fn new(
        name: Arc<str>,
        root: Arc<ScopeContainer>,
        scopes: Arc<ScopeRegistry>,
        orders: Arc<HashMap<&'static str, Vec<ComponentDescriptor>>>,
        resolved: Arc<[ComponentDescriptor]>,
        hooks: HookManager,
    ) -> Self {
        Self {
            name,
            root,
            scopes,
            orders,
            resolved,
            hooks,
        }
    }

    /// The application name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The root (singleton) scope container.
    pub fn root(&self) -> &Arc<ScopeContainer> {
        &self.root
    }

    /// The hook manager, for running lifecycle/event hooks by kind.
    pub fn hooks(&self) -> &HookManager {
        &self.hooks
    }

    /// The resolved component set (the effective per-type descriptors). A protocol may
    /// introspect it — the RPC protocol uses it to decide whether the peer is depended
    /// on and therefore worth seeding.
    pub fn resolved_components(&self) -> &[ComponentDescriptor] {
        &self.resolved
    }

    /// Opens a child container for `scope` over `parent`, seeding `seeds` and
    /// constructing that scope's precomputed order (empty when the scope holds nothing
    /// constructable). An empty, unseeded scope is skipped — `parent` is returned
    /// unchanged. `scope` must be one the app was built for (in the protocol's chain).
    pub async fn open_scope(
        &self,
        scope: &'static dyn Scope,
        parent: Arc<ScopeContainer>,
        seeds: Vec<BoxedComponent>,
    ) -> crate::Result<Arc<ScopeContainer>> {
        const EMPTY: &[ComponentDescriptor] = &[];

        let order = self
            .orders
            .get(scope.name())
            .map_or(EMPTY, |order| order.as_slice());

        ScopeContainer::open_child(scope, parent, Arc::clone(&self.scopes), order, seeds)
            .await
            .map_err(crate::Error::from)
    }
}
