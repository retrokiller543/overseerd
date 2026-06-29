//! The axum protocol plugin and its builder extension.

use std::sync::Arc;

use axum::extract::Request;
use axum::middleware::{self, Next};
use axum::response::IntoResponse;
use overseerd_app::{AppBuilder, AppRegistry, AppRuntime, Plugin, ProtocolPlugin};
use overseerd_core::{Descriptor, Scope};
use overseerd_di::ComponentDescriptor;

use crate::controller::{CONTROLLERS, ControllerDescriptor};
use crate::extract::ScopeHandle;
use crate::protocol::Axum;
use crate::scope::Request as RequestScope;

/// The axum HTTP protocol plugin.
///
/// Accumulates the registered/discovered controllers, contributes no extra DI seeds, and
/// builds the [`Axum`] protocol: each controller's [`axum::Router`] merged together and
/// wrapped by a per-request scope layer that opens the request scope and threads it into
/// the request extensions for the [`Inject`](crate::Inject) extractor.
#[derive(Default)]
pub struct AxumPlugin {
    controllers: Vec<ControllerDescriptor>,
}

impl Plugin for AxumPlugin {
    fn auto_discover(&mut self) {
        self.controllers.extend(CONTROLLERS.iter().copied());
    }

    fn register(&self, _registry: &mut AppRegistry) {}
}

impl ProtocolPlugin for AxumPlugin {
    type Protocol = Axum;
    type Error = crate::Error;

    const SCOPES: &'static [&'static dyn Scope] = &[&RequestScope];

    fn build(self, runtime: &AppRuntime) -> crate::Result<Axum> {
        // Merge every controller's routes. Each builder resolves its controller singleton
        // from the runtime and captures it in the route handlers, so the merged router owns
        // ready-to-call handlers.
        let mut router = axum::Router::new();

        for descriptor in &self.controllers {
            router = router.merge((descriptor.router)(runtime));
        }

        // The bridge: a per-request layer that opens the Request scope (parented at the
        // singleton root) and inserts its handle into the request extensions. `Inject`
        // reads it back out; a scope-build failure degrades to 500 rather than panicking.
        let scope_runtime = runtime.clone();
        let router = router.layer(middleware::from_fn(
            move |mut request: Request, next: Next| {
                let scope_runtime = scope_runtime.clone();

                async move {
                    let parent = Arc::clone(scope_runtime.root());

                    match scope_runtime
                        .open_scope(&RequestScope, parent, Vec::new())
                        .await
                    {
                        Ok(scope) => {
                            request.extensions_mut().insert(ScopeHandle(scope));

                            next.run(request).await
                        }

                        Err(error) => {
                            tracing::error!(
                                target: "overseerd::axum",
                                error = %error,
                                "request scope build failed"
                            );

                            axum::http::StatusCode::INTERNAL_SERVER_ERROR.into_response()
                        }
                    }
                }
            },
        ));

        Ok(Axum::new(router))
    }
}

/// axum-specific builder methods, contributed to [`AppBuilder<AxumPlugin>`] as an extension
/// trait. Bring it into scope to register controllers by type; it is in the prelude.
///
/// Controllers also auto-register through the [`CONTROLLERS`] slice, so an
/// `auto_discover`d app needs no explicit registration — this is the explicit path (and
/// the one the `app!` macro could drive).
pub trait AxumAppBuilder {
    /// Registers controller type `T`: its route header and its construction factory.
    fn controller<T>(self) -> Self
    where
        T: Descriptor<ControllerDescriptor> + Descriptor<ComponentDescriptor>;

    /// Manually registers a raw controller header (prefer [`controller`](Self::controller)).
    fn controller_descriptor(self, descriptor: &'static ControllerDescriptor) -> Self;
}

impl AxumAppBuilder for AppBuilder<AxumPlugin> {
    fn controller<T>(mut self) -> Self
    where
        T: Descriptor<ControllerDescriptor> + Descriptor<ComponentDescriptor>,
    {
        self.protocol_mut()
            .controllers
            .push(<T as Descriptor<ControllerDescriptor>>::DESCRIPTOR);

        self.component::<T>()
    }

    fn controller_descriptor(mut self, descriptor: &'static ControllerDescriptor) -> Self {
        self.protocol_mut().controllers.push(*descriptor);

        self
    }
}
