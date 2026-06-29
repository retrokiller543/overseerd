//! The Overseerd axum/HTTP protocol, built on the protocol-agnostic `overseerd-app` core.
//!
//! This crate is a [`ProtocolPlugin`]: it builds a real [`axum::Router`] from `#[controller]`
//! components, bridges the framework's dependency injection into axum via the [`Inject`]
//! extractor (so route handlers mix native axum extractors with DI), and serves over HTTP.
//! Depend on it directly, or reach it through the `overseerd` facade's `axum` feature.
//!
//! The DI bridge is deliberately thin and one-directional: nothing in `overseerd-di` or
//! `overseerd-core` knows axum exists. A per-request scope layer threads an
//! [`Arc<ScopeContainer>`](overseerd_di::ScopeContainer) through the request extensions, and
//! [`Inject`] resolves components from it.

pub mod controller;
pub mod error;
pub mod extract;
pub mod plugin;
pub mod protocol;
pub mod scope;

pub use controller::{CONTROLLERS, Controller, ControllerDescriptor};
pub use error::{Error, Result};
pub use extract::{Inject, InjectRejection, ScopeHandle};
pub use plugin::{AxumAppBuilder, AxumPlugin};
pub use protocol::Axum;

/// The axum app type: an [`App`](overseerd_app::App) specialized to [`AxumPlugin`].
/// `App::builder(name)` resolves through this alias without a turbofish.
pub type App = overseerd_app::App<AxumPlugin>;

/// The axum app builder: [`AppBuilder`](overseerd_app::AppBuilder) specialized to [`AxumPlugin`].
pub type AppBuilder = overseerd_app::AppBuilder<AxumPlugin>;

// Re-export the agnostic app surface so a standalone `overseerd-axum` user has one import.
pub use overseerd_app::{
    AppRegistry, AppRuntime, LoggingConfig, Plugin, Protocol, ProtocolPlugin, Serve, ServerConfig,
    ShutdownHandle, ShutdownSignal,
};

/// Re-exported so macro-generated code can reach the `#[distributed_slice]` attribute for
/// the `CONTROLLERS` slice through a stable path.
#[doc(hidden)]
pub use linkme;

/// Re-exported so `#[controller]`/`#[handlers]` generated code and users reach axum through a
/// stable path without a separate dependency.
pub use axum;
