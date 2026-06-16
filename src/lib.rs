//! Facade crate for the first Overseer prototype.
//!
//! Responsibility: this root crate is only an ergonomic public entrypoint and
//! re-export surface for prototype core concepts.
//!
//! Excluded responsibilities: this crate does not own descriptor definitions,
//! registration validation, demonstration domain logic, runtime behavior,
//! transports, procedural macros, generated SDKs, persistence, credentials,
//! networking, or framework ownership of application startup. Implementation
//! crates introduced by the prototype live under `crates/`.

pub use overseer_core::{
    ComponentDescriptor, DaemonBuilder, DaemonDefinition, DependencyRelationship, OverseerError,
    Result, RpcOperationDescriptor, ServiceDescriptor,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn facade_exposes_core_concepts_without_owning_implementation() {
        let component = ComponentDescriptor::new(
            "facade_component",
            "Facade Component",
            "Proves root crate can access core descriptors",
        )
        .unwrap();

        let daemon = DaemonBuilder::new("facaded")
            .unwrap()
            .component(component)
            .unwrap()
            .build()
            .unwrap();

        assert_eq!(daemon.name(), "facaded");
        assert_eq!(daemon.components()[0].id(), "facade_component");
        assert_eq!(daemon.inspection_categories()[0], "daemon");
    }
}
