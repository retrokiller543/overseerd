//! Crate-root resolution for macro-generated code.
//!
//! Generated code must name framework items by path. Two roots matter, and both are
//! overridable so a macro works wherever the crates live (a fork, a vendored tree, a
//! third-party plugin):
//!
//! - **core** — the always-present `upwell` facade: vocabulary, the DI engine, config,
//!   hooks, the transport substrate, and the agnostic `client` module. Every plugin relies on
//!   it, so core items resolve here (`::upwell::Component`, `::upwell::client::Client`, …).
//! - **plugin** — the crate owning the macro's *own* generated types. For the RPC macros that
//!   is the `daemon` module (`::upwell::daemon::ServiceDescriptor`, …); a third-party plugin
//!   points it at its own crate.
//!
//! Each macro crate supplies its defaults (so built-in macros are zero-config); a
//! per-invocation `upwell = ::fork` / `crate = ::my_plugin` overrides them.

use quote::ToTokens;
use syn::Path;

pub const UPWELL_CRATE: &str = "upwell";

/// A path to a **core-framework** item, rooted at the always-present `upwell` facade.
/// Used for vocabulary, the DI engine, config, hooks, and transport — everything any plugin
/// can rely on. (The fixed-root counterpart of [`Paths::core`]; the codegen will migrate to
/// the parameterized [`Paths`] as the per-macro path overrides land.)
pub fn upwell_path(item: &str) -> Path {
    syn::parse_str(&format!("::{UPWELL_CRATE}::{item}")).expect("valid upwell facade item path")
}

/// A path to a **daemon (RPC) plugin** item, rooted at the facade's `daemon` module
/// (`::upwell::daemon::<item>`). The fixed-root counterpart of [`Paths::plugin`].
pub fn upwell_daemon_path(item: &str) -> Path {
    syn::parse_str(&format!("::{UPWELL_CRATE}::daemon::{item}"))
        .expect("valid upwell daemon item path")
}

/// A path to a **protocol-agnostic client** item, rooted at the facade's `client` module
/// (`::upwell::client::<item>`). The fixed-root counterpart of [`Paths::client`].
pub fn upwell_client_path(item: &str) -> Path {
    syn::parse_str(&format!("::{UPWELL_CRATE}::client::{item}"))
        .expect("valid upwell client item path")
}

/// The resolved crate roots a macro emits against. Construct with the macro crate's defaults
/// (e.g. [`Paths::upwell`] / [`Paths::upwell_daemon`]) and override per-invocation.
#[derive(Clone)]
pub struct Paths {
    core: Path,
    plugin: Path,
}

impl Default for Paths {
    /// The core-macro default: both roots at the `upwell` facade. Macro crates with a
    /// different default (the RPC macros use [`upwell_daemon`](Paths::upwell_daemon))
    /// construct explicitly.
    fn default() -> Self {
        Self::upwell()
    }
}

impl Paths {
    /// Roots with an explicit `core` facade path and `plugin` own-types path.
    pub fn new(core: Path, plugin: Path) -> Self {
        Self { core, plugin }
    }

    /// The default for **core** macros (`#[component]`, `#[config]`, …): both roots are the
    /// `upwell` facade, since their generated types live at the facade root.
    pub fn upwell() -> Self {
        let root: Path = syn::parse_quote!(::upwell);

        Self {
            core: root.clone(),
            plugin: root,
        }
    }

    /// The default for the **RPC daemon** macros: core at `::upwell`, own types under
    /// `::upwell::daemon`.
    pub fn upwell_daemon() -> Self {
        Self {
            core: syn::parse_quote!(::upwell),
            plugin: syn::parse_quote!(::upwell::daemon),
        }
    }

    /// Overrides the core (`upwell` facade) root — the `upwell = ::fork` argument.
    pub fn with_core(mut self, core: Path) -> Self {
        self.core = core;

        self
    }

    /// Overrides the plugin (own-types) root — the `crate = ::my_plugin` argument.
    pub fn with_plugin(mut self, plugin: Path) -> Self {
        self.plugin = plugin;

        self
    }

    /// Applies optional per-invocation overrides (the `upwell = ..` / `crate = ..` macro
    /// args) onto these defaults, leaving unset roots unchanged.
    pub fn resolve(mut self, core: Option<Path>, plugin: Option<Path>) -> Self {
        if let Some(core) = core {
            self.core = core;
        }

        if let Some(plugin) = plugin {
            self.plugin = plugin;
        }

        self
    }

    /// A core-framework item: `<core>::<item>` (e.g. `::upwell::Component`).
    pub fn core(&self, item: &str) -> Path {
        self.join(&self.core, item)
    }

    /// A plugin-owned item: `<plugin>::<item>` (e.g. `::upwell::daemon::ServiceDescriptor`).
    pub fn plugin(&self, item: &str) -> Path {
        self.join(&self.plugin, item)
    }

    /// An agnostic **client** item: `<core>::client::<item>` (e.g.
    /// `::upwell::client::Client`). The generated client surface is protocol-agnostic, so it
    /// roots at the core facade's `client` module — never at a protocol's `plugin` root.
    pub fn client(&self, item: &str) -> Path {
        self.join(&self.core, &format!("client::{item}"))
    }

    /// Appends `::<item>` (which may itself contain `::`) to a root path.
    fn join(&self, root: &Path, item: &str) -> Path {
        let combined = format!("{}::{item}", root.to_token_stream());

        syn::parse_str(&combined).expect("valid composed item path")
    }
}
