//! A general lifecycle/event **hook** system.
//!
//! A hook is an `async` method on a component or service, marked `#[hook(Kind)]`, that the
//! framework calls when an event of `Kind` occurs. `Kind` is a [`HookKind`] type that owns
//! the hook's *output* (what the method returns) and its *context* (the typed inputs an
//! invocation carries). Built-in kinds are [`Startup`] and [`Shutdown`]; config reload is a
//! kind defined in `overseerd-config`. The system is deliberately general so user-defined
//! event kinds can be added the same way.
//!
//! Hooks do **not** receive component dependencies as parameters — those are reached through
//! `&self` (a hook may also be self-less). A hook's only parameters are the kind's inputs,
//! each a [`HookParam`] extracted from the kind's context. Hooks are collected per type
//! (the `{Type}Hooks` distributed slice, exposed via [`ComponentHooks`]) and registered into
//! a [`HookManager`]; a type with no hooks contributes nothing at runtime.
//!
//! This crate is generic over the [`ResolverCtx`](overseerd_core::ResolverCtx): a hook's
//! erased [`HookCall`] resolves its receiver through the resolver context, so the hook
//! layer never names the DI container (which sits above it).

mod error;
mod lifecycle;

pub use error::{Error, Result};
pub use lifecycle::{Shutdown, Startup};

use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, OnceLock};

use futures::FutureExt;
use overseerd_core::{DependencyDescriptor, OverseerdDescriptor, ResolverCtx, TypeDescriptor};

/// A kind of hook: the event a `#[hook(Kind)]` method reacts to.
///
/// The kind owns the contract: `Output` is what each hook of this kind returns (the kind
/// "decides what output it needs"), and `Cx` is the owned, per-invocation context its
/// parameters are extracted from (e.g. the proposed config values for a reload).
pub trait HookKind: 'static {
    /// A stable name for diagnostics and indexing.
    const NAME: &'static str;

    /// What each hook of this kind returns and the runner collects.
    type Output: Send + 'static;

    /// The owned context one invocation carries, that this kind's [`HookParam`]s read.
    /// `Send + Sync` so the hook future (which borrows it) stays `Send`.
    type Cx: Send + Sync + 'static;
}

/// A parameter a `#[hook(K)]` method may take: an input of kind `K`, extracted from the
/// kind's context — never a component dependency (those come through `&self`).
pub trait HookParam<K: HookKind>: Sized {
    /// The dependency edge this parameter contributes, for validation and event routing
    /// (e.g. the config path a reload hook targets). `path` is the param's `#[config("..")]`
    /// literal, or `None` for the by-type shorthand.
    fn dependency(path: Option<&'static str>) -> DependencyDescriptor;

    /// Extracts this parameter from the kind's context.
    fn extract(cx: &K::Cx, path: Option<&'static str>) -> Result<Self>;
}

/// The erased call shape every hook is compiled to: resolve the receiver (if the method
/// takes `&self`) from the resolver context, extract the kind's params from the erased
/// context, run the method, and box its output. The boxed value is always the kind's
/// `Output`.
///
/// The call takes `&dyn ResolverCtx` (not the container itself), so the hook layer stays
/// below the DI engine. The macro-generated body resolves the receiver through the
/// component source it fetches from the context.
pub type HookCall =
    for<'a> fn(
        &'a (dyn ResolverCtx + Send + Sync),
        &'a (dyn Any + Send + Sync),
    ) -> Pin<Box<dyn Future<Output = Result<Box<dyn Any + Send>>> + Send + 'a>>;

/// Static metadata for one hook, registered into its type's `{Type}Hooks` slice.
#[derive(Clone, Copy)]
pub struct HookDescriptor {
    /// The component/service the hook is defined on.
    pub component_ty: TypeDescriptor,
    /// The hook kind's [`NAME`](HookKind::NAME), for diagnostics.
    pub kind: &'static str,
    /// `TypeId::of::<Kind>()`, used to select hooks of a given kind at runtime.
    pub kind_ty: fn() -> TypeId,
    /// The hook's parameter edges (kind inputs), reported at runtime for validation and
    /// event routing.
    pub dependencies: fn() -> Vec<DependencyDescriptor>,
    /// The erased call.
    pub call: HookCall,
}

impl OverseerdDescriptor for HookDescriptor {}

impl fmt::Debug for HookDescriptor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HookDescriptor")
            .field("component_ty", &self.component_ty)
            .field("kind", &self.kind)
            .field("dependencies", &(self.dependencies)())
            .finish_non_exhaustive()
    }
}

/// A component type's own hooks.
///
/// Implemented for each `#[component]`/`#[service]` by the macro to return that type's
/// `{Type}Hooks` distributed slice — the slice every `#[hook]` method appends to. The owning
/// `ComponentDescriptor` stores this as a fn pointer, so the registry reaches a type's hooks
/// without holding its type.
pub trait ComponentHooks {
    /// Every hook contributed to this type.
    fn hooks() -> &'static [HookDescriptor];
}

/// The empty hooks slice for a type that declares none — the default carried by a
/// `ComponentDescriptor`.
pub fn no_hooks() -> &'static [HookDescriptor] {
    &[]
}

/// Runs hooks of a kind against the live component instances.
///
/// Built once at application build from every registered component's hook slice (empties
/// skipped), so it can be seeded as a framework singleton. The resolver context is attached
/// after the root scope is built (see [`attach`](Self::attach)).
#[derive(Clone)]
pub struct HookManager {
    inner: Arc<HookManagerInner>,
}

struct HookManagerInner {
    ctx: OnceLock<Arc<dyn ResolverCtx + Send + Sync>>,
    /// Hooks indexed by kind `TypeId`, so a kind with no listeners is an O(1) miss and a
    /// fire over it does no work at all.
    by_kind: HashMap<TypeId, Vec<HookDescriptor>>,
}

impl HookManager {
    /// Builds a manager over every registered hook (across all component types), grouped by
    /// kind for O(1) listener lookup.
    pub fn new(hooks: Vec<HookDescriptor>) -> Self {
        let mut by_kind: HashMap<TypeId, Vec<HookDescriptor>> = HashMap::new();

        for hook in hooks {
            by_kind.entry((hook.kind_ty)()).or_default().push(hook);
        }

        Self {
            inner: Arc::new(HookManagerInner {
                ctx: OnceLock::new(),
                by_kind,
            }),
        }
    }

    /// Attaches the resolver context, once it exists. Hooks resolve their `&self` receiver
    /// through it. Idempotent; a second attach is ignored.
    pub fn attach(&self, ctx: Arc<dyn ResolverCtx + Send + Sync>) {
        let _ = self.inner.ctx.set(ctx);
    }

    /// Whether any hook of kind `K` is registered — an O(1) check a firing site uses to
    /// skip building the event entirely when nothing listens.
    pub fn has<K: HookKind>(&self) -> bool {
        self.inner.by_kind.contains_key(&TypeId::of::<K>())
    }

    /// Runs every hook of kind `K` for which `filter` returns true, against `cx`,
    /// **concurrently**, and collects each component's typed outcome (or error) in
    /// registration order. Returns an empty `Vec` (no work) when nothing listens.
    pub async fn run<K: HookKind>(
        &self,
        cx: &K::Cx,
        filter: impl Fn(&HookDescriptor) -> bool,
    ) -> Vec<(TypeDescriptor, Result<K::Output>)> {
        let Some(bucket) = self.inner.by_kind.get(&TypeId::of::<K>()) else {
            return Vec::new();
        };

        let ctx = self
            .inner
            .ctx
            .get()
            .expect("hook manager resolver context attached before hooks run");

        let calls = bucket.iter().filter(|hook| filter(hook)).map(|hook| {
            let component = hook.component_ty;

            async move {
                let outcome = invoke_hook::<K>(hook, ctx.as_ref(), cx).await;

                (component, outcome)
            }
        });

        futures::future::join_all(calls).await
    }

    /// Runs matching hooks sequentially in registration order and stops after the
    /// first failure. Lifecycle startup uses this to guarantee that components after
    /// a failed startup hook never begin side effects.
    pub async fn run_until_error<K: HookKind>(
        &self,
        cx: &K::Cx,
        filter: impl Fn(&HookDescriptor) -> bool,
    ) -> Vec<(TypeDescriptor, Result<K::Output>)> {
        let Some(bucket) = self.inner.by_kind.get(&TypeId::of::<K>()) else {
            return Vec::new();
        };

        let ctx = self
            .inner
            .ctx
            .get()
            .expect("hook manager resolver context attached before hooks run");
        let mut outcomes = Vec::new();

        for hook in bucket.iter().filter(|hook| filter(hook)) {
            let component = hook.component_ty;
            let outcome = invoke_hook::<K>(hook, ctx.as_ref(), cx).await;
            let failed = outcome.is_err();

            outcomes.push((component, outcome));

            if failed {
                break;
            }
        }

        outcomes
    }

    /// Whether `component` declares a hook of kind `K`.
    pub fn component_has<K: HookKind>(&self, component: TypeId) -> bool {
        self.inner
            .by_kind
            .get(&TypeId::of::<K>())
            .is_some_and(|hooks| {
                hooks
                    .iter()
                    .any(|hook| hook.component_ty.type_id == component)
            })
    }
}

async fn invoke_hook<K: HookKind>(
    hook: &HookDescriptor,
    ctx: &(dyn ResolverCtx + Send + Sync),
    cx: &K::Cx,
) -> Result<K::Output> {
    let component = hook.component_ty;
    let future = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        (hook.call)(ctx, cx as &(dyn Any + Send + Sync))
    }))
    .map_err(|_| Error::Panicked {
        hook: K::NAME,
        component: component.name,
    })?;

    std::panic::AssertUnwindSafe(future)
        .catch_unwind()
        .await
        .map_err(|_| Error::Panicked {
            hook: K::NAME,
            component: component.name,
        })?
        .and_then(|boxed| {
            boxed
                .downcast::<K::Output>()
                .map(|boxed| *boxed)
                .map_err(|_| Error::InvalidOutput {
                    hook: K::NAME,
                    component: component.name,
                })
        })
}

/// The stable component id of the seeded [`HookManager`] singleton.
pub const HOOK_MANAGER_ID: &str = "overseerd:hook-manager";

/// The display name of the seeded [`HookManager`] singleton.
pub const HOOK_MANAGER_NAME: &str = "HookManager";

#[cfg(test)]
mod tests;
