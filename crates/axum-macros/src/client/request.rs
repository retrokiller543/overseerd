use overseerd_macros_core::paths::Paths;
use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{Ident, Type};

use super::inputs::{Body, BodyKind, QueryInput};
use crate::route::RouteAttr;

/// The ergonomic body parameter type for a classified request body.
pub(super) fn body_param_type(body: &Option<Body>, paths: &Paths) -> Option<Type> {
    let Body { kind, inner } = body.as_ref()?;

    let ty = match kind {
        BodyKind::Json | BodyKind::Form => inner
            .clone()
            .expect("a Json/Form body carries its payload type"),
        BodyKind::Bytes | BodyKind::RawForm => syn::parse_quote!(::std::vec::Vec<u8>),
        BodyKind::Multipart => {
            let multipart = paths.plugin("client::Multipart");
            syn::parse_quote!(#multipart)
        }
    };

    Some(ty)
}

/// Returns the body parameter declaration, wire wrapper type, and wrapped value expression.
pub(super) fn body_parts(
    body: &Option<Body>,
    paths: &Paths,
) -> (TokenStream, TokenStream, TokenStream) {
    let Some(Body { kind, inner }) = body else {
        return (quote!(), quote!(()), quote!(()));
    };

    let (encode_ty, body_value) = match kind {
        BodyKind::Json => {
            let wrapper = paths.plugin("client::Json");
            (quote!(#wrapper<#inner>), quote!(#wrapper(request)))
        }
        BodyKind::Form => {
            let wrapper = paths.plugin("client::Form");
            (quote!(#wrapper<#inner>), quote!(#wrapper(request)))
        }
        BodyKind::Bytes => {
            let wrapper = paths.plugin("client::OctetStream");
            (quote!(#wrapper), quote!(#wrapper(request)))
        }
        BodyKind::RawForm => {
            let wrapper = paths.plugin("client::RawForm");
            (quote!(#wrapper), quote!(#wrapper(request)))
        }
        BodyKind::Multipart => {
            let multipart = paths.plugin("client::Multipart");
            (quote!(#multipart), quote!(request))
        }
    };

    let param_ty = body_param_type(body, paths);
    (quote!(, request: #param_ty), encode_ty, body_value)
}

/// Returns a query argument and its URI suffix expression.
pub(super) fn query_parts(
    query: &Option<QueryInput>,
    paths: &Paths,
) -> Option<((Ident, TokenStream), TokenStream)> {
    let name = format_ident!("query");

    let (param_ty, suffix) = match query.as_ref()? {
        QueryInput::Typed(ty) => {
            let encode_query = paths.plugin("client::encode_query");
            (
                quote!(#ty),
                quote! {{
                    let __q = #encode_query(&#name);

                    if __q.is_empty() {
                        ::std::string::String::new()
                    } else {
                        ::std::format!("?{}", __q)
                    }
                }},
            )
        }
        QueryInput::Raw => (
            quote!(::core::option::Option<::std::string::String>),
            quote! {
                match &#name {
                    ::core::option::Option::Some(__q) if !__q.is_empty() => {
                        ::std::format!("?{}", __q)
                    }
                    _ => ::std::string::String::new(),
                }
            },
        ),
    };

    Some(((name, param_ty), suffix))
}

/// Builds the typed `http::Request<B>` constructor for a route.
#[allow(clippy::too_many_arguments)]
pub(super) fn request_builder(
    route: &RouteAttr,
    fmt: &str,
    holes: &[String],
    subst: &[TokenStream],
    query_suffix: &TokenStream,
    encode_ty: &TokenStream,
    body_value: &TokenStream,
    per_call_headers: bool,
    paths: &Paths,
) -> TokenStream {
    let http = paths.plugin("http");
    let http_body = paths.plugin("client::HttpBody");
    let encode_path_segment = paths.plugin("client::encode_path_segment");
    let encode_path_segments = paths.plugin("client::encode_path_segments");
    let verb = format_ident!("{}", route.verb.to_string().to_uppercase());
    let base = quote!(Self::BASE);
    let subst = subst
        .iter()
        .zip(holes)
        .map(|(subst, hole)| {
            if hole.starts_with('*') {
                quote!(#encode_path_segments(&#subst))
            } else {
                quote!(#encode_path_segment(&#subst))
            }
        })
        .collect::<Vec<_>>();
    let uri =
        quote!(::std::format!("{}{}", ::std::format!(#fmt, #base #(, #subst)*), #query_suffix));

    let header_merge = if per_call_headers {
        quote! {
            if let ::core::option::Option::Some(__headers) = headers {
                for (__name, __value) in __headers.iter() {
                    __request.headers_mut().insert(__name.clone(), __value.clone());
                }
            }
        }
    } else {
        quote!()
    };

    quote! {{
        let mut __builder = #http::Request::builder()
            .method(#http::Method::#verb)
            .uri(#uri);

        if let ::core::option::Option::Some(__ct) = <#encode_ty as #http_body>::CONTENT_TYPE {
            __builder = __builder.header(#http::header::CONTENT_TYPE, __ct);
        }

        let mut __request = __builder
            .body(#body_value)
            .expect("client request is valid by construction");

        #header_merge
        __request
    }}
}
