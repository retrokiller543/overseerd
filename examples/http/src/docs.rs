//! An OpenAPI-focused demo controller (native + server only). It exists to document two spec
//! behaviours the plain [`greet`](crate::greet) controller does not exercise:
//!
//! - a `Form` request body, which must be advertised as `application/x-www-form-urlencoded` (not the
//!   JSON default utoipa would otherwise assume), and
//! - a handler that overrides the generated responses with its own `#[openapi(responses(..))]` — the
//!   custom set fully replaces the generated `200` rather than being appended beside it.
//!
//! It is not wired for serving; linking it populates the OpenAPI link-time slices for the tests.

use overseerd::axum::axum::extract::Form;
use overseerd::axum::dto;
use overseerd::axum::prelude::*;

use crate::greet::Greeter;

/// A form-encoded login submission — the body of the `application/x-www-form-urlencoded` route.
#[dto]
pub struct LoginForm {
    pub user: String,
    pub password: String,
}

/// The login acknowledgement body.
#[dto]
pub struct LoginAck {
    pub user: String,
    pub ok: bool,
}

/// Documents OpenAPI-specific request/response shapes. Holds the shared [`Greeter`] like the plain
/// controller, purely so it is an ordinary injected singleton.
#[controller(path = "/docs-demo")]
pub struct DocsController {
    greeter: Greeter,
}

#[handlers]
impl DocsController {
    /// `POST /docs-demo/login` — a form-encoded body, documented as `application/x-www-form-urlencoded`.
    #[post("/login")]
    async fn login(&self, Form(form): Form<LoginForm>) -> Json<LoginAck> {
        let _ = self.greeter.greet(&form.user);

        Json(LoginAck {
            user: form.user,
            ok: !form.password.is_empty(),
        })
    }

    /// `GET /docs-demo/teapot` — overrides the generated responses. Both statuses come from the
    /// `#[openapi(..)]` set; the generated default `200` is dropped, so `utoipa::path` sees exactly
    /// one `responses` argument.
    #[get("/teapot")]
    #[openapi(responses(
        (status = 200, body = LoginAck, description = "brewed"),
        (status = 418, description = "I'm a teapot"),
    ))]
    async fn teapot(&self) -> Json<LoginAck> {
        Json(LoginAck {
            user: String::from("teapot"),
            ok: false,
        })
    }
}
