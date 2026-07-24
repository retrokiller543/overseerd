//! `app!` expansion: defines a reusable application host or assembles a legacy builder.
//!
//! ```ignore
//! app! {
//!     pub app Example {
//!         name: "example-daemon",
//!         protocol: RpcPlugin,
//!         services: [Notifications, Echo],
//!         configs: [DbConfig => "app.db.reader", DbConfig => "app.db.writer"],
//!     }
//! }
//!
//! let app = Example::builder()?.build().await?;
//! ```
//!
//! Each `managers` entry is either an **instance** (any expression) or a **config block**
//! (`{ key: value, .. }`) that applies settings to just that manager. A `config` block with
//! no `source` is loaded from the `directories` manager (which must then be present), so the
//! file-reload triggers (`sighup`/`watch`/`debounce`) configure the `ConfigManager` itself,
//! never the app. The listed `services` are asserted `Wired` (under `di-check`).

mod builder;
#[cfg(feature = "cli")]
mod cli;
mod command;
mod model;
mod named;
mod parsing;
mod phase;

use proc_macro2::TokenStream;

pub(crate) use model::{AppAssembly, AppInput, NamedApp};

/// Expands a parsed application definition.
pub fn expand(input: AppInput) -> TokenStream {
    match input {
        AppInput::Named(input) => named::expand(input),
        AppInput::Legacy(input) => builder::expand(input),
    }
}

#[cfg(test)]
mod tests;
