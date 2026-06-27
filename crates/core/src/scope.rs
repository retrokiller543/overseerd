//! Component lifetime scopes.
//!
//! A scope is a pure *label* describing where a component instance is stored and how
//! long it lives. It is modelled as the object-safe [`Scope`] trait (rather than a
//! fixed enum) so a protocol can declare its own lifetimes and carry them as a
//! `&'static [&'static dyn Scope]` chain.
//!
//! The core knows only the two *universal anchors*: [`Singleton`] (one instance for the
//! whole application — the longest-lived scope, so [`u8::MAX`] rank) and [`Transient`]
//! (rebuilt on every resolution — the shortest-lived, so [`u8::MIN`] rank). Every other
//! scope a protocol defines necessarily ranks *between* them, so the captive-dependency
//! bounds hold by construction. Connection/request and any other protocol-shaped
//! lifetimes live in their protocol's crate, not here.

/// Ergonomic authoring sugar for a zero-sized scope: provide the three associated
/// constants and a [`Scope`] impl follows via the blanket impl below. Plugin and user
/// scopes (`Connection`, `Request`, a custom `#[scope]`) declare themselves this way.
pub trait StaticScope: Send + Sync + 'static {
    /// A rank defining the lifetime of the scope relative to others. Longer-lived scopes rank higher.
    /// There are only two ranks reserved for the framework, and both are the two extremes of the `u8`
    /// range: [`Singleton`] occupies [`u8::MAX`] and [`Transient`] [`u8::MIN`].
    const RANK: u8;

    /// The name used for debug purposes.
    const NAME: &'static str;
    
    const IS_TRANSIENT: bool = false;
}

impl<T: StaticScope> Scope for T {
    fn rank(&self) -> u8 {
        Self::RANK
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn is_transient(&self) -> bool {
        Self::IS_TRANSIENT
    }
}

/// A lifetime scope label: where a component instance is stored and how long it
/// lives.
///
/// Object-safe, so a protocol can carry a `&'static [&'static dyn Scope]` chain of
/// the scopes it opens. A scope is a pure *label* — per-session state is not held
/// here; it rides the opened scope container's seeds. The captive-dependency rule (a
/// longer-lived component may not depend on a shorter-lived one) is enforced against
/// [`rank`](Self::rank).
pub trait Scope: Send + Sync + 'static {
    /// Lifetime rank: longer-lived scopes rank higher. A non-transient component may
    /// depend only on equal-or-higher-ranked non-transient components.
    ///
    /// [`Singleton`] occupies [`u8::MAX`] and [`Transient`] [`u8::MIN`], so every
    /// protocol- or user-defined scope ranks strictly between them. Defining the
    /// lifetime order as a number (rather than matching on each label) is what lets a
    /// new scope slot in at its own rank without touching the validation or container
    /// logic.
    fn rank(&self) -> u8;

    /// Stable identifier, for debug/display and build-time scope-chain validation.
    fn name(&self) -> &'static str;

    /// Whether this scope rebuilds its instance on every resolution rather than
    /// caching one per scope.
    fn is_transient(&self) -> bool {
        false
    }
}

/// The root scope: a single instance for the whole application lifetime. The
/// longest-lived scope there can be, so it ranks [`u8::MAX`] — a singleton may depend
/// only on other singletons.
pub struct Singleton;

/// A transient scope: rebuilt fresh on every resolution and never cached. The
/// shortest-lived scope there can be, so it ranks [`u8::MIN`].
pub struct Transient;

impl StaticScope for Singleton {
    const RANK: u8 = u8::MAX;
    const NAME: &'static str = "Singleton";
}

impl StaticScope for Transient {
    const RANK: u8 = u8::MIN;
    const NAME: &'static str = "Transient";
    const IS_TRANSIENT: bool = true;
}
