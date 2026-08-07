//! Cheap, syntax-only HTTP route metadata analysis shared by clients, tooling, and OpenAPI.

mod input;
mod response;

use syn::{Ident, ImplItemFn, LitStr, ReturnType, Type};
use upwell_macros_core::attr::{first_type_arg, type_name};

use crate::client;
use crate::route::{ResponseCase, RouteAttr};

pub(crate) struct Input {
    pub name: String,
    pub source: &'static str,
    pub ty: Type,
}

#[derive(Clone)]
pub(crate) struct Output {
    pub ty: Option<Type>,
    pub declared: Type,
    pub shape: &'static str,
    pub responses: ResponseContract,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ResponseOrigin {
    Explicit,
    Inferred,
    Conventional,
    Opaque,
}

#[derive(Clone)]
pub(crate) struct ResponseContract {
    pub origin: ResponseOrigin,
    pub alternatives: Vec<ResponseAlternative>,
}

#[derive(Clone)]
pub(crate) struct ResponseAlternative {
    pub status: u16,
    pub body: ResponseBody,
    pub redirect: Option<LitStr>,
}

#[derive(Clone)]
pub(crate) enum ResponseBody {
    Empty,
    Typed(Box<Type>),
    Opaque,
}

pub(crate) fn input(ty: &Type) -> (&'static str, Type) {
    input::classify(ty)
}

pub(crate) fn output(
    method: &ImplItemFn,
    route: &RouteAttr,
    server_wrap: Option<&client::ServerWrap>,
    stream_item: Option<&Type>,
    is_streaming: bool,
    streamed: bool,
) -> Output {
    let signature = &method.sig.output;
    let returns = route.returns.as_ref();
    let declared = match signature {
        ReturnType::Default => syn::parse_quote!(()),
        ReturnType::Type(_, ty) => (**ty).clone(),
    };
    let response = returns
        .cloned()
        .unwrap_or_else(|| client::response_type(signature));
    let shape = match server_wrap {
        Some(client::ServerWrap::Ndjson) => "NdjsonStream",
        Some(client::ServerWrap::RawU8) => "RawStream",
        None if is_streaming => explicit_stream_shape(signature),
        None if streamed => "CustomStream",
        None if returns.is_none() && client::is_opaque_response(&response) => "Opaque",
        None => "Unary",
    };
    let ty = match shape {
        "NdjsonStream" | "RawStream" => stream_item.cloned(),
        "Opaque" | "CustomStream" => None,
        _ => Some(response.clone()),
    };

    Output {
        ty,
        declared,
        shape,
        responses: if is_streaming || streamed {
            ResponseContract {
                origin: ResponseOrigin::Opaque,
                alternatives: Vec::new(),
            }
        } else {
            response_contract(method, route, &response)
        },
    }
}

pub(crate) fn response_contract(
    method: &ImplItemFn,
    route: &RouteAttr,
    conventional: &Type,
) -> ResponseContract {
    if !route.responses.is_empty() {
        return ResponseContract {
            origin: ResponseOrigin::Explicit,
            alternatives: route.responses.iter().map(explicit_response).collect(),
        };
    }

    let alternatives = response::infer_block(&method.block);

    if !alternatives.is_empty() {
        return ResponseContract {
            origin: ResponseOrigin::Inferred,
            alternatives: response::normalize(alternatives),
        };
    }

    if !client::is_opaque_response(conventional) {
        return ResponseContract {
            origin: ResponseOrigin::Conventional,
            alternatives: vec![ResponseAlternative {
                status: 200,
                body: if is_unit(conventional) {
                    ResponseBody::Empty
                } else {
                    ResponseBody::Typed(Box::new(conventional.clone()))
                },
                redirect: None,
            }],
        };
    }

    ResponseContract {
        origin: ResponseOrigin::Opaque,
        alternatives: Vec::new(),
    }
}

fn explicit_response(response: &ResponseCase) -> ResponseAlternative {
    ResponseAlternative {
        status: response.status,
        body: response
            .body
            .clone()
            .map(Box::new)
            .map(ResponseBody::Typed)
            .unwrap_or(ResponseBody::Empty),
        redirect: response.redirect.clone(),
    }
}

fn is_unit(ty: &Type) -> bool {
    matches!(ty, Type::Tuple(tuple) if tuple.elems.is_empty())
}

fn explicit_stream_shape(output: &ReturnType) -> &'static str {
    let ReturnType::Type(_, ty) = output else {
        return "CustomStream";
    };
    let inner = first_type_arg(ty, "Result").unwrap_or_else(|| (**ty).clone());

    match type_name(&inner).map(Ident::to_string).as_deref() {
        Some("Ndjson") => "NdjsonStream",
        Some("RawStream") => "RawStream",
        _ => "CustomStream",
    }
}

#[cfg(test)]
mod tests;
