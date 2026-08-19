pub use upwell_core::{IdErrorKind, InvalidNamespacedId as InvalidCompositionId};

upwell_core::namespaced_id_type!(
    /// A stable namespaced protocol definition identity.
    pub struct ProtocolId,
    "protocol"
);
upwell_core::namespaced_id_type!(
    /// A stable namespaced plugin implementation identity.
    pub struct PluginId,
    "plugin"
);
upwell_core::namespaced_id_type!(
    /// A stable namespaced capability slot that a plugin implementation may occupy.
    pub struct PluginSlotId,
    "plugin slot"
);
upwell_core::namespaced_id_type!(
    /// A stable contributor-local identity for one composition contribution.
    pub struct ContributionId,
    "contribution"
);
