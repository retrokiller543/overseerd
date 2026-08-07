mod response;

use overseerd_macros_core::client::{Capability, ClientMethod};
use overseerd_macros_core::paths::Paths;
use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{Ident, ReturnType, Type};

use self::response::{ResponsePlanInput, response_plan};
use super::inputs::{Body, QueryInput, classify};
use super::path::{PathPlan, parse_template, plan_path};
use super::request::{body_param_type, body_parts, query_parts, request_builder};
use crate::route::RouteAttr;

/// The clean unary method and its per-call-header sibling.
pub(crate) struct UnaryMethods {
    pub(crate) base: ClientMethod,
    pub(crate) with_headers: ClientMethod,
    pub(crate) declaration: TokenStream,
}

/// Builds both client methods for one unary route.
pub(crate) fn build_client_method(
    controller_ident: &Ident,
    method_ident: &Ident,
    route: &RouteAttr,
    arg_types: &[&Type],
    output: &ReturnType,
    responses: &crate::http_analysis::ResponseContract,
    paths: &Paths,
) -> Option<UnaryMethods> {
    if route.streamed {
        return None;
    }

    let inputs = classify(arg_types)?;
    let (fmt, holes) = parse_template(&route.path.value());
    let path_plan = plan_path(&holes, inputs.path_ty);

    Some(assemble(
        method_ident,
        controller_ident,
        route,
        fmt,
        holes,
        path_plan,
        inputs.query,
        inputs.body,
        output,
        responses,
        paths,
    ))
}

#[allow(clippy::too_many_arguments)]
fn assemble(
    method_ident: &Ident,
    controller_ident: &Ident,
    route: &RouteAttr,
    fmt: String,
    holes: Vec<String>,
    path_plan: PathPlan,
    query: Option<QueryInput>,
    body: Option<Body>,
    output: &ReturnType,
    responses: &crate::http_analysis::ResponseContract,
    paths: &Paths,
) -> UnaryMethods {
    let http = paths.plugin("http");
    let http_response = paths.plugin("client::HttpResponse");
    let request = body_param_type(&body, paths);
    let (_body_param, encode_ty, body_value) = body_parts(&body, paths);
    let encode_as = request.as_ref().map(|_| encode_ty.clone());
    let query_plan = query_parts(&query, paths);
    let query_suffix = query_plan
        .as_ref()
        .map(|(_, suffix)| suffix.clone())
        .unwrap_or_else(|| quote!(""));
    let mut extra_args = path_plan.args;

    if let Some((query_arg, _)) = query_plan {
        extra_args.push(query_arg);
    }

    let header_builder = request_builder(
        route,
        &fmt,
        &holes,
        &path_plan.subst,
        &query_suffix,
        &encode_ty,
        &body_value,
        true,
        paths,
    );
    let response_plan = response_plan(ResponsePlanInput {
        method: method_ident,
        controller: controller_ident,
        route,
        output,
        contract: responses,
        encode_ty: &encode_ty,
        request: &header_builder,
        paths,
    });
    let response = response_plan.ty.clone();
    let request_envelope = Some(quote!(#http::Request<#encode_ty>));
    let response_envelope = Some(quote!(#http_response<#response>));
    let with_ident = format_ident!("{}_with_headers", method_ident);
    let mut forward_args = extra_args
        .iter()
        .map(|(name, _)| quote!(#name))
        .collect::<Vec<_>>();

    if request.is_some() {
        forward_args.push(quote!(request));
    }

    let with_headers = ClientMethod {
        ident: with_ident.clone(),
        path: String::new(),
        capability: Capability::Unary,
        request: request.clone(),
        encode_as: encode_as.clone(),
        req_item: None,
        resp_item: None,
        response: response.clone(),
        error_ty: None,
        extra_args: extra_args.clone(),
        request_envelope: request_envelope.clone(),
        request_builder: Some(header_builder),
        response_envelope: response_envelope.clone(),
        response_mapper: None,
        trailing_args: vec![(
            format_ident!("headers"),
            quote!(::core::option::Option<#http::HeaderMap>),
        )],
        attrs: Vec::new(),
        override_bounds: Some(response_plan.bounds.clone()),
        override_ret: Some(response_plan.ret.clone()),
        override_body: Some(response_plan.body.clone()),
    };
    let base = ClientMethod {
        ident: method_ident.clone(),
        path: String::new(),
        capability: Capability::Unary,
        request,
        encode_as,
        req_item: None,
        resp_item: None,
        response,
        error_ty: None,
        extra_args,
        request_envelope,
        request_builder: None,
        response_envelope,
        response_mapper: None,
        trailing_args: Vec::new(),
        attrs: vec![quote!(#[inline(always)])],
        override_bounds: Some(response_plan.bounds),
        override_ret: None,
        override_body: Some(quote! {
            self.#with_ident(#(#forward_args,)* ::core::option::Option::None).await
        }),
    };

    UnaryMethods {
        base,
        with_headers,
        declaration: response_plan.declaration,
    }
}
