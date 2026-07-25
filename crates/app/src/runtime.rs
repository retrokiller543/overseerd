//! The protocol-facing runtime handle.
//!
//! [`AppRuntime`] is the cheap-clone handle a [`Protocol`](crate::protocol::Protocol)
//! receives to drive requests through the DI container and reach the app's support
//! systems. It owns the *agnostic* runtime state — the built scope containers, the
//! per-scope construction orders, and the hook manager — that the serve loop used to
//! take as a long argument list, and exposes the scope-opening primitives a protocol
//! drives per connection and per request.

use std::any::TypeId;
use std::collections::HashMap;
use std::sync::Arc;

use overseerd_core::{Scope, ScopeId};
use overseerd_di::{BoxedComponent, ComponentDescriptor, ScopeContainer, ScopeRegistry};
use overseerd_hooks::HookManager;

use crate::scope::{PreparedScopeTopology, ScopeParent, SeedDestination};

/// Everything a protocol needs to drive requests through DI, cheaply cloneable.
///
/// Agnostic to any particular protocol: it holds the built root scope, the per-scope
/// construction orders keyed by stable scope identity, the prepared protocol-owned
/// topology, the resolved component set, and the hook manager. A protocol opens its
/// declared boundaries through [`open_scope`](Self::open_scope).
#[derive(Clone)]
pub struct AppRuntime {
    name: Arc<str>,
    root: Arc<ScopeContainer>,
    scopes: Arc<ScopeRegistry>,
    scope_plan: Arc<RuntimeScopePlan>,
    resolved: Arc<[ComponentDescriptor]>,
    hooks: HookManager,
}

/// Prepared scope state shared by every clone of an application runtime.
#[derive(Clone)]
pub(crate) struct RuntimeScopePlan {
    topology: Arc<PreparedScopeTopology>,
    orders: Arc<HashMap<ScopeId, Vec<ComponentDescriptor>>>,
    seed_destinations: Arc<HashMap<TypeId, SeedDestination>>,
}

impl RuntimeScopePlan {
    pub(crate) fn new(
        topology: Arc<PreparedScopeTopology>,
        orders: Arc<HashMap<ScopeId, Vec<ComponentDescriptor>>>,
        seed_destinations: Arc<HashMap<TypeId, SeedDestination>>,
    ) -> Self {
        Self {
            topology,
            orders,
            seed_destinations,
        }
    }
}

impl AppRuntime {
    pub(crate) fn new(
        name: Arc<str>,
        root: Arc<ScopeContainer>,
        scopes: Arc<ScopeRegistry>,
        scope_plan: RuntimeScopePlan,
        resolved: Arc<[ComponentDescriptor]>,
        hooks: HookManager,
    ) -> Self {
        Self {
            name,
            root,
            scopes,
            scope_plan: Arc::new(scope_plan),
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

    /// The validated protocol-owned scope topology used for opening boundaries.
    pub fn scope_topology(&self) -> &PreparedScopeTopology {
        &self.scope_plan.topology
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

    /// Opens a declared child boundary over its only valid parent.
    ///
    /// The boundary's prepared scope metadata is used for construction. The caller's
    /// `scope` contributes only its stable identity. Every dynamic seed must have a
    /// factory-less descriptor registered at this exact destination and may appear once.
    pub async fn open_scope(
        &self,
        scope: &'static dyn Scope,
        parent: Arc<ScopeContainer>,
        seeds: Vec<BoxedComponent>,
    ) -> crate::Result<Arc<ScopeContainer>> {
        const EMPTY: &[ComponentDescriptor] = &[];

        let child = scope.id();
        let boundary = self
            .scope_plan
            .topology
            .boundary(&child)
            .ok_or(crate::Error::UndeclaredScopeOpen { scope: child })?;

        self.validate_parent(child, boundary.parent(), &parent)?;
        self.validate_seeds(child, &seeds)?;

        let order = self
            .scope_plan
            .orders
            .get(&child)
            .map_or(EMPTY, Vec::as_slice);

        ScopeContainer::open_child(
            boundary.scope(),
            parent,
            Arc::clone(&self.scopes),
            order,
            seeds,
        )
        .await
        .map_err(crate::Error::from)
    }

    fn validate_parent(
        &self,
        child: ScopeId,
        expected: ScopeParent,
        parent: &Arc<ScopeContainer>,
    ) -> crate::Result<()> {
        let actual = parent.scope().id();

        if !parent.belongs_to_registry(&self.scopes) {
            return Err(crate::Error::ForeignScopeParent {
                child,
                parent: actual,
            });
        }

        let valid = match expected {
            ScopeParent::Root => Arc::ptr_eq(parent, &self.root),
            ScopeParent::Boundary(expected) => actual == expected,
        };

        if !valid {
            return Err(crate::Error::InvalidScopeParent {
                child,
                expected: expected.id(),
                actual,
            });
        }

        Ok(())
    }

    fn validate_seeds(&self, scope: ScopeId, seeds: &[BoxedComponent]) -> crate::Result<()> {
        for (index, seed) in seeds.iter().enumerate() {
            let type_id = seed.ty.type_id;
            let type_name = (seed.ty.type_name)();

            if seeds[..index]
                .iter()
                .any(|candidate| candidate.ty.type_id == type_id)
            {
                return Err(crate::Error::DuplicateSeedType { scope, type_name });
            }

            let Some(destination) = self.scope_plan.seed_destinations.get(&type_id) else {
                return Err(crate::Error::UnregisteredSeed { scope, type_name });
            };

            if destination.scope != scope {
                return Err(crate::Error::InvalidSeedDestination {
                    type_name: destination.type_name,
                    expected: destination.scope,
                    actual: scope,
                });
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests;
