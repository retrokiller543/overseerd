//! The protocol-facing runtime handle.
//!
//! [`AppRuntime`] is the cheap-clone handle a [`Protocol`](crate::protocol::Protocol)
//! receives to drive requests through the DI container and reach the app's support
//! systems. It owns the *agnostic* runtime state — the built scope containers, the
//! per-scope construction orders, and the hook manager — that the serve loop used to
//! take as a long argument list, and exposes the scope-opening primitives a protocol
//! drives per connection and per request.

use std::sync::Arc;

use overseerd_di::{BoxedComponent, ComponentDescriptor, ScopeContainer, ScopeRegistry};
use overseerd_hooks::HookManager;

use crate::scope::{Connection as ConnectionScope, Request as RequestScope};

/// Everything a protocol needs to drive requests through DI, cheaply cloneable.
#[derive(Clone)]
pub struct AppRuntime {
    name: Arc<str>,
    root: Arc<ScopeContainer>,
    scopes: Arc<ScopeRegistry>,
    connection_order: Arc<Vec<ComponentDescriptor>>,
    request_order: Arc<Vec<ComponentDescriptor>>,
    needs_peer: bool,
    hooks: HookManager,
}

impl AppRuntime {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        name: Arc<str>,
        root: Arc<ScopeContainer>,
        scopes: Arc<ScopeRegistry>,
        connection_order: Arc<Vec<ComponentDescriptor>>,
        request_order: Arc<Vec<ComponentDescriptor>>,
        needs_peer: bool,
        hooks: HookManager,
    ) -> Self {
        Self {
            name,
            root,
            scopes,
            connection_order,
            request_order,
            needs_peer,
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

    /// Whether any component depends on the framework-seeded `PeerInfo`. When false a
    /// connection scope that would hold only the peer can be skipped.
    pub fn needs_peer(&self) -> bool {
        self.needs_peer
    }

    /// Opens a connection-scoped child over the root, seeding `seeds` (e.g. the peer)
    /// and constructing the connection-scope order. An empty scope is skipped, so this
    /// returns the root unchanged when nothing connection-scoped exists.
    pub async fn open_connection_scope(
        &self,
        seeds: Vec<BoxedComponent>,
    ) -> crate::Result<Arc<ScopeContainer>> {
        ScopeContainer::open_child(
            &ConnectionScope,
            Arc::clone(&self.root),
            Arc::clone(&self.scopes),
            &self.connection_order,
            seeds,
        )
        .await
        .map_err(crate::Error::from)
    }

    /// Opens a request-scoped child over `parent` (a connection scope, or the root),
    /// constructing the request-scope order.
    pub async fn open_request_scope(
        &self,
        parent: Arc<ScopeContainer>,
        seeds: Vec<BoxedComponent>,
    ) -> crate::Result<Arc<ScopeContainer>> {
        ScopeContainer::open_child(
            &RequestScope,
            parent,
            Arc::clone(&self.scopes),
            &self.request_order,
            seeds,
        )
        .await
        .map_err(crate::Error::from)
    }
}
