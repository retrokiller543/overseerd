//! Leaf vocabulary for the Overseerd framework.
//!
//! This crate is the bottom of the dependency graph: it depends on nothing internal and
//! everything else depends on it. It defines the shared *language* the layers above
//! speak — type descriptors, the dependency-edge model, component scopes, the by-type
//! [`Descriptor`] seam — and the [`resolver`] abstraction through which all dependency
//! resolution flows.
//!
//! It contains no runtime, no container, no config, and no protocol code. Those live in
//! `overseerd-di`, `overseerd-config`, `overseerd-hooks`, and `overseerd-daemon`.

pub mod dependency;
pub mod descriptor;
pub mod resolver;
pub mod scope;
pub mod types;

pub use dependency::{Cardinality, DependencyDescriptor, ResolutionMode};
pub use descriptor::{
    Descriptor, DescriptorFor, OverseerdDescriptor, RegistryFor, RuntimeDescriptor,
};
pub use resolver::{Resolver, ResolverCtx, ResolverCtxExt, ResolverSet};
pub use scope::{Scope, Singleton, StaticScope, Transient};
pub use types::{TypeDescriptor, type_id_of};
