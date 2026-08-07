use syn::{ImplItemFn, parse_quote};

use super::{ResponseBody, ResponseOrigin, input, response_contract};
use crate::route::parse_route_attr;

#[test]
fn detects_only_returned_leaves_and_ignores_unused_builders() {
    let method: ImplItemFn = parse_quote! {
        async fn route(&self, denied: bool) -> impl IntoResponse {
            if denied {
                return (StatusCode::FORBIDDEN, "denied").into_response();
            }

            let response = Response::builder()
                .status(StatusCode::CREATED)
                .body(body);

            Redirect::to("/callback")
        }
    };
    let attr = parse_quote!(#[get("/route")]);
    let route = parse_route_attr(&attr).expect("route parses");
    let conventional: syn::Type = parse_quote!(impl IntoResponse);
    let contract = response_contract(&method, &route, &conventional);
    let responses = &contract.alternatives;

    assert_eq!(contract.origin, ResponseOrigin::Inferred);
    assert!(responses.iter().any(|response| response.status == 403));
    assert!(!responses.iter().any(|response| response.status == 201));
    assert!(responses.iter().any(|response| {
        response.status == 303
            && response
                .redirect
                .as_ref()
                .map(syn::LitStr::value)
                .as_deref()
                == Some("/callback")
    }));
    assert!(responses.iter().any(|response| {
        response.status == 403 && matches!(response.body, ResponseBody::Opaque)
    }));
}

#[test]
fn explicit_responses_are_authoritative() {
    let method: ImplItemFn = parse_quote! {
        async fn route() -> impl IntoResponse {
            Redirect::to("/inferred")
        }
    };
    let attr = parse_quote!(
        #[get("/route", responses = [(status = 403, body = ErrorBody)])]
    );
    let route = parse_route_attr(&attr).expect("route parses");
    let conventional: syn::Type = parse_quote!(impl IntoResponse);
    let contract = response_contract(&method, &route, &conventional);

    assert_eq!(contract.origin, ResponseOrigin::Explicit);
    assert_eq!(contract.alternatives.len(), 1);
    assert_eq!(contract.alternatives[0].status, 403);
    assert!(matches!(
        contract.alternatives[0].body,
        ResponseBody::Typed(_)
    ));
}

#[test]
fn traces_builder_body_through_local_json_encoding() {
    let method: ImplItemFn = parse_quote! {
        async fn route() -> Response {
            let body = Json(WhoAmI { name: None });
            let bytes = body.encode().expect("encode");
            let json = String::from_utf8(bytes).expect("utf8");

            Response::builder()
                .status(StatusCode::FORBIDDEN)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::new(json))
                .expect("response")
        }
    };
    let route = parse_route_attr(&parse_quote!(#[get("/route")])).expect("route parses");
    let conventional: syn::Type = parse_quote!(Response);
    let contract = response_contract(&method, &route, &conventional);

    assert_eq!(contract.alternatives.len(), 1);
    assert_eq!(contract.alternatives[0].status, 403);
    let ResponseBody::Typed(body) = &contract.alternatives[0].body else {
        panic!("body should be inferred")
    };
    assert_eq!(quote::quote!(#body).to_string(), "WhoAmI");
}

#[test]
fn axum_extra_wrappers_are_classified_without_resolving_the_crate() {
    let json: syn::Type = parse_quote!(axum_extra::extract::JsonDeserializer<Input>);
    let protobuf: syn::Type = parse_quote!(axum_extra::protobuf::Protobuf<Message>);
    let cached: syn::Type = parse_quote!(axum_extra::extract::Cached<TypedHeader<Auth>>);

    assert_eq!(input(&json).0, "JsonBody");
    assert_eq!(input(&protobuf).0, "BytesBody");
    assert_eq!(input(&cached).0, "Header");
}
