use proc_macro2::TokenStream;
use quote::quote;
use syn::{Ident, ImplItemFn, LitStr, parse_quote};

use overseerd_macros_core::client::ClientMethod;
use overseerd_macros_core::paths::Paths;

use super::ws::ws_payload_type;
use crate::client;

pub(super) fn build_message_send_method(
    method_ident: &Ident,
    method: &ImplItemFn,
    destination: &LitStr,
    protocol: &syn::Path,
    codec: &TokenStream,
    paths: &Paths,
) -> syn::Result<Option<ClientMethod>> {
    let payload = ws_payload_type(method)?;
    let client_error = paths.client("ClientError");
    let message_send = paths.plugin("client::MessageSend");
    let topic_client_protocol = paths.plugin("MessagingClientProtocol");
    let topic_codec = paths.plugin("TopicCodec");
    let (request, encode_body) = match payload {
        Some(ty) => (
            Some(ty),
            quote!(
                <#codec as #topic_codec<#protocol>>::encode(&request)
                    .map_err(|__e| #client_error::Encode(::std::string::ToString::to_string(&__e)))?
            ),
        ),
        None => (
            None,
            quote!(
                <#codec as #topic_codec<#protocol>>::encode(&())
                    .map_err(|__e| #client_error::Encode(::std::string::ToString::to_string(&__e)))?
            ),
        ),
    };

    Ok(Some(ClientMethod {
        ident: method_ident.clone(),
        path: String::new(),
        capability: overseerd_macros_core::client::Capability::Unary,
        request,
        encode_as: None,
        req_item: None,
        resp_item: None,
        response: parse_quote!(()),
        error_ty: None,
        extra_args: Vec::new(),
        request_envelope: None,
        request_builder: None,
        response_envelope: None,
        response_mapper: None,
        trailing_args: Vec::new(),
        attrs: Vec::new(),
        override_bounds: Some(quote!( C: #message_send<#protocol> )),
        override_ret: Some(quote!(
            ::core::result::Result<(), #client_error<<#protocol as #topic_client_protocol>::Status>>
        )),
        override_body: Some(quote!({
            let __body = #encode_body;
            <C as #message_send<#protocol>>::send(&self.0, #destination, __body).await
        })),
    }))
}

pub(super) fn build_message_request_method(
    method_ident: &Ident,
    method: &ImplItemFn,
    destination: &LitStr,
    protocol: &syn::Path,
    codec: &TokenStream,
    paths: &Paths,
) -> syn::Result<Option<ClientMethod>> {
    let payload = ws_payload_type(method)?;
    let response = client::response_type(&method.sig.output);
    let client_error = paths.client("ClientError");
    let message_request = paths.plugin("client::MessageRequest");
    let topic_client_protocol = paths.plugin("MessagingClientProtocol");
    let topic_codec = paths.plugin("TopicCodec");
    let (request, encode_body) = match payload {
        Some(ty) => (
            Some(ty),
            quote!(
                <#codec as #topic_codec<#protocol>>::encode(&request)
                    .map_err(|__e| #client_error::Encode(::std::string::ToString::to_string(&__e)))?
            ),
        ),
        None => (
            None,
            quote!(
                <#codec as #topic_codec<#protocol>>::encode(&())
                    .map_err(|__e| #client_error::Encode(::std::string::ToString::to_string(&__e)))?
            ),
        ),
    };

    Ok(Some(ClientMethod {
        ident: method_ident.clone(),
        path: String::new(),
        capability: overseerd_macros_core::client::Capability::Unary,
        request,
        encode_as: None,
        req_item: None,
        resp_item: None,
        response: response.clone(),
        error_ty: None,
        extra_args: Vec::new(),
        request_envelope: None,
        request_builder: None,
        response_envelope: None,
        response_mapper: None,
        trailing_args: Vec::new(),
        attrs: Vec::new(),
        override_bounds: Some(quote!( C: #message_request<#protocol> )),
        override_ret: Some(quote!(
            ::core::result::Result<#response, #client_error<<#protocol as #topic_client_protocol>::Status>>
        )),
        override_body: Some(quote!({
            let __body = #encode_body;
            let __reply =
                <C as #message_request<#protocol>>::request(&self.0, #destination, __body).await?;

            <#codec as #topic_codec<#protocol>>::decode::<#response>(__reply)
                .map_err(|__e| #client_error::Decode(::std::string::ToString::to_string(&__e)))
        })),
    }))
}
