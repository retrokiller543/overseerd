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
    BoxedComponent, Component, ComponentConstructionContext, ComponentDescriptor, ComponentFactory,
    ComponentScope, Conn, DependencyDescriptor, Descriptor, DescriptorRegistry, Error, Extension,
    FromContext, Handler, OperationKind, ParameterDescriptor, ParameterKind, Payload, Result,
    RpcCallContext, RpcDescriptor, RpcGroup, RpcHandler, RpcResponse, ServiceDescriptor,
    TypeDescriptor, dispatch_with, handlers, rpc, service, type_id_of,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn facade_exposes_core_types() {
        let td = TypeDescriptor::of::<u8>("byte");

        assert_eq!(td.name, "byte");
        assert_eq!((td.type_id)(), (type_id_of::<u8>)());
    }
}
