//! Immutable protocol-owned scope topology declarations and validation.

use std::fmt;

use overseerd_core::{Scope, ScopeId, Singleton, StaticScope, Transient};
use thiserror::Error;

mod plan;

pub(crate) use plan::{ScopePlan, SeedDestination};

/// The declared parent of a protocol-owned scope boundary.
///
/// The application root is implicit and therefore is not itself a boundary in a
/// [`ScopeTopology`].
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum ScopeParent {
    /// The framework-owned application root.
    Root,
    /// Another boundary declared by the same topology.
    Boundary(ScopeId),
}

impl ScopeParent {
    /// Creates a parent reference to a static scope type.
    pub const fn of<T: StaticScope>() -> Self {
        Self::Boundary(T::ID)
    }

    /// Creates a parent reference to another declared boundary.
    pub const fn boundary(id: ScopeId) -> Self {
        Self::Boundary(id)
    }

    /// Returns the stable identity of this parent.
    pub fn id(self) -> ScopeId {
        match self {
            Self::Root => Singleton.id(),
            Self::Boundary(id) => id,
        }
    }

    /// Returns whether this parent is the implicit application root.
    pub const fn is_root(self) -> bool {
        matches!(self, Self::Root)
    }
}

/// One openable scope and its single declared parent boundary.
#[derive(Clone, Copy)]
pub struct ScopeBoundary {
    scope: &'static dyn Scope,
    id: ScopeId,
    name: &'static str,
    rank: u8,
    parent: ScopeParent,
}

impl ScopeBoundary {
    /// Declares an openable scope boundary under its only valid parent.
    pub const fn new<T>(scope: &'static T, parent: ScopeParent) -> Self
    where
        T: StaticScope,
    {
        Self {
            scope,
            id: T::ID,
            name: T::NAME,
            rank: T::RANK,
            parent,
        }
    }

    /// Returns the stable identity of this boundary.
    pub const fn id(self) -> ScopeId {
        self.id
    }

    /// Returns the scope lifetime metadata and diagnostic label.
    pub const fn scope(self) -> &'static dyn Scope {
        self.scope
    }

    /// Returns the immutable diagnostic label captured from the static scope declaration.
    pub const fn name(self) -> &'static str {
        self.name
    }

    /// Returns the immutable lifetime rank captured from the static scope declaration.
    pub const fn rank(self) -> u8 {
        self.rank
    }

    /// Returns this boundary's only valid parent.
    pub const fn parent(self) -> ScopeParent {
        self.parent
    }
}

impl fmt::Debug for ScopeBoundary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ScopeBoundary")
            .field("id", &self.id())
            .field("name", &self.name())
            .field("rank", &self.rank())
            .field("parent", &self.parent)
            .finish()
    }
}

/// An immutable declaration of protocol-owned scope boundaries.
///
/// The root is implicit. Every item in [`boundaries`](Self::boundaries) declares
/// exactly one non-root boundary and its only valid parent. Call [`prepare`](Self::prepare)
/// before using a topology for planning or runtime opening.
#[derive(Clone, Copy, Debug)]
pub struct ScopeTopology {
    boundaries: &'static [ScopeBoundary],
}

impl ScopeTopology {
    /// Creates a topology declaration from static scope boundaries.
    pub const fn new(boundaries: &'static [ScopeBoundary]) -> Self {
        Self { boundaries }
    }

    /// Returns the boundaries in declaration order.
    pub const fn boundaries(self) -> &'static [ScopeBoundary] {
        self.boundaries
    }

    /// Validates and owns this topology for deterministic planning and lookup.
    pub fn prepare(self) -> Result<PreparedScopeTopology, ScopeTopologyError> {
        PreparedScopeTopology::new(self.boundaries)
    }
}

/// A validated, immutable scope topology ready for planning and runtime lookup.
///
/// Boundaries are stored in stable identity order. The implicit root is available
/// to ancestry and reachability queries but is never returned by [`boundary`](Self::boundary).
#[derive(Clone, Debug)]
pub struct PreparedScopeTopology {
    boundaries: Box<[ScopeBoundary]>,
}

impl PreparedScopeTopology {
    fn new(boundaries: &[ScopeBoundary]) -> Result<Self, ScopeTopologyError> {
        let mut boundaries = boundaries.to_vec();

        boundaries.sort_by_key(|boundary| boundary.id());
        validate_ids(&boundaries)?;
        validate_parents(&boundaries)?;
        validate_cycles(&boundaries)?;
        validate_parent_ranks(&boundaries)?;

        Ok(Self {
            boundaries: boundaries.into_boxed_slice(),
        })
    }

    /// Returns all declared boundaries in stable identity order.
    pub fn boundaries(&self) -> &[ScopeBoundary] {
        &self.boundaries
    }

    /// Returns the declared boundary with `id`, if present.
    pub fn boundary(&self, id: ScopeId) -> Option<&ScopeBoundary> {
        let index = self
            .boundaries
            .binary_search_by_key(&id, |boundary| boundary.id())
            .ok()?;

        self.boundaries.get(index)
    }

    /// Returns whether `id` names a declared non-root boundary.
    pub fn contains(&self, id: ScopeId) -> bool {
        self.boundary(id).is_some()
    }

    /// Returns the declared parent of `id`, if `id` is a boundary.
    pub fn parent_of(&self, id: ScopeId) -> Option<ScopeParent> {
        self.boundary(id).map(|boundary| boundary.parent())
    }

    /// Iterates from a boundary's immediate parent through the implicit root.
    ///
    /// An undeclared ID and the root itself have no ancestors.
    pub fn ancestors(&self, id: ScopeId) -> impl Iterator<Item = ScopeId> + '_ {
        let first = self.parent_of(id).map(ScopeParent::id);

        std::iter::successors(first, move |parent| {
            self.parent_of(*parent).map(ScopeParent::id)
        })
    }

    /// Returns whether `ancestor` is a strict ancestor of `descendant`.
    ///
    /// The implicit root is an ancestor of every declared boundary.
    pub fn is_ancestor(&self, ancestor: ScopeId, descendant: ScopeId) -> bool {
        self.ancestors(descendant)
            .any(|candidate| candidate == ancestor)
    }

    /// Returns whether a consumer boundary can reach a dependency boundary.
    ///
    /// A dependency is reachable when it is the consumer itself or one of its
    /// ancestors. This makes siblings unreachable regardless of lifetime rank.
    pub fn is_reachable(&self, consumer: ScopeId, dependency: ScopeId) -> bool {
        let root = Singleton.id();
        let consumer_exists = consumer == root || self.contains(consumer);
        let dependency_exists = dependency == root || self.contains(dependency);

        consumer_exists
            && dependency_exists
            && (consumer == dependency || self.is_ancestor(dependency, consumer))
    }
}

/// A structural failure in a protocol-owned scope topology declaration.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
#[non_exhaustive]
pub enum ScopeTopologyError {
    /// The same stable identity appears more than once.
    #[error("scope boundary '{id}' is declared more than once")]
    DuplicateId {
        /// The duplicated stable identity.
        id: ScopeId,
    },
    /// A declaration attempts to use a framework-owned anchor identity.
    #[error("scope boundary '{id}' uses a reserved framework identity")]
    ReservedId {
        /// The reserved root or transient identity.
        id: ScopeId,
    },
    /// A boundary names itself as its parent.
    #[error("scope boundary '{id}' cannot be its own parent")]
    SelfParent {
        /// The self-parented boundary identity.
        id: ScopeId,
    },
    /// A boundary's parent is not declared by the topology.
    #[error("scope boundary '{id}' refers to missing parent '{parent}'")]
    MissingParent {
        /// The child boundary identity.
        id: ScopeId,
        /// The undeclared parent identity.
        parent: ScopeId,
    },
    /// Parent links contain a structural cycle.
    #[error("scope topology contains a parent cycle involving {members:?}")]
    Cycle {
        /// The cycle members in canonical parent-link order.
        members: Box<[ScopeId]>,
    },
    /// A parent is not strictly longer-lived than its child.
    #[error(
        "scope boundary '{child}' has rank {child_rank}, but parent '{parent}' has rank {parent_rank}; parent rank must be greater than child rank"
    )]
    InvalidParentRank {
        /// The child boundary identity.
        child: ScopeId,
        /// The child lifetime rank.
        child_rank: u8,
        /// The parent boundary or root identity.
        parent: ScopeId,
        /// The parent lifetime rank.
        parent_rank: u8,
    },
}

fn validate_ids(boundaries: &[ScopeBoundary]) -> Result<(), ScopeTopologyError> {
    let root = Singleton.id();
    let transient = Transient.id();

    for (index, boundary) in boundaries.iter().enumerate() {
        let id = boundary.id();

        if index > 0 && boundaries[index - 1].id() == id {
            return Err(ScopeTopologyError::DuplicateId { id });
        }

        if id == root || id == transient {
            return Err(ScopeTopologyError::ReservedId { id });
        }
    }

    Ok(())
}

fn validate_parents(boundaries: &[ScopeBoundary]) -> Result<(), ScopeTopologyError> {
    for boundary in boundaries {
        let id = boundary.id();
        let ScopeParent::Boundary(parent) = boundary.parent() else {
            continue;
        };

        if id == parent {
            return Err(ScopeTopologyError::SelfParent { id });
        }

        if find_boundary(boundaries, parent).is_none() {
            return Err(ScopeTopologyError::MissingParent { id, parent });
        }
    }

    Ok(())
}

fn validate_cycles(boundaries: &[ScopeBoundary]) -> Result<(), ScopeTopologyError> {
    for boundary in boundaries {
        let mut path = Vec::new();
        let mut current = boundary.id();

        loop {
            if let Some(cycle_start) = path.iter().position(|candidate| *candidate == current) {
                let members = canonical_cycle(path[cycle_start..].to_vec());

                return Err(ScopeTopologyError::Cycle { members });
            }

            path.push(current);

            let current_boundary = find_boundary(boundaries, current)
                .expect("parent existence is validated before cycle detection");

            match current_boundary.parent() {
                ScopeParent::Root => break,
                ScopeParent::Boundary(parent) => current = parent,
            }
        }
    }

    Ok(())
}

fn validate_parent_ranks(boundaries: &[ScopeBoundary]) -> Result<(), ScopeTopologyError> {
    for boundary in boundaries {
        let parent = boundary.parent().id();
        let parent_rank = match boundary.parent() {
            ScopeParent::Root => Singleton.rank(),
            ScopeParent::Boundary(parent) => find_boundary(boundaries, parent)
                .expect("parent existence is validated before rank ordering")
                .rank(),
        };
        let child_rank = boundary.rank();

        if parent_rank <= child_rank {
            return Err(ScopeTopologyError::InvalidParentRank {
                child: boundary.id(),
                child_rank,
                parent,
                parent_rank,
            });
        }
    }

    Ok(())
}

fn find_boundary(boundaries: &[ScopeBoundary], id: ScopeId) -> Option<&ScopeBoundary> {
    let index = boundaries
        .binary_search_by_key(&id, |boundary| boundary.id())
        .ok()?;

    boundaries.get(index)
}

fn canonical_cycle(mut members: Vec<ScopeId>) -> Box<[ScopeId]> {
    let start = members
        .iter()
        .enumerate()
        .min_by_key(|(_, id)| **id)
        .map(|(index, _)| index)
        .unwrap_or_default();

    members.rotate_left(start);

    members.into_boxed_slice()
}

#[cfg(test)]
mod tests;
