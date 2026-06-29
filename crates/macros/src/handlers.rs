//! `#[handlers]` expansion (impl).
//!
//! Contributes the impl's `#[rpc]` methods to the service of `Self` as an
//! `RpcGroup` appended to the service's `{Service}Rpcs` slice (the one `#[service]`
//! declares), so multiple impl blocks merge into one service with no coordination.
//! A block in another module than its `#[service]` brings the slice into scope with
//! `use path::{Service}Rpcs;` (or both pass a matching `rpc_slice = ..`). It also
//! turns an optional `#[init]` constructor into an explicit singleton factory.
//!
//! The `#[init]` constructor (any name) gets a fixed-name `init` associated fn
//! generated on the type that forwards the injected dependencies to it
//! (constructor injection). That fixed name is also a compile-time guard: two
//! `#[init]`s anywhere on the type produce two `init` definitions and fail with
//! E0592 ("duplicate definitions with name `init`"). When the marked method is
//! itself named `init`, it serves as its own marker and no wrapper is emitted.

use proc_macro2::TokenStream;
use quote::{ToTokens, format_ident, quote};
use syn::{
    Attribute, FnArg, Ident, ImplItem, ImplItemFn, ItemImpl, LitStr, Meta, ReturnType, Type,
    spanned::Spanned,
};

use crate::{
    attr,
    attr::HandlersArgs,
    paths::{overseerd_daemon_path, overseerd_path},
};

pub fn expand(args: HandlersArgs, mut item: ItemImpl) -> syn::Result<TokenStream> {
    let self_ty = (*item.self_ty).clone();
    let self_ident = self_ty_ident(&self_ty)?;
    let self_name = LitStr::new(&self_ident.to_string(), self_ident.span());

    let mut wrappers = Vec::new();
    let mut descriptors = Vec::new();
    let mut client_methods = Vec::new();
    let mut hooks: Vec<crate::hook::HookInfo> = Vec::new();

    for impl_item in &mut item.items {
        let ImplItem::Fn(method) = impl_item else {
            continue;
        };

        if method.attrs.iter().any(|a| a.path().is_ident("init")) {
            return Err(syn::Error::new_spanned(
                &method.sig,
                "#[init] is a lifecycle method — put it in a #[methods] impl, not #[handlers]",
            ));
        }

        if let Some(pos) = method.attrs.iter().position(|a| a.path().is_ident("hook")) {
            let attr = method.attrs.remove(pos);
            let kind = crate::hook::parse_hook_kind(&attr)?;

            hooks.push(crate::hook::parse_hook(method, kind)?);

            continue;
        }

        let Some(pos) = method.attrs.iter().position(|a| a.path().is_ident("rpc")) else {
            continue;
        };

        let rpc_attr = method.attrs.remove(pos);
        let stream_flag = parse_rpc_attr(&rpc_attr)?;

        let (wrapper, descriptor, client) =
            expand_method(&self_ty, &self_ident, &self_name, method, stream_flag)?;

        wrappers.push(wrapper);
        descriptors.push(descriptor);
        client_methods.push(client);
    }

    let hooks_slice = crate::inject::hooks_slice_ident(&self_ident);
    let hook_tokens: Vec<TokenStream> = hooks
        .iter()
        .enumerate()
        .map(|(index, info)| {
            crate::hook::generate_hook(&self_ty, &self_name, &hooks_slice, info, index)
        })
        .collect();

    let client_code = generate_client(&self_ident, &args.client_trait, &client_methods);

    let rpc_registration = if descriptors.is_empty() {
        quote!()
    } else {
        let count = descriptors.len();
        let distributed_slice = overseerd_path("linkme::distributed_slice");
        let linkme_crate = overseerd_path("linkme");
        let rpc_descriptor = overseerd_daemon_path("RpcDescriptor");
        let rpc_group = overseerd_daemon_path("RpcGroup");
        let rpcs_slice = args
            .rpc_slice
            .clone()
            .unwrap_or_else(|| format_ident!("{}Rpcs", self_ident));
        let type_descriptor = overseerd_path("TypeDescriptor");

        quote! {
            static __OVERSEERD_RPCS: [#rpc_descriptor; #count] = [
                #(#descriptors),*
            ];

            #[#distributed_slice(#rpcs_slice)]
            #[linkme(crate = #linkme_crate)]
            static __OVERSEERD_RPC_GROUP: #rpc_group = #rpc_group {
                service: #type_descriptor::of::<#self_ty>(#self_name),
                rpcs: &__OVERSEERD_RPCS,
            };
        }
    };

    Ok(quote! {
        #item

        #client_code

        const _: () = {
            #(#wrappers)*

            #rpc_registration

            #(#hook_tokens)*
        };
    })
}

/// Parses the `#[rpc]` attribute, returning whether the `stream` flag is set.
///
/// `#[rpc]` infers the RPC kind from the signature; the sole argument
/// `#[rpc(stream)]` marks a *concrete* return type as a server stream for the
/// cases the macro cannot otherwise see it is a `Stream` (it only flips the
/// operation kind — item serialization is unchanged).
fn parse_rpc_attr(attr: &Attribute) -> syn::Result<bool> {
    match &attr.meta {
        Meta::Path(_) => Ok(false),

        Meta::List(list) => {
            let ident: Ident = list.parse_args()?;

            if ident == "stream" {
                Ok(true)
            } else {
                Err(syn::Error::new_spanned(
                    &list.tokens,
                    "unknown #[rpc] argument; the only argument is `stream` (mark a concrete \
                     return type as a server stream)",
                ))
            }
        }

        Meta::NameValue(meta) => Err(syn::Error::new_spanned(
            meta,
            "#[rpc] takes no name-value arguments",
        )),
    }
}

/// The streaming shape resolved from a handler signature, threaded into the
/// client codegen: which directions stream, the value item types the client
/// sends/receives (1:1 with the daemon), and a streaming output's per-item error.
struct StreamTypes {
    streamed_input: bool,
    streamed_output: bool,
    req_item: Option<Type>,
    resp_item: Option<Type>,
    stream_err: Option<Type>,
}

/// How the dispatch wrapper adapts a streaming return value into the canonical
/// `ResponseStream` before it reaches the `Responder` layer.
enum OutputWrap {
    /// Already a `Responder` (an explicit `ResponseStream<T>`); pass through.
    None,
    /// `impl Stream<Item = T>` — wrap with `ResponseStream::from_items`.
    Items,
    /// `impl Stream<Item = Result<T, E>>` — wrap with `ResponseStream::from_results`.
    Results,
}

/// Builds the erased handler wrapper, `RpcDescriptor`, and client method for one
/// `#[rpc]` method.
fn expand_method(
    self_ty: &Type,
    self_ident: &syn::Ident,
    self_name: &LitStr,
    method: &ImplItemFn,
    stream_flag: bool,
) -> syn::Result<(TokenStream, TokenStream, ClientMethod)> {
    if method.sig.asyncness.is_none() {
        return Err(syn::Error::new_spanned(
            &method.sig,
            "rpc methods must be `async`",
        ));
    }

    let takes_self = match method.sig.inputs.first() {
        Some(FnArg::Receiver(receiver)) => {
            if receiver.reference.is_none() || receiver.mutability.is_some() {
                return Err(syn::Error::new_spanned(
                    receiver,
                    "rpc methods may take `&self` only (the service singleton is shared; \
                     `self` by value and `&mut self` are not allowed)",
                ));
            }

            true
        }
        _ => false,
    };

    let param_types: Vec<&Type> = method
        .sig
        .inputs
        .iter()
        .filter_map(|arg| match arg {
            FnArg::Typed(typed) => Some(typed.ty.as_ref()),
            FnArg::Receiver(_) => None,
        })
        .collect();

    let generics = &method.sig.generics;
    let streaming = overseerd_daemon_path("Streaming");
    let request_stream = overseerd_daemon_path("RequestStream");

    // Resolve each parameter into the concrete extractor type the dispatch
    // closure declares (so it always flows through `FromContext`), and detect the
    // at-most-one streaming input. A `Streaming<T>` parameter keeps its fallible
    // items; an `impl Stream<Item = T>` / generic `S: Stream<..>` parameter maps
    // to `RequestStream<T>` (or `Streaming<T>` when its items are `Result`s) so
    // the handler receives exactly the item shape it declared.
    let mut extractor_types: Vec<TokenStream> = Vec::new();
    let mut streaming_inputs = 0usize;
    let mut has_payload = false;
    let mut req_item: Option<Type> = None;

    for ty in &param_types {
        if attr::is_payload_param(ty) {
            has_payload = true;
        }

        if attr::is_streaming_param(ty) {
            streaming_inputs += 1;
            req_item = attr::streaming_inner(ty);
            extractor_types.push(quote!(#ty));

            continue;
        }

        if let Some(shape) = attr::stream_shape(ty, generics) {
            let item = &shape.item;
            streaming_inputs += 1;
            req_item = Some(item.clone());

            if shape.fallible() {
                extractor_types.push(quote!(#streaming<#item>));
            } else {
                extractor_types.push(quote!(#request_stream<#item>));
            }

            continue;
        }

        extractor_types.push(quote!(#ty));
    }

    if streaming_inputs > 1 {
        return Err(syn::Error::new_spanned(
            &method.sig.inputs,
            "an rpc method may take at most one streaming-input parameter",
        ));
    }

    let streamed_input = streaming_inputs == 1;

    if streamed_input && has_payload {
        return Err(syn::Error::new_spanned(
            &method.sig.inputs,
            "a streaming-input rpc reads its request from its stream parameter; \
             remove the `Payload<T>` parameter",
        ));
    }

    // Output streaming: an explicit `ResponseStream<T>` return (already a
    // `Responder`, so no wrapping), an `impl Stream` / generic stream return
    // (wrapped per its item shape), or a concrete return flagged `#[rpc(stream)]`
    // (wrapped as plain items — the macro cannot introspect its shape).
    let return_ty = match &method.sig.output {
        ReturnType::Type(_, ty) => Some(ty.as_ref()),
        ReturnType::Default => None,
    };
    let return_shape = return_ty.and_then(|ty| attr::stream_shape(ty, generics));
    let returns_response_stream = attr::returns_response_stream(&method.sig.output);

    let (streamed_output, output_wrap, resp_item, stream_err): (
        bool,
        OutputWrap,
        Option<Type>,
        Option<Type>,
    ) = if returns_response_stream {
        let item = attr::response_body_type(&method.sig.output);
        let err = attr::result_type_args(&method.sig.output).and_then(|(_, err)| err);

        (true, OutputWrap::None, Some(item), err)
    } else if let Some(shape) = return_shape {
        let wrap = if shape.fallible() {
            OutputWrap::Results
        } else {
            OutputWrap::Items
        };

        (true, wrap, Some(shape.item), shape.error)
    } else if stream_flag {
        // A concrete return the macro cannot introspect: serialize each item as-is,
        // and recover the wire item type by projecting through the `Stream` trait so
        // the client stays well-typed (`<ReturnType as Stream>::Item`).
        let stream_trait = overseerd_daemon_path("__Stream");
        let item = return_ty.map(|ty| syn::parse_quote!(<#ty as #stream_trait>::Item));

        (true, OutputWrap::Items, item, None)
    } else {
        (false, OutputWrap::None, None, None)
    };

    let operation = attr::operation_ident(streamed_input, streamed_output);

    let method_ident = &method.sig.ident;
    let method_name = LitStr::new(&method_ident.to_string(), method_ident.span());

    // Descriptor metadata uses the streamed item type (not the stream wrapper,
    // which may be an un-nameable `impl Trait`); a non-streaming output keeps the
    // peeled response body type.
    let output_ty: Type = if streamed_output {
        resp_item.clone().unwrap_or_else(|| syn::parse_quote!(()))
    } else {
        attr::response_body_type(&method.sig.output)
    };
    let output_name = LitStr::new(&output_ty.to_token_stream().to_string(), output_ty.span());

    let wrapper_ident = format_ident!(
        "__overseerd_rpc_{}_{}",
        self_ident.to_string().to_lowercase(),
        method_ident
    );

    // A `Result` return dispatches through `FallibleHandler` (which enforces
    // `E: ResponseError`); any other `Responder` return goes through `Handler`.
    // Both erase to the same `RpcHandler` fn pointer.
    let dispatch = if attr::returns_result(&method.sig.output) {
        overseerd_daemon_path("dispatch_fallible")
    } else {
        overseerd_daemon_path("dispatch_with")
    };
    let error = overseerd_daemon_path("Error");
    let operation_kind = overseerd_daemon_path("OperationKind");
    let response_stream = overseerd_daemon_path("ResponseStream");
    let rpc_call_context = overseerd_daemon_path("RpcCallContext");
    let rpc_descriptor = overseerd_daemon_path("RpcDescriptor");
    let type_descriptor = overseerd_path("TypeDescriptor");
    let ret = handler_return_type();

    let wrap = |inner: TokenStream| match output_wrap {
        OutputWrap::None => inner,
        OutputWrap::Items => quote!(#response_stream::from_items(#inner)),
        OutputWrap::Results => quote!(#response_stream::from_results(#inner)),
    };

    let arg_idents: Vec<_> = (0..param_types.len())
        .map(|i| format_ident!("__a{i}"))
        .collect();

    let wrapper = if takes_self {
        let body = wrap(quote!(<#self_ty>::#method_ident(&__svc, #(#arg_idents),*).await));

        quote! {
            fn #wrapper_ident(ctx: #rpc_call_context) -> #ret {
                ::std::boxed::Box::pin(async move {
                    let __svc = ctx
                        .component::<#self_ty>()
                        .ok_or(#error::MissingComponent(#self_name))?;

                    #dispatch(
                        move |#(#arg_idents: #extractor_types),*| {
                            let __svc = ::std::sync::Arc::clone(&__svc);

                            async move { #body }
                        },
                        ctx,
                    )
                    .await
                })
            }
        }
    } else {
        let body = wrap(quote!(<#self_ty>::#method_ident(#(#arg_idents),*).await));

        quote! {
            fn #wrapper_ident(ctx: #rpc_call_context) -> #ret {
                #dispatch(
                    move |#(#arg_idents: #extractor_types),*| async move { #body },
                    ctx,
                )
            }
        }
    };

    let descriptor = quote! {
        #rpc_descriptor {
            name: #method_name,
            operation: #operation_kind::#operation,
            parameters: &[],
            output: #type_descriptor::of::<#output_ty>(#output_name),
            handler: #wrapper_ident,
        }
    };

    let stream_types = StreamTypes {
        streamed_input,
        streamed_output,
        req_item,
        resp_item,
        stream_err,
    };
    let client = client_method(
        self_ident,
        method_ident,
        &param_types,
        &method.sig.output,
        &stream_types,
    );

    Ok((wrapper, descriptor, client))
}

/// A generated client method's pieces: the call name, its argument lists
/// (including `&self`), its return type, and the body that drives the connection.
///
/// `args` and `args_trait` differ only for a streaming-input method: the inherent
/// client takes an `impl Stream` (ergonomic), while the `dyn`-compatible trait
/// client takes a boxed `StreamArg<T>` so the method stays object-safe. They are
/// identical for every other shape.
struct ClientMethod {
    ident: syn::Ident,
    args: TokenStream,
    args_trait: TokenStream,
    ret: TokenStream,
    body: TokenStream,
}

/// Derives the typed client method mirroring one `#[rpc]`. Request types come from
/// the `Payload<T>` parameter and the resolved streaming-input item; response and
/// error types from the resolved streaming-output item (or the unary return
/// shape); the operation kind selects which `ClientConnection` call to make.
///
/// `req_item`/`resp_item`/`stream_err` are the value item types (and per-item
/// error) the macro recovered from the handler's stream shapes — used so the
/// client mirrors the daemon's parameters and differs only in the error type.
fn client_method(
    self_ident: &syn::Ident,
    method_ident: &syn::Ident,
    param_types: &[&Type],
    output: &ReturnType,
    stream: &StreamTypes,
) -> ClientMethod {
    let StreamTypes {
        streamed_input,
        streamed_output,
        req_item,
        resp_item,
        stream_err,
    } = stream;
    let (streamed_input, streamed_output) = (*streamed_input, *streamed_output);
    let path = LitStr::new(
        &format!("{}.{}", self_ident, method_ident),
        method_ident.span(),
    );
    let client_error = overseerd_daemon_path("ClientError");
    let client_transport = overseerd_daemon_path("ClientTransport");
    let response_error = overseerd_daemon_path("ResponseError");
    let raw = overseerd_daemon_path("Raw");

    let payload_ty = param_types.iter().find_map(|ty| attr::payload_inner(ty));
    let (result_ok, result_err) = match attr::result_type_args(output) {
        Some((ok, err)) => (Some(ok), err),
        None => (None, None),
    };

    // The client decodes the *body* the error serializes, which may differ from
    // the handler's error type — so it tracks `<E as ResponseError>::Body`, not `E`.
    // A streaming output's per-item error wins; otherwise the unary result error.
    // A hidden error (one-argument `Result<T>`, or an infallible stream) is `Raw`.
    let err_source = stream_err.clone().or(result_err);
    let err_ty = match &err_source {
        Some(e) => quote!(<#e as #response_error>::Body),
        None => quote!(#raw),
    };

    // The unary success type carried on the wire: the `Ok` of a `Result`, else the
    // bare return type, else `()` for an absent return. `Option<T>` is left intact.
    let success_ty: Type = match (&result_ok, output) {
        (Some(ok), _) => ok.clone(),
        (None, ReturnType::Type(_, ty)) => (**ty).clone(),
        (None, ReturnType::Default) => syn::parse_quote!(()),
    };

    // The optional request parameter and the value forwarded to a unary or
    // server-streaming call (a no-payload method sends the unit body).
    let (req_arg, call_arg) = match &payload_ty {
        Some(req) => (quote!(, req: &#req), quote!(req)),
        None => (quote!(), quote!(&())),
    };
    let req_item_ty = req_item
        .as_ref()
        .map(|t| quote!(#t))
        .unwrap_or_else(|| quote!(()));
    let resp_item_ty = resp_item.clone().unwrap_or_else(|| success_ty.clone());

    let (args, args_trait, ret, body) = match (streamed_input, streamed_output) {
        (false, false) => {
            let args = quote!(&self #req_arg);
            let ret = quote!(::core::result::Result<#success_ty, #client_error<#err_ty>>);
            let body = quote!(self.conn.call(#path, #call_arg).await);

            (args.clone(), args, ret, body)
        }

        (false, true) => {
            let server_stream = overseerd_daemon_path("ServerStream");
            let args = quote!(&self #req_arg);
            let ret = quote! {
                ::core::result::Result<
                    #server_stream<<T as #client_transport>::Call, #resp_item_ty, #err_ty>,
                    #client_error<#err_ty>,
                >
            };
            let body = quote!(self.conn.server_stream(#path, #call_arg).await);

            (args.clone(), args, ret, body)
        }

        // Client streaming mirrors the daemon: take the input stream, return the one
        // response. The inherent client accepts any `impl Stream`; the trait client
        // takes a boxed `StreamArg<T>` to stay object-safe.
        (true, false) => {
            let stream_trait = overseerd_daemon_path("__Stream");
            let stream_arg = overseerd_daemon_path("StreamArg");
            let ret = quote!(::core::result::Result<#success_ty, #client_error<#err_ty>>);
            let body = quote! {
                self.conn
                    .client_stream::<#req_item_ty, #success_ty, #err_ty, _>(#path, input)
                    .await
            };
            let args = quote! {
                &self,
                input: impl #stream_trait<Item = #req_item_ty> + ::core::marker::Send + 'static
            };
            let args_trait = quote!(&self, input: #stream_arg<#req_item_ty>);

            (args, args_trait, ret, body)
        }

        // Bidi is symmetric too: an input stream in, a response stream out, pumped
        // concurrently. The caller's input stream is their sink (push to a channel
        // for cause-and-effect); the returned stream is read independently.
        (true, true) => {
            let stream_trait = overseerd_daemon_path("__Stream");
            let stream_arg = overseerd_daemon_path("StreamArg");
            let bidi_responses = overseerd_daemon_path("BidiResponses");
            let ret = quote! {
                ::core::result::Result<
                    #bidi_responses<<T as #client_transport>::Call, #resp_item_ty, #err_ty>,
                    #client_error<#err_ty>,
                >
            };
            let body = quote! {
                self.conn
                    .bidi_stream::<#req_item_ty, #resp_item_ty, #err_ty, _>(#path, input)
                    .await
            };
            let args = quote! {
                &self,
                input: impl #stream_trait<Item = #req_item_ty> + ::core::marker::Send + 'static
            };
            let args_trait = quote!(&self, input: #stream_arg<#req_item_ty>);

            (args, args_trait, ret, body)
        }
    };

    ClientMethod {
        ident: method_ident.clone(),
        args,
        args_trait,
        ret,
        body,
    }
}

/// Assembles the per-service client: a `{Service}Client<T>` wrapper plus its
/// methods, as either a plain inherent impl or — with `#[handlers(client_trait =
/// Name)]` — a `dyn`-compatible trait `Name` and its impl. Emits nothing when the
/// macro is built without the `client` feature or the block declares no `#[rpc]`s.
fn generate_client(
    self_ident: &syn::Ident,
    client_trait: &Option<syn::Ident>,
    methods: &[ClientMethod],
) -> TokenStream {
    if !cfg!(feature = "client") || methods.is_empty() {
        return quote!();
    }

    let client_ident = format_ident!("{}Client", self_ident);
    let client_connection = overseerd_daemon_path("ClientConnection");
    let client_transport = overseerd_daemon_path("ClientTransport");

    let scaffold = quote! {
        pub struct #client_ident<T: #client_transport> {
            conn: #client_connection<T>,
        }

        impl<T: #client_transport> #client_ident<T> {
            /// Wraps an established client connection.
            pub fn new(conn: #client_connection<T>) -> Self {
                Self { conn }
            }
        }
    };

    match client_trait {
        None => {
            let methods = methods.iter().map(|m| {
                let ClientMethod {
                    ident,
                    args,
                    ret,
                    body,
                    ..
                } = m;

                quote! {
                    pub async fn #ident(#args) -> #ret {
                        #body
                    }
                }
            });

            quote! {
                #scaffold

                impl<T: #client_transport> #client_ident<T> {
                    #(#methods)*
                }
            }
        }

        Some(trait_ident) => {
            let async_trait = overseerd_daemon_path("async_trait::async_trait");
            let signatures = methods.iter().map(|m| {
                let ClientMethod {
                    ident,
                    args_trait,
                    ret,
                    ..
                } = m;

                quote!(async fn #ident(#args_trait) -> #ret;)
            });
            let implementations = methods.iter().map(|m| {
                let ClientMethod {
                    ident,
                    args_trait,
                    ret,
                    body,
                    ..
                } = m;

                quote! {
                    async fn #ident(#args_trait) -> #ret {
                        #body
                    }
                }
            });

            quote! {
                #scaffold

                #[#async_trait]
                pub trait #trait_ident<T: #client_transport> {
                    #(#signatures)*
                }

                #[#async_trait]
                impl<T: #client_transport> #trait_ident<T> for #client_ident<T> {
                    #(#implementations)*
                }
            }
        }
    }
}

/// The erased `RpcHandler` return type, repeated by both wrapper forms.
fn handler_return_type() -> TokenStream {
    let error_response = overseerd_daemon_path("ErrorResponse");
    let rpc_outcome = overseerd_daemon_path("RpcOutcome");

    quote! {
        ::core::pin::Pin<
            ::std::boxed::Box<
                dyn ::core::future::Future<
                    Output = ::core::result::Result<#rpc_outcome, #error_response>,
                > + ::core::marker::Send,
            >,
        >
    }
}

/// Extracts the named type ident from an impl's `Self` type.
fn self_ty_ident(ty: &Type) -> syn::Result<syn::Ident> {
    match ty {
        Type::Path(path) => path
            .path
            .segments
            .last()
            .map(|segment| segment.ident.clone())
            .ok_or_else(|| syn::Error::new_spanned(ty, "expected a named type")),
        _ => Err(syn::Error::new_spanned(
            ty,
            "#[handlers] must be applied to an impl of a named type",
        )),
    }
}
