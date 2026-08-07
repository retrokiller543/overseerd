use overseerd::app;
use overseerd::axum::axum::Json;
use overseerd::axum::axum::http::StatusCode;
use overseerd::axum::axum::response::{IntoResponse, Response};
use overseerd::axum::client::{HyperClient, ReqwestClient};
use overseerd::axum::prelude::*;
use overseerd_test_utils::{TestEnvironment, TestServer, deadline};

app! {
    app ResponseTestApplication {
        name: "response-contract-test",
        protocol: overseerd::axum::Axum,
    }
}

#[dto]
#[derive(Clone, Debug, PartialEq)]
pub struct Accepted {
    id: u64,
}

#[dto]
#[derive(Clone, Debug, PartialEq)]
pub struct Rejected {
    reason: String,
}

#[dto]
#[derive(Clone, Debug, PartialEq)]
enum ManualOutcome {
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
struct Responses {
    #[default]
    _unit: (),
}

#[handlers]
impl Responses {
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

    #[get(
        "/manual/{accepted}",
        response = ManualOutcome,
        responses = [
            (status = 202, body = Accepted),
            (status = 422, body = Rejected),
        ]
    )]
    async fn manual(&self, Path(accepted): Path<bool>) -> Response {
        if accepted {
            (StatusCode::ACCEPTED, Json(Accepted { id: 9 })).into_response()
        } else {
            (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(Rejected {
                    reason: String::from("manual-invalid"),
                }),
            )
                .into_response()
        }
    }

    #[delete("/empty")]
    async fn empty(&self) -> StatusCode {
        StatusCode::NO_CONTENT
    }
}

#[tokio::test]
async fn generated_clients_decode_every_status_contract() {
    let environment = TestEnvironment::new("overseerd-http-responses-");
    let app = ResponseTestApplication::builder()
        .expect("app builder")
        .config_source(environment.config())
        .directories(environment.directories())
        .build()
        .await
        .expect("app builds");
    let server = TestServer::start_with_guard(app, environment).await;
    let base = format!("http://{}", server.address());
    let reqwest = ResponsesClient::new(ReqwestClient::new(base.clone()));
    let hyper = ResponsesClient::new(HyperClient::new(base));

    let accepted = deadline("generated accepted", reqwest.generated(true))
        .await
        .expect("accepted response");
    assert_eq!(accepted.status(), StatusCode::ACCEPTED);
    assert!(matches!(
        accepted.body(),
        ResponsesGeneratedResponse::Status202(Accepted { id: 7 })
    ));

    let rejected = deadline("generated rejected", reqwest.generated(false))
        .await
        .expect("rejected response");
    assert_eq!(rejected.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert!(matches!(
        rejected.body(),
        ResponsesGeneratedResponse::Status422(Rejected { reason }) if reason == "invalid"
    ));

    let manual = deadline("manual accepted", hyper.manual(true))
        .await
        .expect("manual response");
    assert_eq!(manual.status(), StatusCode::ACCEPTED);
    assert_eq!(manual.body(), &ManualOutcome::Accepted(Accepted { id: 9 }));

    let empty = deadline("empty response", hyper.empty())
        .await
        .expect("empty response");
    assert_eq!(empty.status(), StatusCode::NO_CONTENT);
    assert_eq!(*empty.body(), ());

    server.shutdown().await;
}
