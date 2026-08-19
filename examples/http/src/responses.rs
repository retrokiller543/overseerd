//! Response-shape stress examples for generated clients, tooling, and OpenAPI.

use upwell::axum::prelude::*;

#[cfg(not(target_family = "wasm"))]
use upwell::axum::axum::Json;
#[cfg(not(target_family = "wasm"))]
use upwell::axum::axum::http::StatusCode;
#[cfg(not(target_family = "wasm"))]
use upwell::axum::axum::response::{IntoResponse, Response};

#[dto]
#[derive(Clone, Debug, PartialEq)]
pub struct Accepted {
    pub id: u64,
}

#[dto]
#[derive(Clone, Debug, PartialEq)]
pub struct Rejected {
    pub reason: String,
}

#[dto]
#[derive(Clone, Debug, PartialEq)]
pub enum ManualOutcome {
    Accepted(Accepted),
    Rejected(Rejected),
}

impl From<Accepted> for ManualOutcome {
    fn from(value: Accepted) -> Self {
        Self::Accepted(value)
    }
}

impl From<Rejected> for ManualOutcome {
    fn from(value: Rejected) -> Self {
        Self::Rejected(value)
    }
}

#[controller(path = "/responses")]
pub struct ResponseController {
    #[default]
    _unit: (),
}

#[handlers]
impl ResponseController {
    /// Distinct inferred leaves generate a status enum.
    #[get("/generated/{accepted}")]
    async fn generated(&self, Path(accepted): Path<bool>) -> impl IntoResponse {
        if accepted {
            return (StatusCode::ACCEPTED, Json(Accepted { id: 7 })).into_response();
        }

        (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(Rejected {
                reason: String::from("invalid"),
            }),
        )
            .into_response()
    }

    /// A user-owned enum can be reused while per-status DTOs remain explicit.
    #[get(
        "/manual/{accepted}",
        response = ManualOutcome,
        responses = [
            (status = 202, body = Accepted),
            (status = 422, body = Rejected),
        ]
    )]
    async fn manual(&self, Path(accepted): Path<bool>) -> Response {
        opaque_manual_response(accepted)
    }

    /// Explicit metadata is authoritative even though this syntax would otherwise look like 200.
    #[get("/authoritative", responses = [(status = 418, body = Rejected)])]
    async fn authoritative(&self) -> Response {
        opaque_teapot_response()
    }

    /// Empty responses retain status without inventing a body type.
    #[delete("/empty")]
    async fn empty(&self) -> upwell::axum::http::StatusCode {
        upwell::axum::http::StatusCode::NO_CONTENT
    }

    /// Empty and redirect outcomes are distinct generated response variants.
    #[get(
        "/empty-or-redirect",
        responses = [
            (status = 204),
            (status = 303, redirect = "/responses/empty"),
        ]
    )]
    async fn empty_or_redirect(&self) -> Response {
        opaque_teapot_response()
    }
}

#[cfg(not(target_family = "wasm"))]
fn opaque_manual_response(accepted: bool) -> Response {
    if accepted {
        (StatusCode::ACCEPTED, Json(Accepted { id: 7 })).into_response()
    } else {
        (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(Rejected {
                reason: String::from("invalid"),
            }),
        )
            .into_response()
    }
}

#[cfg(not(target_family = "wasm"))]
fn opaque_teapot_response() -> Response {
    (
        StatusCode::IM_A_TEAPOT,
        Json(Rejected {
            reason: String::from("teapot"),
        }),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use upwell::axum::client::{HttpResponse, ReqwestClient};

    use super::{ManualOutcome, ResponseControllerClient, ResponseControllerGeneratedResponse};

    fn generated_type(
        client: &ResponseControllerClient<ReqwestClient>,
    ) -> impl Future<
        Output = Result<
            HttpResponse<ResponseControllerGeneratedResponse>,
            upwell::client::ClientError<upwell::axum::http::StatusCode>,
        >,
    > + '_ {
        client.generated(true)
    }

    fn manual_type(
        client: &ResponseControllerClient<ReqwestClient>,
    ) -> impl Future<
        Output = Result<
            HttpResponse<ManualOutcome>,
            upwell::client::ClientError<upwell::axum::http::StatusCode>,
        >,
    > + '_ {
        client.manual(true)
    }

    #[test]
    fn generated_and_user_owned_response_types_are_stable() {
        let client = ResponseControllerClient::new(ReqwestClient::new("http://localhost"));
        let _generated = generated_type(&client);
        let _manual = manual_type(&client);
    }
}
