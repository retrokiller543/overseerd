//! Cheap, syntax-only HTTP route metadata analysis shared by tooling and OpenAPI.

use overseerd_macros_core::attr::{first_type_arg, type_name};
use quote::ToTokens;
use syn::spanned::Spanned;
use syn::{Ident, ImplItemFn, LitStr, ReturnType, Type};

use crate::client;
use crate::route::{ResponseCase, RouteAttr};

pub(crate) struct Input {
    pub name: String,
    pub source: &'static str,
    pub ty: Type,
}

pub(crate) struct Output {
    pub ty: Option<Type>,
    pub declared: Type,
    pub shape: &'static str,
    pub responses: Vec<ResponseCase>,
}

pub(crate) fn input(ty: &Type) -> (&'static str, Type) {
    let ty = peel_input_combinator(ty);

    for (wrapper, source) in [
        ("Path", "Path"),
        ("Query", "Query"),
        ("TypedHeader", "Header"),
        ("Json", "JsonBody"),
        ("TypedJson", "JsonBody"),
        ("JsonDeserializer", "JsonBody"),
        ("Protobuf", "BytesBody"),
        ("Form", "FormBody"),
        ("Inject", "Injected"),
    ] {
        if let Some(inner) = first_type_arg(ty, wrapper) {
            return (source, inner);
        }
    }

    let source = match type_name(ty).map(Ident::to_string).as_deref() {
        Some("HeaderMap") => "Header",
        Some("Bytes") => "BytesBody",
        Some("RawForm") => "RawFormBody",
        Some("Multipart") => "MultipartBody",
        Some("JsonLines") => "Stream",
        _ => "Context",
    };

    (source, ty.clone())
}

fn peel_input_combinator(ty: &Type) -> &Type {
    let mut current = ty;

    loop {
        let next = match type_name(current).map(Ident::to_string).as_deref() {
            Some("Cached" | "WithRejection") => first_type_arg_ref(current),
            _ => None,
        };

        match next {
            Some(inner) => current = inner,
            None => return current,
        }
    }
}

fn first_type_arg_ref(ty: &Type) -> Option<&Type> {
    let Type::Path(path) = ty else {
        return None;
    };
    let segment = path.path.segments.last()?;
    let syn::PathArguments::AngleBracketed(arguments) = &segment.arguments else {
        return None;
    };

    arguments.args.iter().find_map(|argument| match argument {
        syn::GenericArgument::Type(ty) => Some(ty),
        _ => None,
    })
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
        _ => Some(response),
    };

    Output {
        ty,
        declared,
        shape,
        responses: responses(method, route),
    }
}

pub(crate) fn responses(method: &ImplItemFn, route: &RouteAttr) -> Vec<ResponseCase> {
    let mut responses = route.responses.clone();

    collect_statements(&method.block.stmts, &mut responses);

    responses.sort_by_key(|response| response.status);
    responses.dedup_by(|left, right| {
        left.status == right.status
            && left
                .body
                .as_ref()
                .map(ToTokens::to_token_stream)
                .map(|tokens| tokens.to_string())
                == right
                    .body
                    .as_ref()
                    .map(ToTokens::to_token_stream)
                    .map(|tokens| tokens.to_string())
            && left.redirect.as_ref().map(LitStr::value)
                == right.redirect.as_ref().map(LitStr::value)
    });

    responses
}

fn collect_response_expressions(expression: &syn::Expr, responses: &mut Vec<ResponseCase>) {
    match expression {
        syn::Expr::Return(returned) => {
            if let Some(expression) = &returned.expr {
                collect_response_expressions(expression, responses);
            }
        }
        syn::Expr::Block(block) => collect_statements(&block.block.stmts, responses),
        syn::Expr::If(branch) => {
            collect_statements(&branch.then_branch.stmts, responses);

            if let Some((_, otherwise)) = &branch.else_branch {
                collect_response_expressions(otherwise, responses);
            }
        }
        syn::Expr::Match(branches) => {
            for arm in &branches.arms {
                collect_response_expressions(&arm.body, responses);
            }
        }
        syn::Expr::Tuple(tuple) => {
            if let Some(status) = tuple.elems.first().and_then(status_expression) {
                responses.push(ResponseCase {
                    status,
                    body: None,
                    redirect: None,
                    span: tuple.span(),
                });
            }
        }
        syn::Expr::Call(call) => {
            if let Some((status, target)) = redirect_expression(call) {
                responses.push(ResponseCase {
                    status,
                    body: None,
                    redirect: Some(target),
                    span: call.span(),
                });
            }
        }
        syn::Expr::MethodCall(call) => {
            if call.method == "body"
                && let syn::Expr::MethodCall(status_call) = call.receiver.as_ref()
                && status_call.method == "status"
                && let Some(status) = status_call.args.first().and_then(status_expression)
            {
                responses.push(ResponseCase {
                    status,
                    body: None,
                    redirect: None,
                    span: call.span(),
                });
            }

            collect_response_expressions(&call.receiver, responses);
        }
        _ => {}
    }
}

fn collect_statements(statements: &[syn::Stmt], responses: &mut Vec<ResponseCase>) {
    for statement in statements {
        match statement {
            syn::Stmt::Expr(expression, _) => collect_response_expressions(expression, responses),
            syn::Stmt::Local(local) => {
                if let Some(initializer) = &local.init {
                    collect_response_expressions(&initializer.expr, responses);
                }
            }
            _ => {}
        }
    }
}

fn status_expression(expression: &syn::Expr) -> Option<u16> {
    let syn::Expr::Path(path) = expression else {
        return None;
    };
    let status = path.path.segments.last()?.ident.to_string();

    Some(match status.as_str() {
        "OK" => 200,
        "CREATED" => 201,
        "ACCEPTED" => 202,
        "NO_CONTENT" => 204,
        "MOVED_PERMANENTLY" => 301,
        "FOUND" => 302,
        "SEE_OTHER" => 303,
        "TEMPORARY_REDIRECT" => 307,
        "PERMANENT_REDIRECT" => 308,
        "BAD_REQUEST" => 400,
        "UNAUTHORIZED" => 401,
        "FORBIDDEN" => 403,
        "NOT_FOUND" => 404,
        "CONFLICT" => 409,
        "UNPROCESSABLE_ENTITY" => 422,
        "TOO_MANY_REQUESTS" => 429,
        "INTERNAL_SERVER_ERROR" => 500,
        "BAD_GATEWAY" => 502,
        "SERVICE_UNAVAILABLE" => 503,
        "GATEWAY_TIMEOUT" => 504,
        _ => return None,
    })
}

fn redirect_expression(call: &syn::ExprCall) -> Option<(u16, LitStr)> {
    let syn::Expr::Path(path) = call.func.as_ref() else {
        return None;
    };
    let method = path.path.segments.last()?.ident.to_string();

    if !path
        .path
        .segments
        .iter()
        .any(|segment| segment.ident == "Redirect")
    {
        return None;
    }

    let syn::Expr::Lit(syn::ExprLit {
        lit: syn::Lit::Str(target),
        ..
    }) = call.args.first()?
    else {
        return None;
    };
    let status = match method.as_str() {
        "to" => 303,
        "temporary" => 307,
        "permanent" => 308,
        _ => return None,
    };

    Some((status, target.clone()))
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
