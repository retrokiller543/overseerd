use std::collections::BTreeMap;

use quote::ToTokens;
use syn::{Expr, LitStr, Type};

use super::{ResponseAlternative, ResponseBody};

pub(super) fn infer_block(block: &syn::Block) -> Vec<ResponseAlternative> {
    let mut bindings = BTreeMap::new();
    let mut alternatives = Vec::new();

    for (index, statement) in block.stmts.iter().enumerate() {
        match statement {
            syn::Stmt::Local(local) => {
                if let (syn::Pat::Ident(ident), Some(initializer)) = (&local.pat, &local.init)
                    && ident.by_ref.is_none()
                    && ident.mutability.is_none()
                    && ident.subpat.is_none()
                {
                    bindings.insert(ident.ident.to_string(), (*initializer.expr).clone());
                }

                if let Some(initializer) = &local.init {
                    collect_explicit_returns(&initializer.expr, &bindings, &mut alternatives);
                }
            }
            syn::Stmt::Expr(expression, semicolon) => {
                collect_explicit_returns(expression, &bindings, &mut alternatives);

                if index + 1 == block.stmts.len() && semicolon.is_none() {
                    collect_terminal(expression, &bindings, &mut alternatives, 0);
                }
            }
            _ => {}
        }
    }

    alternatives
}

fn collect_explicit_returns(
    expression: &Expr,
    bindings: &BTreeMap<String, Expr>,
    alternatives: &mut Vec<ResponseAlternative>,
) {
    match expression {
        Expr::Return(returned) => {
            if let Some(expression) = &returned.expr {
                collect_terminal(expression, bindings, alternatives, 0);
            }
        }
        Expr::Block(block) => collect_returns_in_block(&block.block, bindings, alternatives),
        Expr::If(branch) => {
            collect_returns_in_block(&branch.then_branch, bindings, alternatives);
            if let Some((_, otherwise)) = &branch.else_branch {
                collect_explicit_returns(otherwise, bindings, alternatives);
            }
        }
        Expr::Match(branches) => {
            for arm in &branches.arms {
                collect_explicit_returns(&arm.body, bindings, alternatives);
            }
        }
        _ => {}
    }
}

fn collect_returns_in_block(
    block: &syn::Block,
    outer: &BTreeMap<String, Expr>,
    alternatives: &mut Vec<ResponseAlternative>,
) {
    let mut bindings = outer.clone();

    for statement in &block.stmts {
        match statement {
            syn::Stmt::Local(local) => {
                if let (syn::Pat::Ident(ident), Some(initializer)) = (&local.pat, &local.init)
                    && ident.mutability.is_none()
                {
                    bindings.insert(ident.ident.to_string(), (*initializer.expr).clone());
                }
            }
            syn::Stmt::Expr(expression, _) => {
                collect_explicit_returns(expression, &bindings, alternatives)
            }
            _ => {}
        }
    }
}

fn collect_terminal(
    expression: &Expr,
    bindings: &BTreeMap<String, Expr>,
    alternatives: &mut Vec<ResponseAlternative>,
    depth: usize,
) {
    if depth > 16 {
        return;
    }

    let expression = unwrap_transparent(expression);

    match expression {
        Expr::Path(path) => {
            if path.path.segments.len() == 1
                && let Some(bound) = bindings.get(&path.path.segments[0].ident.to_string())
            {
                collect_terminal(bound, bindings, alternatives, depth + 1);
            } else if let Some(status) = status_expression(expression) {
                alternatives.push(ResponseAlternative {
                    status,
                    body: ResponseBody::Empty,
                    redirect: None,
                });
            }
        }
        Expr::Block(block) => alternatives.extend(infer_block(&block.block)),
        Expr::If(branch) => {
            alternatives.extend(infer_block(&branch.then_branch));
            if let Some((_, otherwise)) = &branch.else_branch {
                collect_terminal(otherwise, bindings, alternatives, depth + 1);
            }
        }
        Expr::Match(branches) => {
            for arm in &branches.arms {
                collect_terminal(&arm.body, bindings, alternatives, depth + 1);
            }
        }
        Expr::Tuple(tuple) => {
            if let Some(status) = tuple.elems.first().and_then(status_expression) {
                let body = tuple
                    .elems
                    .last()
                    .map(|body| infer_body(body, bindings, depth + 1))
                    .unwrap_or(ResponseBody::Empty);
                alternatives.push(ResponseAlternative {
                    status,
                    body,
                    redirect: None,
                });
            }
        }
        Expr::Call(call) => {
            if let Some((status, target)) = redirect_expression(call) {
                alternatives.push(ResponseAlternative {
                    status,
                    body: ResponseBody::Empty,
                    redirect: Some(target),
                });
            }
        }
        Expr::MethodCall(call) => {
            if let Some((status, body)) = response_builder(call, bindings, depth + 1) {
                alternatives.push(ResponseAlternative {
                    status,
                    body,
                    redirect: None,
                });
            }
        }
        _ => {}
    }
}

fn response_builder(
    call: &syn::ExprMethodCall,
    bindings: &BTreeMap<String, Expr>,
    depth: usize,
) -> Option<(u16, ResponseBody)> {
    if call.method != "body" {
        return None;
    }

    let status = builder_status(&call.receiver)?;
    let body = call
        .args
        .first()
        .map(|body| infer_body(body, bindings, depth))
        .unwrap_or(ResponseBody::Empty);

    Some((status, body))
}

fn builder_status(expression: &Expr) -> Option<u16> {
    let expression = unwrap_transparent(expression);

    match expression {
        Expr::MethodCall(call) if call.method == "status" => {
            call.args.first().and_then(status_expression)
        }
        Expr::MethodCall(call) => builder_status(&call.receiver),
        _ => None,
    }
}

fn infer_body(expression: &Expr, bindings: &BTreeMap<String, Expr>, depth: usize) -> ResponseBody {
    if depth > 16 {
        return ResponseBody::Opaque;
    }

    let expression = unwrap_transparent(expression);

    match expression {
        Expr::Path(path) if path.path.segments.len() == 1 => bindings
            .get(&path.path.segments[0].ident.to_string())
            .map(|bound| infer_body(bound, bindings, depth + 1))
            .unwrap_or(ResponseBody::Opaque),
        Expr::Call(call) => {
            let name = call_path_name(&call.func);

            match name.as_deref() {
                Some("Json") => call
                    .args
                    .first()
                    .and_then(expression_type)
                    .map(Box::new)
                    .map(ResponseBody::Typed)
                    .unwrap_or(ResponseBody::Opaque),
                Some("new" | "from_utf8") => call
                    .args
                    .first()
                    .map(|inner| infer_body(inner, bindings, depth + 1))
                    .unwrap_or(ResponseBody::Opaque),
                Some("empty") => ResponseBody::Empty,
                _ => ResponseBody::Opaque,
            }
        }
        Expr::MethodCall(call) if call.method == "encode" => {
            infer_body(&call.receiver, bindings, depth + 1)
        }
        Expr::Tuple(tuple) if tuple.elems.is_empty() => ResponseBody::Empty,
        _ => ResponseBody::Opaque,
    }
}

fn unwrap_transparent(mut expression: &Expr) -> &Expr {
    loop {
        expression = match expression {
            Expr::Paren(paren) => &paren.expr,
            Expr::Group(group) => &group.expr,
            Expr::MethodCall(call)
                if matches!(
                    call.method.to_string().as_str(),
                    "expect" | "unwrap" | "into_response"
                ) =>
            {
                &call.receiver
            }
            _ => return expression,
        };
    }
}

fn expression_type(expression: &Expr) -> Option<Type> {
    match expression {
        Expr::Struct(value) => Some(Type::Path(syn::TypePath {
            qself: None,
            path: value.path.clone(),
        })),
        Expr::Call(call) => {
            let Expr::Path(path) = call.func.as_ref() else {
                return None;
            };
            Some(Type::Path(syn::TypePath {
                qself: None,
                path: path.path.clone(),
            }))
        }
        _ => None,
    }
}

fn call_path_name(expression: &Expr) -> Option<String> {
    let Expr::Path(path) = expression else {
        return None;
    };
    Some(path.path.segments.last()?.ident.to_string())
}

pub(super) fn normalize(mut alternatives: Vec<ResponseAlternative>) -> Vec<ResponseAlternative> {
    alternatives.sort_by_key(|response| response.status);
    let mut normalized: Vec<ResponseAlternative> = Vec::new();

    for alternative in alternatives {
        if let Some(existing) = normalized
            .iter_mut()
            .find(|existing| existing.status == alternative.status)
        {
            if !same_body(&existing.body, &alternative.body)
                || existing.redirect.as_ref().map(LitStr::value)
                    != alternative.redirect.as_ref().map(LitStr::value)
            {
                existing.body = ResponseBody::Opaque;
                existing.redirect = None;
            }
        } else {
            normalized.push(alternative);
        }
    }

    normalized
}

fn same_body(left: &ResponseBody, right: &ResponseBody) -> bool {
    match (left, right) {
        (ResponseBody::Empty, ResponseBody::Empty)
        | (ResponseBody::Opaque, ResponseBody::Opaque) => true,
        (ResponseBody::Typed(left), ResponseBody::Typed(right)) => {
            left.to_token_stream().to_string() == right.to_token_stream().to_string()
        }
        _ => false,
    }
}

fn status_expression(expression: &Expr) -> Option<u16> {
    let Expr::Path(path) = expression else {
        return None;
    };
    let segments = path.path.segments.iter().collect::<Vec<_>>();

    if segments.len() < 2 || segments[segments.len() - 2].ident != "StatusCode" {
        return None;
    }

    Some(match segments.last()?.ident.to_string().as_str() {
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
    let Expr::Path(path) = call.func.as_ref() else {
        return None;
    };
    let segments = path.path.segments.iter().collect::<Vec<_>>();

    if segments.len() < 2 || segments[segments.len() - 2].ident != "Redirect" {
        return None;
    }

    let method = segments.last()?.ident.to_string();
    let Expr::Lit(syn::ExprLit {
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
