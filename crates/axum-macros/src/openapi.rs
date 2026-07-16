//! OpenAPI codegen for `#[dto]` (schema registration) and `#[handlers]` HTTP routes (operation
//! registration), gated on the macro crate's `openapi` feature.
//!
//! The whole spec model is utoipa's: `#[dto]` gains a `utoipa::ToSchema` derive, and each route
//! gets a generated `#[utoipa::path(..)]` attribute on a hidden marker fn — utoipa turns that into a
//! `__path_*` type implementing `utoipa::Path`. This module reads the route's already-classified
//! wire inputs (via [`crate::client::classify`]) to fill the attribute's `params` / `request_body` /
//! `responses`, so dependency-injected extractors never reach utoipa. Each generated item then
//! registers a closure into the runtime `OPENAPI_OPERATIONS` / `OPENAPI_SCHEMAS` link-time slices,
//! which the plugin folds into one document.
//!
//! **Gating rule:** the feature is decided here at expansion time; emitted tokens are concrete
//! (never a `cfg_attr(feature = ..)` that would leak the feature onto the user's crate).

use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{Attribute, DeriveInput, Ident, ImplItemFn, ReturnType, Type};

use overseerd_macros_core::paths::Paths;

use crate::client::{self, BodyKind};
use crate::route::RouteAttr;

/// Whether OpenAPI codegen is enabled for this build of the macro crate.
pub(crate) fn enabled() -> bool {
    cfg!(feature = "openapi")
}

/// The schema-registration tokens for a `#[dto]` type: a `ToSchema` derive plus a link-time entry
/// contributing the type's own component schema and those of its nested `#[dto]` fields. Emitted
/// only for a non-generic type — a concrete `static` cannot name a generic type; a generic DTO's
/// concrete instantiations are still referenced by the operations that use them.
///
/// Returns `(derive, registration)`: the derive is spliced next to the other `#[dto]` derives, the
/// registration is appended after the item.
pub(crate) fn dto_tokens(item: &DeriveInput, paths: &Paths) -> (TokenStream, TokenStream) {
    if !enabled() {
        return (quote!(), quote!());
    }

    // utoipa's derive/attribute macros emit bare `utoipa::` paths in their *output*, so — like
    // `tsify`/`wasm-bindgen` — it must be a direct dependency of the user's crate; reference it
    // absolutely as `::utoipa`, never through a re-export.
    let utoipa = quote!(::utoipa);
    let derive = quote!(#[derive(#utoipa::ToSchema)]);

    // A generic DTO cannot be registered as a concrete static; skip its registration (its concrete
    // uses are still pulled in as operation-referenced schemas).
    if !item.generics.params.is_empty() {
        return (derive, quote!());
    }

    let ident = &item.ident;
    let schema_entry = paths.plugin("SchemaEntry");
    let schemas_slice = paths.plugin("OPENAPI_SCHEMAS");
    let distributed_slice = paths.core("linkme::distributed_slice");
    let linkme_crate = paths.core("linkme");
    let register = format_ident!(
        "__OVERSEERD_OPENAPI_SCHEMA_{}",
        ident.to_string().to_uppercase()
    );

    // The whole schema surface is native + server-only (utoipa is not compiled for wasm), so gate
    // the registration the same way the runtime slices are.
    let registration = overseerd_macros_core::gate::native_only(quote! {
        const _: () = {
            #[#distributed_slice(#schemas_slice)]
            #[linkme(crate = #linkme_crate)]
            static #register: #schema_entry = |__schemas| {
                __schemas.push((
                    <#ident as #utoipa::ToSchema>::name().into_owned(),
                    <#ident as #utoipa::PartialSchema>::schema(),
                ));

                <#ident as #utoipa::ToSchema>::schemas(__schemas);
            };
        };
    });

    (derive, registration)
}

/// The operation-registration tokens for one HTTP route: a hidden marker fn carrying a generated
/// `#[utoipa::path(..)]` attribute (which utoipa lowers to a `__path_*` `utoipa::Path` type), plus a
/// link-time entry contributing `(full_path, methods, operation)` to the document. The full path
/// joins the controller `BASE` (read at runtime through the [`Controller`] trait) with the route's
/// relative path. `None` when OpenAPI is disabled.
///
/// `extra` is the raw token stream from a handler's `#[openapi(..)]` attribute, appended verbatim
/// into the `#[utoipa::path(..)]` arguments so a user can add tags, descriptions, or extra responses
/// without us parsing them.
pub(crate) fn operation_tokens(
    self_ty: &Type,
    self_ident: &Ident,
    method: &ImplItemFn,
    route: &RouteAttr,
    doc_attrs: &[Attribute],
    extra: Option<TokenStream>,
    paths: &Paths,
) -> Option<TokenStream> {
    if !enabled() {
        return None;
    }

    // See `dto_tokens`: utoipa must be a direct user dep; reference it as `::utoipa`.
    let utoipa = quote!(::utoipa);
    let controller_trait = paths.plugin("Controller");
    let operation_entry = paths.plugin("OperationEntry");
    let operations_slice = paths.plugin("OPENAPI_OPERATIONS");
    let join_base = paths.plugin("join_base");
    let distributed_slice = paths.core("linkme::distributed_slice");
    let linkme_crate = paths.core("linkme");

    let verb = &route.verb;
    let path = &route.path;
    let method_ident = &method.sig.ident;
    // The marker fn's name is utoipa's default `operationId`, so name it `{Controller}_{method}` —
    // readable and unique across controllers. A user overrides it with `#[openapi(operation_id =
    // ..)]`, which wins over this default. The fn lives inside an isolated `const _` block, so the
    // non-snake-case name pollutes no namespace.
    let marker_fn = format_ident!("{}_{}", self_ident, method_ident);
    let path_struct = format_ident!("__path_{}", marker_fn);
    let register = format_ident!(
        "__OVERSEERD_OPENAPI_OP_{}_{}",
        self_ident.to_string().to_uppercase(),
        method_ident.to_string().to_uppercase()
    );

    let arg_types: Vec<&Type> = method
        .sig
        .inputs
        .iter()
        .filter_map(|arg| match arg {
            syn::FnArg::Typed(typed) => Some(typed.ty.as_ref()),
            syn::FnArg::Receiver(_) => None,
        })
        .collect();

    let params = params_arg(path, &arg_types);
    let request_body = request_body_arg(&arg_types);
    let responses = responses_arg(&method.sig.output);
    let extra = extra.map(|tokens| quote!(, #tokens)).unwrap_or_default();

    Some(overseerd_macros_core::gate::native_only(quote! {
        const _: () = {
            #[#utoipa::path(#verb, path = #path #params #request_body, #responses #extra)]
            #(#doc_attrs)*
            #[allow(non_snake_case, dead_code)]
            fn #marker_fn() {}

            #[#distributed_slice(#operations_slice)]
            #[linkme(crate = #linkme_crate)]
            static #register: #operation_entry = || {
                let __relative = <#path_struct as #utoipa::Path>::path();
                let __full = #join_base(<#self_ty as #controller_trait>::BASE, &__relative);

                (
                    __full,
                    <#path_struct as #utoipa::Path>::methods(),
                    <#path_struct as #utoipa::Path>::operation(),
                )
            };
        };
    }))
}

/// The `, params((name = Type, Path), ..)` argument for the route's path holes, or empty when the
/// route has none. Types come from the `Path<T>` extractor (via [`client::hole_param_types`]);
/// absent, each hole is a `String`. Query parameters are not auto-documented (they would need a
/// `utoipa::IntoParams` impl that `#[dto]` cannot derive for every shape) — add them with
/// `#[openapi(params(..))]`.
fn params_arg(path: &syn::LitStr, arg_types: &[&Type]) -> TokenStream {
    let (_, holes) = client::parse_template(&path.value());

    if holes.is_empty() {
        return quote!();
    }

    let path_ty = client::classify(arg_types).and_then(|inputs| inputs.path_ty);
    let types = client::hole_param_types(&holes, path_ty);
    let entries = holes.iter().enumerate().zip(&types).map(|((i, hole), ty)| {
        let name = client::hole_ident(hole, i).to_string();

        quote!((#name = #ty, Path))
    });

    quote!(, params(#(#entries),*))
}

/// The `, request_body = T` argument for a `Json`/`Form` body, or empty for no body or a
/// wrapper-typed body (`Bytes`/`RawForm`/`Multipart`, whose schema is not a single `Dto`).
fn request_body_arg(arg_types: &[&Type]) -> TokenStream {
    let Some(inputs) = client::classify(arg_types) else {
        return quote!();
    };

    match inputs.body {
        Some(client::Body {
            kind: BodyKind::Json | BodyKind::Form,
            inner: Some(ty),
        }) => quote!(, request_body = #ty),

        _ => quote!(),
    }
}

/// The `responses(..)` argument. Documents a `200`; the body is the peeled response type
/// (`client::response_type` peels `Result`/`Json`, matching what the client decodes) — unless that
/// type is one of the [`Dto`](../overseerd_axum/trait.Dto.html) escape hatches that is not a
/// `utoipa::ToSchema` ([`undocumented_body`]), in which case the `200` is bodyless. This parallels
/// how those same shapes yield an uncallable typed client method: they carry no schema.
fn responses_arg(output: &ReturnType) -> TokenStream {
    let response = client::response_type(output);

    if undocumented_body(&response) {
        quote!(responses((status = 200)))
    } else {
        quote!(responses((status = 200, body = #response)))
    }
}

/// Whether a response type has no documentable schema and must yield a bodyless response: the unit
/// type, a borrowed `&T` (plaintext responses), or `http::StatusCode` (a status-only response).
/// These are exactly the `Dto` blanket-impl escape hatches that are not `utoipa::ToSchema`; keyed
/// syntactically (like the `Json`/`Result` peeling), since the macro cannot resolve trait impls.
fn undocumented_body(ty: &Type) -> bool {
    match ty {
        Type::Tuple(tuple) => tuple.elems.is_empty(),
        Type::Reference(_) => true,

        Type::Path(path) => path
            .path
            .segments
            .last()
            .is_some_and(|segment| segment.ident == "StatusCode"),

        _ => false,
    }
}

/// Finds and removes a `#[openapi(..)]` attribute from a handler method, returning its inner tokens
/// (the `..`) to be appended into the generated `#[utoipa::path(..)]`. A handler carries at most one.
pub(crate) fn take_openapi_attr(method: &mut ImplItemFn) -> syn::Result<Option<TokenStream>> {
    let Some(pos) = method
        .attrs
        .iter()
        .position(|attr| attr.path().is_ident("openapi"))
    else {
        return Ok(None);
    };

    let attr = method.attrs.remove(pos);

    match &attr.meta {
        syn::Meta::List(list) => Ok(Some(list.tokens.clone())),

        _ => Err(syn::Error::new_spanned(
            &attr,
            "#[openapi(..)] takes a parenthesized list of utoipa::path arguments, e.g. \
             #[openapi(tag = \"users\")]",
        )),
    }
}
