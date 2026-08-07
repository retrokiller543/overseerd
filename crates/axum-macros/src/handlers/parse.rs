use proc_macro2::TokenStream;
use syn::{FnArg, Ident, ImplItemFn, Type};

use overseerd_macros_core::client::ClientMethod;
use overseerd_macros_core::extend::ParseMethod;
use overseerd_macros_core::paths::Paths;

use super::AxumHandlers;
use super::args::{add_use_capture, take_stream_param};
use super::http::{HttpRouteContext, build_route};
use super::ws::{build_ws_route, resolve_message_reply, ws_payload_type};
use super::ws_client::{build_message_request_method, build_message_send_method};
use crate::{client, route};

impl ParseMethod for AxumHandlers {
    fn parse_method(&mut self, method: &mut ImplItemFn) -> syn::Result<Option<ClientMethod>> {
        if let Some(pos) = method.attrs.iter().position(route::is_message_attr) {
            return self.parse_message_method(method, pos);
        }

        let Some(pos) = method.attrs.iter().position(route::is_route_attr) else {
            return Ok(None);
        };
        self.parse_http_method(method, pos)
    }

    fn extra_client_tokens(
        &self,
        client_ident: &Ident,
        methods: &[ClientMethod],
        paths: &Paths,
    ) -> TokenStream {
        let backend = match &self.ws_protocol {
            Some(protocol) => Some(client::WasmBackend::Message {
                protocol: protocol.clone(),
            }),
            None => Some(client::WasmBackend::Http),
        };

        client::extra_client_tokens(
            client_ident,
            methods,
            &self.header_methods,
            self.wire_types.clone(),
            &self.response_types,
            backend,
            paths,
        )
    }
}

impl AxumHandlers {
    fn parse_message_method(
        &mut self,
        method: &mut ImplItemFn,
        pos: usize,
    ) -> syn::Result<Option<ClientMethod>> {
        let attr = method.attrs.remove(pos);
        let args = route::parse_message_attr(&attr)?;
        let destination = &args.destination;
        let cx = self
            .context
            .as_ref()
            .expect("AxumHandlers::parse_item runs before parse_method");
        let protocol = self.ws_protocol.as_ref().ok_or_else(|| {
            syn::Error::new_spanned(
                &attr,
                "a #[handlers] block containing #[message] methods requires `ws = P`",
            )
        })?;
        let is_request = resolve_message_reply(args.mode, &method.sig.output);
        let codec = self.resolve_ws_codec(protocol, &cx.paths);
        let spec = build_ws_route(
            &cx.self_ty,
            protocol,
            method,
            destination,
            &codec,
            is_request,
            &cx.paths,
        )?;
        let hint = if is_request {
            build_message_request_method(
                &method.sig.ident,
                method,
                destination,
                protocol,
                &codec,
                &cx.paths,
            )?
        } else {
            build_message_send_method(
                &method.sig.ident,
                method,
                destination,
                protocol,
                &codec,
                &cx.paths,
            )?
        };

        if let Some(payload) = ws_payload_type(method)? {
            self.wire_types.push(payload);
        }
        self.ws_routes.push(spec);
        Ok(hint)
    }

    fn parse_http_method(
        &mut self,
        method: &mut ImplItemFn,
        pos: usize,
    ) -> syn::Result<Option<ClientMethod>> {
        let attr = method.attrs.remove(pos);
        let route_attr = route::parse_route_attr(&attr)?;
        let openapi_extra = crate::openapi::take_openapi_attr(method)?;
        let openapi_docs: Vec<syn::Attribute> = method
            .attrs
            .iter()
            .filter(|attr| attr.path().is_ident("doc"))
            .cloned()
            .collect();
        let cx = self
            .context
            .as_ref()
            .expect("AxumHandlers::parse_item runs before parse_method");
        let stream_param = take_stream_param(method, &cx.paths)?;
        let stream_return = if stream_param.is_some() {
            None
        } else {
            client::classify_stream_return(&method.sig.output, route_attr.streamed, &cx.paths)
        };
        add_use_capture(&mut method.sig.output, &cx.capture);
        let arg_types: Vec<&Type> = method
            .sig
            .inputs
            .iter()
            .filter_map(|arg| match arg {
                FnArg::Typed(typed) => Some(typed.ty.as_ref()),
                FnArg::Receiver(_) => None,
            })
            .collect();
        let server_wrap = stream_return.as_ref().and_then(|s| s.server_wrap.as_ref());
        let stream_item = stream_return
            .as_ref()
            .and_then(|stream| stream.client.as_ref().map(|(_, item)| item));
        let in_result = stream_return.as_ref().is_some_and(|s| s.in_result);
        let analyzed_output = crate::http_analysis::output(
            method,
            &route_attr,
            server_wrap,
            stream_item,
            stream_return.is_some(),
            route_attr.streamed,
        );

        let hint = if let Some((index, item)) = &stream_param {
            let path_args: Vec<&Type> = arg_types
                .iter()
                .enumerate()
                .filter_map(|(i, ty)| (i != *index).then_some(*ty))
                .collect();
            client::build_client_stream_method(
                &method.sig.ident,
                &route_attr,
                &path_args,
                item.clone(),
                &method.sig.output,
                &cx.paths,
            )
        } else if let Some(stream) = &stream_return {
            match &stream.client {
                Some((wrapper_unit, item)) => client::build_stream_client_method(
                    &method.sig.ident,
                    &route_attr,
                    &arg_types,
                    wrapper_unit.clone(),
                    item.clone(),
                    &cx.paths,
                ),
                None => None,
            }
        } else {
            match client::build_client_method(
                &cx.self_ident,
                &method.sig.ident,
                &route_attr,
                &arg_types,
                &method.sig.output,
                &analyzed_output.responses,
                &cx.paths,
            ) {
                Some(methods) => {
                    self.header_methods.push(methods.with_headers);
                    if !methods.declaration.is_empty() {
                        self.response_types.push(methods.declaration);
                    }
                    Some(methods.base)
                }
                None => None,
            }
        };

        if stream_param.is_none() && stream_return.is_none() {
            client::collect_wire_types(&arg_types, &method.sig.output, &mut self.wire_types);
            if let Some(returns) = &route_attr.returns {
                self.wire_types.push(returns.clone());
            }
            if let Some(response) = &route_attr.response {
                self.wire_types.push(response.clone());
            }
            self.wire_types
                .extend(
                    analyzed_output
                        .responses
                        .alternatives
                        .iter()
                        .filter_map(|response| match &response.body {
                            crate::http_analysis::ResponseBody::Typed(body) => {
                                Some((**body).clone())
                            }
                            _ => None,
                        }),
                );
        }

        let openapi_op = if stream_param.is_none() && stream_return.is_none() {
            crate::openapi::operation_tokens(
                &cx.self_ty,
                &cx.self_ident,
                method,
                &route_attr,
                &openapi_docs,
                openapi_extra,
                &cx.paths,
            )
        } else {
            None
        };
        let spec = build_route(
            &cx.self_ty,
            method,
            &route_attr,
            HttpRouteContext {
                stream_param: stream_param.as_ref(),
                server_wrap,
                stream_item,
                is_streaming: stream_return.is_some(),
                in_result,
            },
            &cx.paths,
        )?;
        self.routes.push(spec);
        if let Some(op) = openapi_op {
            self.openapi_ops.push(op);
        }
        Ok(hint)
    }
}
