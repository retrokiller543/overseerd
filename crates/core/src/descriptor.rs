/// Connects a type to a static descriptor of kind `D`, enabling type-to-descriptor
/// lookups where the type is known but its link-time descriptor would otherwise
/// only be reachable through a slice.
///
/// A type may implement this once per descriptor kind: a `#[service]` carries both
/// `Descriptor<ComponentDescriptor>` (its construction factory) and
/// `Descriptor<ServiceDescriptor>` (its identity header). This is what lets the
/// daemon builder register a component/service/config *by type*
/// (`builder.service::<T>()`) instead of by `&'static` descriptor or by global
/// auto-discovery.
///
/// RPC groups are deliberately *not* expressed this way: a single service may span
/// several `#[handlers]` blocks, so the relationship is one-to-many.
pub trait Descriptor<D> {
    const DESCRIPTOR: D;
}
