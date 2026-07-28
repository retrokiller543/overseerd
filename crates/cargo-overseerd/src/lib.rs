//! Cargo target discovery and probe execution for Overseerd developer tooling.
//!
//! This crate contains presentation-neutral orchestration shared by the `cargo overseerd`
//! subcommand and editor integrations. It does not render terminal output or assign process exit
//! codes.

mod discovery;
mod selection;

pub use discovery::{CargoExecutable, DiscoveryError, DiscoveryRequest, discover};
pub use selection::{
    BinaryCandidate, FeatureSelection, PackageCandidate, SelectedTarget, SelectionError,
    WorkspaceCatalog,
};
