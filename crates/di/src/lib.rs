//! The Overseerd dependency-injection engine.
//!
//! This crate owns the runtime DI machinery: the parent-linked [`ScopeContainer`], the
//! construction-time [`Factory`]/[`FromContainer`] extractors, the component and provider
//! descriptors, and the [`ComponentRegistry`] that validates the graph. It builds on the
//! leaf vocabulary in `overseerd-core` (type descriptors, the dependency model, the
//! resolver abstraction) and on `overseerd-hooks` for the per-component hook slice each
//! [`ComponentDescriptor`] carries.
//!
//! Config is deliberately *not* here: it is an external resolver (`overseerd-config`)
//! reached through the [`ResolverCtx`](overseerd_core::ResolverCtx), so the container
//! stays unaware of it.

pub mod construct;
pub mod container;
pub mod descriptors;
pub mod error;
pub mod registry;
mod seeded;

pub use construct::{
    Factory, FactoryOutput, FromContainer, dependency_of, dispatch_factory, factory_dependencies,
    short_name,
};
pub use container::{
    ComponentContainer, ComponentSource, ScopeContainer, ScopeRegistry, topological_sort,
};
pub use descriptors::component::from_boxed;
pub use descriptors::{
    BoxedComponent, COMPONENTS, Cardinality, Component, ComponentConstructionContext,
    ComponentDescriptor, ComponentFactories, ComponentFactory, ComponentFactoryDescriptor,
    ComponentScope, Dep, DependencyDescriptor, Dynamic, Injectable, Live, LiveRef, PROVIDERS,
    Provide, ProviderDescriptor, ServiceComponent, TypeDescriptor, Wired, Wiring,
};
pub use error::{Error, Result};
pub use registry::ComponentRegistry;

/// Re-exported so macro-generated code can reach the `#[distributed_slice]` attribute
/// through a stable path.
#[doc(hidden)]
pub use linkme;
