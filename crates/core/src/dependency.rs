use crate::types::TypeDescriptor;

/// Cardinality of a dependency edge: how many values satisfy it.
///
/// A `One` edge wants a single value, resolved by its handle's `Target` type —
/// whether that is a concrete type (`Arc<T>`) or a trait object (`Arc<dyn Trait>`).
/// For a trait object the container has already placed the chosen (primary or
/// sole) provider under the trait's `TypeId`, so the dependency resolves through
/// the same path and never sees the `#[primary]` selection itself.
///
/// `Collection` and `Keyed` are *multi-valued* and always satisfiable — zero
/// providers yields an empty `Vec`/`HashMap`, never a missing-dependency error.
/// Only `One` (when not `optional`) requires a value to exist.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cardinality {
    /// A single value, resolved by concrete type or by the chosen trait provider
    /// (`Arc<T>`, a by-value handle, or `Arc<dyn Trait>`).
    One,
    /// Every provider of a trait, as `Vec<Arc<dyn Trait>>`. Empty is valid.
    Collection,
    /// Every provider of a trait keyed by qualifier, as `HashMap<String, Arc<dyn Trait>>`. Empty is valid.
    Keyed,
}

impl Cardinality {
    /// Whether an edge of this cardinality requires at least one value to exist.
    /// Multi-valued edges (`Collection`/`Keyed`) accept zero.
    pub fn requires_provider(self) -> bool {
        matches!(self, Cardinality::One)
    }
}

/// Declares that a component requires another component (or a config value).
///
/// The edge's shape is described by orthogonal axes: `cardinality` (how many
/// providers satisfy it), `optional` (whether absence is tolerated), and `dynamic`
/// (whether the provider is registered at runtime rather than discovered — which
/// exempts the edge from static dependency validation). This is pure vocabulary,
/// shared by the DI engine, the hook system, and the config layer.
#[derive(Clone, Copy, Debug)]
pub struct DependencyDescriptor {
    pub name: &'static str,
    pub ty: TypeDescriptor,
    pub cardinality: Cardinality,
    pub optional: bool,
    pub dynamic: bool,
    /// For a single `Arc<dyn Trait>` edge, selects a specific provider by its
    /// qualifier (`#[qualifier = ".."]`) instead of the primary/sole one. For a
    /// `config` edge it carries the property path (`#[config("..")]`), or `None` for
    /// the sole-binding shorthand.
    pub qualifier: Option<&'static str>,
    /// Whether this edge resolves a `#[config]` binding (a `Cfg<T>` keyed by property
    /// path) rather than a component or trait provider. Config edges are validated
    /// against the registered config bindings, not the component graph, so they are
    /// exempt from the standard dependency/scope checks.
    pub config: bool,
}
