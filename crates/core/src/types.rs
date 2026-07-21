use std::{any::TypeId, fmt};

/// Returns the [`TypeId`] of `T` for use in const descriptor construction.
///
/// `?Sized` so trait-object keys (`dyn Trait`) can be used to look up providers.
pub const fn type_id_of<T: ?Sized + 'static>() -> TypeId {
    TypeId::of::<T>()
}

/// Static metadata describing a Rust type, safe to embed in `'static` descriptors.
///
/// `type_name` remains a function pointer so it can be stored in statics and
/// coerce naturally in proc macro output: `type_name: std::any::type_name::<MyType>`.
#[derive(Clone, Copy)]
pub struct TypeDescriptor {
    pub name: &'static str,
    pub type_name: fn() -> &'static str,
    pub type_id: TypeId,
}

impl TypeDescriptor {
    /// Constructs a TypeDescriptor for `T`.
    ///
    /// Safe to use in `static` initializers.
    pub const fn of<T: ?Sized + 'static>(name: &'static str) -> Self {
        Self {
            name,
            type_name: std::any::type_name::<T>,
            type_id: type_id_of::<T>(),
        }
    }
}

impl fmt::Debug for TypeDescriptor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", (self.type_name)())
    }
}
