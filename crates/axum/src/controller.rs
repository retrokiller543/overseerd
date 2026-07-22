//! Controllers: the HTTP-facing services, and how they register their routes.
//!
//! A controller is a DI singleton (like an RPC service) annotated with `#[controller]`. Its
//! `#[handlers]` impl blocks contribute routes. The macro emits a [`ControllerDescriptor`]
//! into the [`CONTROLLERS`] slice and an implementation of [`Controller`]; the
//! [`AxumPlugin`](crate::AxumPlugin) folds the slice on `auto_discover` and merges each
//! controller's [`axum::Router`] when the protocol is built.

use std::sync::Arc;

use overseerd_app::AppRuntime;
use overseerd_core::{OverseerdDescriptor, TypeDescriptor};

/// One `#[handlers]` block's route-group builder, tagged with its controller type `C`.
///
/// The route registries hold bare `fn` pointers, which cannot implement [`OverseerdDescriptor`]
/// (a primitive fn-pointer type is foreign and carries no local type, so the marker impl would
/// violate the orphan rule). This local newtype is the HTTP analog of the RPC `RpcGroup`: it wraps
/// the builder so it *can* be an `OverseerdDescriptor` and thus a `DescriptorFor<C, ControllerRoute<C>>`
/// bucket element on the `inventory` backend. `Copy` is manual (a naive derive would wrongly demand
/// `C: Copy`); the wrapped fn pointer is always `Copy`.
pub struct ControllerRoute<C>(pub fn(Arc<C>, &AppRuntime) -> axum::Router);

impl<C> Clone for ControllerRoute<C> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<C> Copy for ControllerRoute<C> {}

impl<C: 'static> OverseerdDescriptor for ControllerRoute<C> {}

/// A controller's link-time registration: its identity and a builder for its routes.
///
/// The `router` builder is handed the assembled [`AppRuntime`] so it can resolve the
/// controller singleton once (capturing it in the route closures) before returning the
/// fully-pathed [`axum::Router`]. It is a plain `fn` pointer so it can live in a
/// `#[distributed_slice]`.
#[derive(Clone, Copy)]
pub struct ControllerDescriptor {
    /// The controller's id (defaults to the lowercased type name).
    pub id: &'static str,

    /// The controller's display name (the type name).
    pub name: &'static str,

    /// The controller's concrete type.
    pub ty: TypeDescriptor,

    /// The base path every route in this controller is mounted under.
    pub base: &'static str,

    /// Builds this controller's routes, with full paths already joined onto [`base`](Self::base).
    pub router: fn(&AppRuntime) -> axum::Router,
}

/// The link-time slice every `#[controller]` registers into, mirroring the RPC `SERVICES`
/// slice. [`AxumPlugin::auto_discover`](crate::AxumPlugin) folds it into the builder.
#[linkme::distributed_slice]
pub static CONTROLLERS: [ControllerDescriptor];

/// Implemented by every `#[controller]` struct: the base path and a builder for its routes.
///
/// Generated alongside the [`ControllerDescriptor`]; both point at the same `router`
/// builder, so registering a controller by type and discovering it from the slice are
/// equivalent.
pub trait Controller {
    /// The base path this controller's routes are mounted under (e.g. `"/users"`).
    const BASE: &'static str;

    /// Builds this controller's [`axum::Router`], resolving the controller singleton from
    /// the runtime and capturing it in the route handlers.
    fn router(runtime: &AppRuntime) -> axum::Router;
}
