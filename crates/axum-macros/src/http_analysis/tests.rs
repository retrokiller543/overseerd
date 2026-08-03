use syn::{ImplItemFn, parse_quote};

use super::{input, responses};
use crate::route::parse_route_attr;

#[test]
fn detects_direct_redirects_status_tuples_and_builders_without_body_inference() {
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
    let responses = responses(&method, &route);

    assert!(responses.iter().any(|response| response.status == 403));
    assert!(responses.iter().any(|response| response.status == 201));
    assert!(responses.iter().any(|response| {
        response.status == 303
            && response
                .redirect
                .as_ref()
                .map(syn::LitStr::value)
                .as_deref()
                == Some("/callback")
    }));
    assert!(responses.iter().all(|response| response.body.is_none()));
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
