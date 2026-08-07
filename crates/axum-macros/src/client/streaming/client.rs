use overseerd_macros_core::client::{Capability, ClientMethod};
use overseerd_macros_core::paths::Paths;
use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{Ident, ReturnType, Type};

use super::super::inputs::classify;
use super::super::path::{parse_template, plan_path};
use super::super::request::{body_param_type, body_parts, query_parts, request_builder};
use super::super::response::response_type;
use crate::route::RouteAttr;

/// Builds a server-streaming client method hint.
pub(crate) fn build_stream_client_method(
    method_ident: &Ident,
    route: &RouteAttr,
    arg_types: &[&Type],
    wrapper_unit: TokenStream,
    item: Type,
    paths: &Paths,
) -> Option<ClientMethod> {
    let inputs = classify(arg_types)?;
    let (_req_param, encode_ty, body_value) = body_parts(&inputs.body, paths);
    let request_param = body_param_type(&inputs.body, paths);
    let (fmt, holes) = parse_template(&route.path.value());
    let path_plan = plan_path(&holes, inputs.path_ty);
    let query_plan = query_parts(&inputs.query, paths);
    let query_suffix = query_plan
        .as_ref()
        .map(|(_, suffix)| suffix.clone())
        .unwrap_or_else(|| quote!(""));
    let mut extra_args = path_plan.args;

    if let Some((query_arg, _)) = query_plan {
        extra_args.push(query_arg);
    }

    let request = request_builder(
        route,
        &fmt,
        &holes,
        &path_plan.subst,
        &query_suffix,
        &encode_ty,
        &body_value,
        false,
        paths,
    );
    let client_error = paths.client("ClientError");
    let http = paths.plugin("http");
    let encodes = paths.client("Encodes");
    let http_streaming = paths.plugin("client::HttpStreaming");
    let stream_decode = paths.plugin("client::StreamDecode");
    let stream_trait = paths.plugin("__Stream");
    let request = ClientMethodOverrideBody {
        bounds: quote!(C: #http_streaming + #encodes<#encode_ty>),
        ret: quote! {
            ::core::result::Result<
                impl #stream_trait<Item = #item>,
                #client_error<#http::StatusCode>,
            >
        },
        body: quote! {{
            let __request = #request;
            let __bytes = #http_streaming::open_stream(&self.0, __request).await?;

            ::core::result::Result::Ok(
                <#wrapper_unit as #stream_decode<#item>>::decode_stream(__bytes),
            )
        }},
    };

    Some(ClientMethod {
        ident: method_ident.clone(),
        path: String::new(),
        capability: Capability::ServerStreaming,
        request: request_param,
        encode_as: None,
        req_item: None,
        resp_item: None,
        response: item,
        error_ty: None,
        extra_args,
        request_envelope: None,
        request_builder: None,
        response_envelope: None,
        response_mapper: None,
        trailing_args: Vec::new(),
        attrs: Vec::new(),
        override_bounds: Some(request.bounds),
        override_ret: Some(request.ret),
        override_body: Some(request.body),
    })
}

struct ClientMethodOverrideBody {
    bounds: TokenStream,
    ret: TokenStream,
    body: TokenStream,
}

/// Builds a client-streaming route method hint.
pub(crate) fn build_client_stream_method(
    method_ident: &Ident,
    route: &RouteAttr,
    arg_types: &[&Type],
    item: Type,
    output: &ReturnType,
    paths: &Paths,
) -> Option<ClientMethod> {
    let inputs = classify(arg_types)?;

    if inputs.body.is_some() {
        return None;
    }

    let (fmt, holes) = parse_template(&route.path.value());
    let path_plan = plan_path(&holes, inputs.path_ty);
    let response = response_type(output);
    let http = paths.plugin("http");
    let ndjson = paths.plugin("Ndjson");
    let stream_encode = paths.plugin("StreamEncode");
    let encode_stream = paths.plugin("client::encode_stream");
    let http_client_streaming = paths.plugin("client::HttpClientStreaming");
    let http_response = paths.plugin("client::HttpResponse");
    let encode_path_segment = paths.plugin("client::encode_path_segment");
    let encode_path_segments = paths.plugin("client::encode_path_segments");
    let client_error = paths.client("ClientError");
    let decodes = paths.client("Decodes");
    let stream_arg = paths.client("StreamArg");
    let query_plan = query_parts(&inputs.query, paths);
    let query_suffix = query_plan
        .as_ref()
        .map(|(_, suffix)| suffix.clone())
        .unwrap_or_else(|| quote!(""));
    let mut extra_args = path_plan.args;

    if let Some((query_arg, _)) = query_plan {
        extra_args.push(query_arg);
    }

    let verb = format_ident!("{}", route.verb.to_string().to_uppercase());
    let base = quote!(Self::BASE);
    let subst = path_plan
        .subst
        .iter()
        .zip(&holes)
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
    let request = ClientMethodOverrideBody {
        bounds: quote!(C: #http_client_streaming + #decodes<#response>),
        ret: quote!(::core::result::Result<#http_response<#response>, #client_error<#http::StatusCode>>),
        body: quote! {{
            let __stream = ::core::convert::Into::<#stream_arg<#item>>::into(input).into_inner();
            let __body = #encode_stream::<#ndjson<()>, #item, _>(__stream);

            let __request = #http::Request::builder()
                .method(#http::Method::#verb)
                .uri(#uri)
                .header(
                    #http::header::CONTENT_TYPE,
                    <#ndjson<()> as #stream_encode<#item>>::CONTENT_TYPE,
                )
                .body(__body)
                .expect("client request is valid by construction");

            #http_client_streaming::send_stream(&self.0, __request).await
        }},
    };

    Some(ClientMethod {
        ident: method_ident.clone(),
        path: String::new(),
        capability: Capability::ClientStreaming,
        request: None,
        encode_as: None,
        req_item: Some(item),
        resp_item: None,
        response,
        error_ty: None,
        extra_args,
        request_envelope: None,
        request_builder: None,
        response_envelope: None,
        response_mapper: None,
        trailing_args: Vec::new(),
        attrs: Vec::new(),
        override_bounds: Some(request.bounds),
        override_ret: Some(request.ret),
        override_body: Some(request.body),
    })
}
