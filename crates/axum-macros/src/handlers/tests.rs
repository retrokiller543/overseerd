use overseerd_macros_core::paths::Paths;
use quote::quote;
use syn::{ImplItemFn, ReturnType, parse_quote};

use super::{
    AxumHandlers, HandlerContext, build_message_request_method, build_message_send_method,
    build_ws_route, message_success_value, resolve_message_reply,
};
use crate::route::MessageMode;

fn paths() -> Paths {
    Paths::new(parse_quote!(::core_crate), parse_quote!(::plugin_crate))
}

fn output(tokens: proc_macro2::TokenStream) -> String {
    tokens.to_string()
}

#[test]
fn inferred_modes_use_peeled_success_type() {
    let unit: ReturnType = parse_quote!(-> Result<(), Failure>);
    let value: ReturnType = parse_quote!(-> Result<Reply, Failure>);

    assert!(!resolve_message_reply(MessageMode::Infer, &unit));
    assert!(resolve_message_reply(MessageMode::Infer, &value));
    assert!(resolve_message_reply(MessageMode::Request, &unit));
    assert!(!resolve_message_reply(MessageMode::Send, &value));
}

#[test]
fn result_success_normalization_maps_application_errors() {
    let unit: ReturnType = parse_quote!(-> Result<(), Failure>);
    let value: ReturnType = parse_quote!(-> Result<Json<Reply>, Failure>);
    let error = parse_quote!(::plugin_crate::WsDispatchError);

    let (unit_ty, unit_value) = message_success_value(&unit, false, &error);
    let (value_ty, value_value) = message_success_value(&value, true, &error);
    let unit_tokens = output(quote!(#unit_value));
    let value_tokens = output(quote!(#value_value));

    assert_eq!(output(quote!(#unit_ty)), "()");
    assert_eq!(output(quote!(#value_ty)), "Reply");
    assert!(unit_tokens.contains("WsDispatchError :: Application"));
    assert!(value_tokens.contains("WsDispatchError :: Application"));
    assert!(value_tokens.ends_with(". 0"));
    assert!(!unit_tokens.contains("WsDispatchError :: Encode"));
}

#[test]
fn server_send_normalizes_result_before_ws_respond() {
    let method: ImplItemFn = parse_quote! {
        async fn send(&self, payload: Payload) -> Result<(), Failure> {
            Ok(())
        }
    };
    let protocol = parse_quote!(custom::Wire);
    let codec = quote!(<custom::Wire as ::plugin_crate::MessagingProtocol>::DefaultCodec);
    let route = build_ws_route(
        &parse_quote!(Controller),
        &protocol,
        &method,
        &parse_quote!("send"),
        &codec,
        false,
        &paths(),
    )
    .expect("send route");
    let tokens = output(route.builder);

    assert!(tokens.contains("plugin_crate :: TopicCodec < custom :: Wire"));
    assert!(tokens.contains("WsDispatchError :: Application"));
    assert!(tokens.contains("plugin_crate :: WsRespond < ()"));
}

#[test]
fn server_request_uses_message_reply_and_application_error() {
    let method: ImplItemFn = parse_quote! {
        async fn request(&self, payload: Payload) -> Result<Reply, Failure> {
            Err(Failure)
        }
    };
    let protocol = parse_quote!(custom::Wire);
    let codec = quote!(custom::Codec);
    let route = build_ws_route(
        &parse_quote!(Controller),
        &protocol,
        &method,
        &parse_quote!("request"),
        &codec,
        true,
        &paths(),
    )
    .expect("request route");
    let tokens = output(route.builder);

    assert!(tokens.contains("custom :: Codec as :: plugin_crate :: TopicCodec < custom :: Wire >"));
    assert!(tokens.contains("WsDispatchError :: Application"));
    assert!(tokens.contains("custom :: Wire as :: plugin_crate :: MessageReply"));
}

#[test]
fn generated_clients_use_actual_protocol_and_codec() {
    let send: ImplItemFn = parse_quote! {
        fn send(&self, payload: Payload) -> Result<(), Failure> {
            Ok(())
        }
    };
    let request: ImplItemFn = parse_quote! {
        fn request(&self, payload: Payload) -> Result<Reply, Failure> {
            Err(Failure)
        }
    };
    let protocol = parse_quote!(custom::Wire);
    let codec = quote!(custom::Codec);
    let destination = parse_quote!("messages");
    let send = build_message_send_method(
        &parse_quote!(send),
        &send,
        &destination,
        &protocol,
        &codec,
        &paths(),
    )
    .expect("send client")
    .expect("send method");
    let request = build_message_request_method(
        &parse_quote!(request),
        &request,
        &destination,
        &protocol,
        &codec,
        &paths(),
    )
    .expect("request client")
    .expect("request method");
    let send_tokens = output(send.override_bounds.expect("send bounds"));
    let request_bounds = request.override_bounds.expect("request bounds");
    let request_body = request.override_body.expect("request body");
    let request_ret = request.override_ret.expect("request return");
    let request_tokens = output(quote!(#request_bounds #request_ret #request_body));

    assert!(send_tokens.contains("MessageSend < custom :: Wire >"));
    assert!(request_tokens.contains("MessageRequest < custom :: Wire >"));
    assert!(
        request_tokens
            .contains("custom :: Codec as :: plugin_crate :: TopicCodec < custom :: Wire >")
    );
    assert!(request_tokens.contains("Result < Reply"));
    assert!(!request_tokens.contains("Failure"));
}

#[test]
fn no_payload_clients_still_use_topic_codec() {
    let method: ImplItemFn = parse_quote! {
        fn ping(&self) {}
    };
    let protocol = parse_quote!(custom::Wire);
    let codec = quote!(custom::Codec);
    let destination = parse_quote!("ping");
    let method = build_message_send_method(
        &parse_quote!(ping),
        &method,
        &destination,
        &protocol,
        &codec,
        &paths(),
    )
    .expect("send client")
    .expect("send method");
    let body = output(method.override_body.expect("send body"));

    assert!(body.contains("custom :: Codec as :: plugin_crate :: TopicCodec < custom :: Wire >"));
    assert!(body.contains("encode (& ())"));
    assert!(!body.contains("Default :: default"));
}

#[test]
fn explicit_unit_request_has_unit_client_response() {
    let output: ReturnType = parse_quote!(-> Result<(), Failure>);

    assert!(resolve_message_reply(MessageMode::Request, &output));
    let response = super::client::response_type(&output);

    assert_eq!(quote!(#response).to_string(), "()");
}

#[test]
fn ws_route_group_uses_handlers_protocol() {
    let mut handlers = AxumHandlers::default();
    let protocol = parse_quote!(custom::Wire);
    let mut method: ImplItemFn = parse_quote! {
        #[message("send")]
        fn send(&self, payload: Payload) {}
    };

    handlers.ws_protocol = Some(protocol);
    handlers.context = Some(HandlerContext {
        self_ty: parse_quote!(Controller),
        self_ident: parse_quote!(Controller),
        paths: paths(),
        capture: Vec::new(),
    });
    overseerd_macros_core::extend::ParseMethod::parse_method(&mut handlers, &mut method)
        .expect("message method");

    let tokens = output(quote!(#handlers));

    assert!(tokens.contains("WsRoute < custom :: Wire >"));
    assert!(tokens.contains("WebsocketController < Protocol = custom :: Wire >"));
}
#[test]

fn default_codec_is_projected_from_actual_protocol() {
    let handlers = AxumHandlers::default();
    let protocol = parse_quote!(custom::Wire);
    let codec = handlers.resolve_ws_codec(&protocol, &paths());
    let tokens = output(codec);

    assert_eq!(
        tokens,
        "< custom :: Wire as :: plugin_crate :: MessagingProtocol > :: DefaultCodec"
    );
}
