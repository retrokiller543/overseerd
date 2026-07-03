//! Middleware demo: a raw `axum::middleware::from_fn` closure registered globally (standard
//! axum middleware needs no wrapping to keep working), a DI-backed [`AxumMiddleware`] scoped to
//! one route, and the worked example from the middleware design — a request-scoped component
//! that reads the `Authorization` header via [`RequestMeta`], "fetches" a user once, and is
//! shared by every handler that injects it, with no second fetch.
//!
//! `middleware = [Type, ..]` takes the identical DI-typed list on `#[controller(..)]` (scoping
//! to every route on that controller) as it does here on `#[get(..)]` (scoping to just this
//! route) — [`RequireAuth`] just happens to guard a single route in this demo.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use overseerd::axum::axum::Json;
use overseerd::axum::axum::extract::Request;
use overseerd::axum::axum::http::{StatusCode, header};
use overseerd::axum::axum::middleware::Next;
use overseerd::axum::axum::response::{IntoResponse, Response};
use overseerd::axum::prelude::*;
use overseerd::axum::{AxumMiddleware, RequestMeta};
use overseerd::{component, methods};

/// A plain `axum::middleware::from_fn` closure — standard, un-wrapped axum middleware,
/// registered globally in `main.rs` via `.layer(...)` alongside the DI-backed kind below.
pub async fn log_requests(req: Request, next: Next) -> Response {
    let method = req.method().clone();
    let uri = req.uri().clone();

    let response = next.run(req).await;

    tracing::info!(
        target: "example_http::auth",
        %method, %uri, status = %response.status(),
        "request"
    );

    response
}

/// A trivial in-memory user directory, standing in for a database call. A real implementation
/// would hold a connection pool instead.
#[component(by_value)]
#[derive(Clone)]
struct UserDirectory {
    #[default]
    fetches: Arc<AtomicUsize>,
}

impl UserDirectory {
    /// "Looks up" a user by bearer token — in this demo, the token *is* the username.
    fn fetch(&self, token: &str) -> String {
        self.fetches.fetch_add(1, Ordering::Relaxed);

        format!("user:{token}")
    }
}

/// Rejects a request missing an `Authorization` header before it reaches the handler. A
/// `#[component]` implementing [`AxumMiddleware`] is resolved once (as a DI singleton) and
/// shared across every attach point it's registered at, instead of being constructed per
/// attach point.
#[component]
struct RequireAuth;

impl AxumMiddleware for RequireAuth {
    async fn handle(&self, req: Request, next: Next) -> Response {
        if req.headers().contains_key(header::AUTHORIZATION) {
            return next.run(req).await;
        }

        StatusCode::UNAUTHORIZED.into_response()
    }
}

/// The worked example: a request-scoped component that reads the bearer token from
/// [`RequestMeta`], fetches the user once, and is then shared — via the request scope's
/// per-type caching — by every handler that injects it, with no second fetch.
#[component(scope = Request)]
struct AuthenticatedUser {
    #[default]
    name: Option<String>,
}

#[methods]
impl AuthenticatedUser {
    #[init]
    async fn init(meta: RequestMeta, users: UserDirectory) -> Self {
        let token = meta
            .headers
            .get(header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.strip_prefix("Bearer "));

        Self {
            name: token.map(|token| users.fetch(token)),
        }
    }
}

/// The `/me` response: the authenticated user's name, and whether both `Inject`ions below
/// resolved the same cached instance.
#[dto]
struct WhoAmI {
    name: Option<String>,
    same_instance: bool,
}

/// A controller with no middleware of its own — [`RequireAuth`] and [`AuthenticatedUser`] are
/// only attached to `/whoami` below, not the whole controller.
#[controller(path = "/me")]
struct MeController;

#[handlers]
impl MeController {
    /// `GET /me/public` — no `Authorization` header required. Returns a plain string (some APIs do);
    /// a borrowed response is allowed as a wire type, though the typed client can't decode into it.
    #[get("/public")]
    async fn public(&self) -> &'static str {
        "hello, anonymous"
    }

    /// `GET /me/whoami` — guarded by `RequireAuth` (path-level middleware). Injecting
    /// [`AuthenticatedUser`] twice proves it was fetched once and shared, not re-fetched.
    #[get("/whoami", middleware = [RequireAuth])]
    async fn whoami(
        &self,
        Inject(user): Inject<Arc<AuthenticatedUser>>,
        Inject(same_user): Inject<Arc<AuthenticatedUser>>,
    ) -> Json<WhoAmI> {
        Json(WhoAmI {
            name: user.name.clone(),
            same_instance: Arc::ptr_eq(&user, &same_user),
        })
    }
}
