//! The DI↔axum bridge: a per-request scope handle and the [`Inject`] extractor.
//!
//! The integration surface between the framework's dependency injection and axum is a
//! single [`ScopeHandle`] — an `Arc` of the request's
//! [`ScopeContainer`](overseerd_di::ScopeContainer) — carried in the request's
//! [`Extensions`](axum::http::Extensions). The protocol's scope layer opens the request
//! scope and inserts the handle; [`Inject`] reads it back out and resolves a component
//! through the scope chain. Native axum extractors (`Json`, `Path`, `Query`, `State`, …)
//! are untouched and compose with [`Inject`] in the same handler signature.

use std::sync::Arc;

use axum::extract::FromRequestParts;
use axum::http::StatusCode;
use axum::http::request::Parts;
use axum::response::{IntoResponse, Response};
use overseerd_di::{FromContainer, ScopeContainer};

/// The per-request DI scope, carried in request extensions.
///
/// Inserted by the protocol's scope layer at the start of every request and read by
/// [`Inject`]. Cloning is cheap — it is an `Arc` bump.
#[derive(Clone)]
pub struct ScopeHandle(pub Arc<ScopeContainer>);

/// Injects anything a constructor parameter can be — any [`FromContainer`] — from the
/// request's scope: a component handle (`Arc<T>`, `Dep<T>`, a by-value injectable), every
/// provider of a trait (`Vec<Arc<dyn T>>` / `HashMap<String, Arc<dyn T>>`), an optional
/// component (`Option<Arc<T>>`), or a resolver-backed value such as `Cfg<T>`. Resolution
/// walks the request → singleton scope chain and constructs a fresh instance when the target
/// is a `Transient`.
///
/// This is how a route reaches request-scoped and transient components: the route runs
/// inside the request scope, so injection can reach down to shorter-lived components that a
/// singleton controller cannot hold as a field. The controller itself arrives through
/// `&self`, resolved once when its router is built.
pub struct Inject<H>(pub H);

/// Why an [`Inject`] extraction failed: a misconfiguration rather than bad client input, so
/// both arms render as `500 Internal Server Error` (and log the cause).
#[derive(Debug)]
pub enum InjectRejection {
    /// No [`ScopeHandle`] in the request extensions — the scope layer was not installed.
    MissingScope,

    /// The requested type could not be extracted from the container (no such component or
    /// provider, or a resolver-backed value like `Cfg<T>` was unavailable). Carries the type
    /// name and the underlying DI error.
    Unresolved(&'static str, String),
}

impl IntoResponse for InjectRejection {
    fn into_response(self) -> Response {
        match self {
            InjectRejection::MissingScope => {
                tracing::error!(
                    target: "overseerd::axum",
                    "request scope handle missing from extensions; is the AxumPlugin scope layer installed?"
                );
            }

            InjectRejection::Unresolved(name, error) => {
                tracing::error!(
                    target: "overseerd::axum",
                    type_name = name,
                    error = %error,
                    "failed to extract injected dependency"
                );
            }
        }

        StatusCode::INTERNAL_SERVER_ERROR.into_response()
    }
}

impl<S, H> FromRequestParts<S> for Inject<H>
where
    S: Send + Sync,
    H: FromContainer,
{
    type Rejection = InjectRejection;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let scope = parts
            .extensions
            .get::<ScopeHandle>()
            .ok_or(InjectRejection::MissingScope)?
            .0
            .clone();

        scope.extract::<H>().await.map(Inject).map_err(|error| {
            InjectRejection::Unresolved(std::any::type_name::<H>(), error.to_string())
        })
    }
}
