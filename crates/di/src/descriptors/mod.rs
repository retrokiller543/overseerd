//! Component and provider descriptors, and the link-time slices that collect them.

pub mod component;

pub use component::{
    BoxedComponent, Component, ComponentConstructionContext, ComponentDescriptor,
    ComponentFactories, ComponentFactory, ComponentFactoryDescriptor, Dep, Dynamic, Injectable,
    Live, LiveRef, Provide, ProviderDescriptor, ServiceComponent, Wired, Wiring,
};

pub use overseerd_core::{Cardinality, ComponentScope, DependencyDescriptor, TypeDescriptor};

/// Link-time registry of every discovered [`ComponentDescriptor`].
///
/// Proc macros register one element each via `#[linkme::distributed_slice(COMPONENTS)]`:
/// a `#[component]`/`#[service]` factory descriptor. The component registry reads the
/// assembled slice; each slice is homogeneous and assembled at link time with no
/// per-startup registration walk.
#[linkme::distributed_slice]
pub static COMPONENTS: [ComponentDescriptor];

/// Link-time registry of every discovered [`ProviderDescriptor`] (a component declaring
/// `provide = dyn Trait`). See [`COMPONENTS`].
#[linkme::distributed_slice]
pub static PROVIDERS: [ProviderDescriptor];
