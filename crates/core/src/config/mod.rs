//! Configuration as a dependency-injection citizen.
//!
//! A [`ConfigManager`] (built in `main`) is split into small structs bound to property
//! paths: `app.db.reader` and `app.db.writer` may both deserialize the same
//! `DbConfig` type, injected wherever needed as [`Cfg<T>`]. Binding keys on the
//! property path — the same type may appear at several paths — with a type-only
//! shorthand that resolves only when exactly one binding of that type exists.

mod source;

use std::ops::Deref;
use std::path::PathBuf;
use std::sync::Arc;

use serde::de::DeserializeOwned;

use crate::descriptors::{BoxedComponent, Injectable, TypeDescriptor};

pub use overseerd_config::{DefaultSpec, EnumTag};

/// Runtime, object-safe access to a config type's [`DefaultSpec`].
///
/// [`ConfigProperties`] exposes its defaults as the associated `const`
/// [`DEFAULTS`](ConfigProperties::DEFAULTS), which is unreachable through a trait object (a
/// `const` is not part of the vtable). This companion trait — blanket-implemented for every
/// `ConfigProperties` — re-exposes that const through a `&self` method, so the spec can be
/// read from a value or a `dyn ConfigDefaults`.
pub trait ConfigDefaults {
    /// This type's field defaults (its [`ConfigProperties::DEFAULTS`]).
    fn defaults(&self) -> DefaultSpec;
}

impl<T: ConfigProperties> ConfigDefaults for T {
    fn defaults(&self) -> DefaultSpec {
        T::DEFAULTS
    }
}
#[cfg(feature = "yaml")]
pub use source::Yaml;
pub use source::{ConfigManager, Dynamic, Format, FormatId, Toml};

/// Errors from loading, merging, and binding configuration.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("failed to read config file '{}': {source}", .path.display())]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("failed to parse config file '{}': {source}", .path.display())]
    Parse {
        path: PathBuf,
        #[source]
        source: overseerd_config::ConfigError,
    },

    #[error("no configuration found at path '{path}'")]
    MissingPath { path: String },

    #[error("config substitution failed at '{path}': {source}")]
    Substitution {
        path: String,
        #[source]
        source: overseerd_config::ConfigError,
    },
}

/// An injected configuration value, bound from a property path.
///
/// Wraps `Arc<T>` and derefs to it. The path is supplied at the injection site via
/// `#[config("...")]` (or omitted for the sole-binding shorthand). Reads always go
/// through `Cfg<T>` rather than a raw `Arc<T>`, which reserves the seam a future
/// hot-reload needs without changing any injection site.
pub struct Cfg<T>(pub Arc<T>);

impl<T> Clone for Cfg<T> {
    fn clone(&self) -> Self {
        Self(Arc::clone(&self.0))
    }
}

impl<T> Deref for Cfg<T> {
    type Target = Arc<T>;

    fn deref(&self) -> &Arc<T> {
        &self.0
    }
}

impl<T: Send + Sync + 'static> Injectable for Cfg<T> {
    type Target = T;
}

/// A struct bindable from a configuration subtree, injectable as [`Cfg<Self>`].
///
/// Implemented by `#[config]`; the type must also be
/// `Deserialize`. The default [`bind`](Self::bind) deserializes the subtree and wraps
/// it, so the builder can construct the value without naming the concrete type.
pub trait ConfigProperties: DeserializeOwned + Send + Sync + 'static + Sized {
    /// A display name for the type, used in descriptors and error messages.
    const NAME: &'static str;

    /// The type's `#[default = ".."]` field defaults, emitted by the `#[config]` macro as a
    /// compile-time constant.
    ///
    /// Defaults to [`DefaultSpec::None`](overseerd_config::DefaultSpec::None) — no fields
    /// carry a default. The values are template strings merged *under* the config so they
    /// resolve through the normal `${...}` pipeline (see
    /// [`ConfigManager::get_config`](crate::ConfigManager::get_config)). For runtime access
    /// through a value or trait object, use [`ConfigDefaults::defaults`].
    const DEFAULTS: DefaultSpec = DefaultSpec::none();

    /// Deserializes this type from the subtree at `path` (filling missing fields from
    /// [`DEFAULTS`](Self::DEFAULTS)) and wraps it as a stored `Cfg<Self>` handle.
    fn bind(tree: &ConfigManager, path: &str) -> Result<BoxedComponent, ConfigError> {
        let value: Self = tree.get_config::<Self>(path)?;

        Ok(BoxedComponent {
            ty: TypeDescriptor::of::<Self>(Self::NAME),
            value: Box::new(Cfg(Arc::new(value))),
        })
    }
}

/// A requested binding of a config type to a property path, registered on the
/// [`ConfigManager`] and resolved against the merged tree at build. The same type may be
/// bound at several paths; the `bind` thunk is monomorphized per type so the manager need not
/// name it. `defaults` is the type's compile-time [`DefaultSpec`], carried so the manager can
/// seed every bound type's defaults into the tree (enabling cross-path `${a.b.c}` references).
#[derive(Clone)]
pub struct ConfigBinding {
    pub ty: TypeDescriptor,
    pub path: String,
    pub bind: fn(&ConfigManager, &str) -> Result<BoxedComponent, ConfigError>,
    pub defaults: DefaultSpec,
}

impl ConfigBinding {
    /// Builds a binding for type `T` at `path`, capturing `T`'s `bind` thunk and
    /// compile-time defaults.
    pub fn of<T: ConfigProperties>(path: impl Into<String>) -> Self {
        Self {
            ty: TypeDescriptor::of::<T>(T::NAME),
            path: path.into(),
            bind: T::bind,
            defaults: T::DEFAULTS,
        }
    }
}

impl std::fmt::Debug for ConfigBinding {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConfigBinding")
            .field("ty", &self.ty)
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

/// The link-time form of a [`ConfigBinding`]: a config type with a fixed property
/// path, registered by `#[config(path = "..")]` so it is picked up by
/// [`ConfigManager::auto_discover`]. The same type may still be bound at extra paths
/// explicitly via [`ConfigManager::with_config`] / [`DaemonBuilder::config`](crate::DaemonBuilder::config).
/// It is also exposed on the type itself as
/// [`Descriptor<ConfigBindingDescriptor>`](crate::Descriptor).
pub struct ConfigBindingDescriptor {
    pub ty: TypeDescriptor,
    pub path: &'static str,
    pub bind: fn(&ConfigManager, &str) -> Result<BoxedComponent, ConfigError>,
    pub defaults: DefaultSpec,
}

impl ConfigBindingDescriptor {
    /// Lifts this link-time descriptor into a runtime binding.
    pub fn to_binding(&self) -> ConfigBinding {
        ConfigBinding {
            ty: self.ty,
            path: self.path.to_string(),
            bind: self.bind,
            defaults: self.defaults,
        }
    }
}
