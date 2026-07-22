use core::marker::PhantomData;
use core::ops::Deref;
use inventory::Registry;

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

/// Provides object-safe access to a type's descriptor at runtime.
///
/// The blanket implementation bridges statically described concrete types to
/// trait objects whose trait includes `RuntimeDescriptor<D>` as a supertrait.
pub trait RuntimeDescriptor<D> {
    fn descriptor(&self) -> D;
}

impl<T, D> RuntimeDescriptor<D> for T
where
    T: Descriptor<D>,
{
    #[inline(always)]
    fn descriptor(&self) -> D {
        T::DESCRIPTOR
    }
}

/// Marker for every valid link-time descriptor kind — the `D` in [`DescriptorFor`].
///
/// **Open, not sealed:** protocol and third-party plugin crates implement this for their own
/// descriptor kinds, registering through the same seam as the in-tree ones. The `Copy + Send +
/// Sync + 'static` supertraits are the guard against bad types: they are the concrete requirements
/// the registration backends impose (the shared `inventory` static must be `Sync + 'static`; the
/// accessors copy the descriptor out of the slice/collection), so a type that cannot meet them
/// cannot be registered.
pub trait OverseerdDescriptor: Copy + Send + Sync + 'static {}

/// A descriptor `D` tagged with the owner type `T` it belongs to.
///
/// This is the per-type bucket key for the `inventory` registration backend: one distinct
/// `(T, D)` pair is one `inventory` collection, so `inventory::iter::<DescriptorFor<T, D>>()`
/// yields exactly the descriptors of kind `D` registered against `T` — no runtime scan across
/// unrelated types. `T` is a phantom disambiguator only; `PhantomData<fn() -> T>` keeps the
/// shared static `Sync` regardless of whether `T` is.
#[derive(Clone, Copy)]
pub struct DescriptorFor<T, D: OverseerdDescriptor> {
    descriptor: D,
    _owner: PhantomData<fn() -> T>,
}

impl<T, D: OverseerdDescriptor> DescriptorFor<T, D> {
    /// Wraps an explicit descriptor value. Used by the many-per-type kinds (component factories,
    /// hooks, rpc groups, routes), where a type carries several descriptors of the same kind.
    pub const fn new(descriptor: D) -> Self {
        Self {
            descriptor,
            _owner: PhantomData,
        }
    }
}

impl<T, D: OverseerdDescriptor> Deref for DescriptorFor<T, D> {
    type Target = D;

    #[inline]
    fn deref(&self) -> &D {
        &self.descriptor
    }
}

impl<T, D: OverseerdDescriptor> AsRef<D> for DescriptorFor<T, D> {
    #[inline]
    fn as_ref(&self) -> &D {
        &self.descriptor
    }
}

/// Supplies the `inventory` registry that backs a `(T, D)` bucket.
///
/// A blanket `impl inventory::Collect for DescriptorFor<T, D>` cannot be written directly by user
/// code — `DescriptorFor` is foreign to the user crate and not `#[fundamental]`, so a local owner
/// `T` as its argument does not satisfy the orphan rule (`E0117`). This trait sidesteps that: the
/// owner type `T` is **local** to the crate defining it, so `impl RegistryFor<D> for T` is always
/// orphan-legal there. The generated impl holds the per-`(T, D)` `Registry` in a `static` inside its
/// concrete `registry()` — one registry per monomorphization — and the blanket `Collect` impl below
/// forwards to it. The macros emit one `impl RegistryFor<D> for T` per owner/kind.
pub trait RegistryFor<D: OverseerdDescriptor> {
    /// The `(Self, D)` bucket's registry — a distinct `&'static Registry` per implementing type.
    fn registry() -> &'static Registry;
}

impl<T, D> inventory::Collect for DescriptorFor<T, D>
where
    T: RegistryFor<D> + 'static,
    D: OverseerdDescriptor,
{
    #[inline]
    fn registry() -> &'static Registry {
        <T as RegistryFor<D>>::registry()
    }
}

impl<T, D> DescriptorFor<T, D>
where
    T: Descriptor<D>,
    D: OverseerdDescriptor,
{
    /// Pulls the descriptor straight from `T`'s static [`Descriptor<D>`] impl — the zero-arg
    /// convenience for the one-descriptor-per-type header kinds (component/service/controller/
    /// provider headers) that already carry an `impl Descriptor<D>`.
    pub const fn of() -> Self {
        Self::new(<T as Descriptor<D>>::DESCRIPTOR)
    }
}

#[cfg(test)]
mod tests;
