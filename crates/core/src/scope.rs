/// Lifetime policy for a component instance.
///
/// Determines where the instance is stored and how long it lives: a `Singleton`
/// in the root container for the application's lifetime, a `Connection`/`Request`
/// instance in a per-connection/per-call scope, and a `Transient` built fresh on
/// every resolution. The captive-dependency rule (a longer-lived component may not
/// depend on a shorter-lived one) is enforced against [`rank`](Self::rank).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ComponentScope {
    Singleton,
    Connection,
    Request,
    Transient,
}

impl ComponentScope {
    /// Lifetime rank: longer-lived scopes rank higher. A non-transient component
    /// may depend only on equal-or-higher-ranked non-transient components.
    ///
    /// Defining the lifetime order as a numeric rank (rather than matching on each
    /// variant at every call site) is what lets a future user-defined scope slot in
    /// at its own rank without touching the validation or container logic.
    pub fn rank(self) -> u8 {
        match self {
            ComponentScope::Singleton => 3,
            ComponentScope::Connection => 2,
            ComponentScope::Request => 1,
            ComponentScope::Transient => 0,
        }
    }

    /// Whether this scope rebuilds its instance on every resolution rather than
    /// caching one per scope.
    pub fn is_transient(self) -> bool {
        matches!(self, ComponentScope::Transient)
    }
}
