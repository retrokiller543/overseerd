pub use overseerd_core::{IdErrorKind, InvalidNamespacedId as InvalidCompositionId};

overseerd_core::namespaced_id_type!(
    /// A stable namespaced protocol definition identity.
    pub struct ProtocolId,
    "protocol"
);
overseerd_core::namespaced_id_type!(
    /// A stable namespaced plugin implementation identity.
    pub struct PluginId,
    "plugin"
);
overseerd_core::namespaced_id_type!(
    /// A stable namespaced capability slot that a plugin implementation may occupy.
    pub struct PluginSlotId,
    "plugin slot"
);
overseerd_core::namespaced_id_type!(
    /// A stable contributor-local identity for one composition contribution.
    pub struct ContributionId,
    "contribution"
);
