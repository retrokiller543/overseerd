//! `#[handlers]` expansion (impl).
//!
//! Contributes the impl's `#[rpc]` methods to the service of `Self` (as an
//! `RpcGroup` keyed by type, so multiple impls merge into one service), and
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
    FnArg, ImplItem, ImplItemFn, ItemImpl, LitStr, Meta, ReturnType, Type, spanned::Spanned,
};

use crate::{attr, attr::HandlersArgs, paths::overseer_path};

pub fn expand(args: HandlersArgs, mut item: ItemImpl) -> syn::Result<TokenStream> {
    let self_ty = (*item.self_ty).clone();
    let self_ident = self_ty_ident(&self_ty)?;
    let self_name = LitStr::new(&self_ident.to_string(), self_ident.span());

    let mut wrappers = Vec::new();
    let mut descriptors = Vec::new();
    let mut client_methods = Vec::new();
    let mut init: Option<InitInfo> = None;

    for impl_item in &mut item.items {
        let ImplItem::Fn(method) = impl_item else {
            continue;
        };

        if let Some(pos) = method.attrs.iter().position(|a| a.path().is_ident("init")) {
            method.attrs.remove(pos);

            if init.is_some() {
                return Err(syn::Error::new_spanned(
                    &method.sig,
                    "this impl block already has an #[init] constructor",
                ));
            }

            init = Some(parse_init(method)?);
            continue;
        }

        let Some(pos) = method.attrs.iter().position(|a| a.path().is_ident("rpc")) else {
            continue;
        };

        let rpc_attr = method.attrs.remove(pos);

        if !matches!(rpc_attr.meta, Meta::Path(_)) {
            return Err(syn::Error::new_spanned(
                &rpc_attr.meta,
                "#[rpc] takes no arguments; the RPC kind is inferred from the signature \
                 (`Streaming<T>` parameter and/or `ResponseStream<T>` return)",
            ));
        }

        let (wrapper, descriptor, client) = expand_method(&self_ty, &self_ident, &self_name, method)?;

        wrappers.push(wrapper);
        descriptors.push(descriptor);
        client_methods.push(client);
    }

    let client_code = generate_client(&self_ident, &args.client_trait, &client_methods);

    let rpc_registration = if descriptors.is_empty() {
        quote!()
    } else {
        let count = descriptors.len();
        let distributed_slice = overseer_path("linkme::distributed_slice");
        let linkme_crate = overseer_path("linkme");
        let rpc_descriptor = overseer_path("RpcDescriptor");
        let rpc_group = overseer_path("RpcGroup");
        let rpc_groups_slice = overseer_path("RPC_GROUPS");
        let type_descriptor = overseer_path("TypeDescriptor");

        quote! {
            static __OVERSEER_RPCS: [#rpc_descriptor; #count] = [
                #(#descriptors),*
            ];

            #[#distributed_slice(#rpc_groups_slice)]
            #[linkme(crate = #linkme_crate)]
            static __OVERSEER_RPC_GROUP: #rpc_group = #rpc_group {
                service: #type_descriptor::of::<#self_ty>(#self_name),
                rpcs: &__OVERSEER_RPCS,
            };
        }
    };

    let (init_marker, init_component) = match &init {
        Some(info) => generate_init(&self_ty, &self_name, info),
        None => (quote!(), quote!()),
    };

    Ok(quote! {
        #item

        #init_marker

        #client_code

        const _: () = {
            #(#wrappers)*

            #init_component

            #rpc_registration
        };
    })
}

/// Builds the erased handler wrapper, `RpcDescriptor`, and client method for one
/// `#[rpc]` method.
fn expand_method(
    self_ty: &Type,
    self_ident: &syn::Ident,
    self_name: &LitStr,
    method: &ImplItemFn,
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

    // The kind is inferred from the signature: a `Streaming<T>` parameter means
    // streamed input, a `ResponseStream<T>` return means streamed output.
    let streaming_params: Vec<&&Type> = param_types
        .iter()
        .filter(|ty| attr::is_streaming_param(ty))
        .collect();

    if streaming_params.len() > 1 {
        return Err(syn::Error::new_spanned(
            &method.sig.inputs,
            "an rpc method may take at most one `Streaming<T>` parameter",
        ));
    }

    let streamed_input = streaming_params.len() == 1;

    if streamed_input && param_types.iter().any(|ty| attr::is_payload_param(ty)) {
        return Err(syn::Error::new_spanned(
            &method.sig.inputs,
            "a streaming-input rpc reads its request from `Streaming<T>`; \
             remove the `Payload<T>` parameter",
        ));
    }

    let streamed_output = attr::returns_response_stream(&method.sig.output);
    let operation = attr::operation_ident(streamed_input, streamed_output);

    let method_ident = &method.sig.ident;
    let method_name = LitStr::new(&method_ident.to_string(), method_ident.span());
    let output_ty = attr::response_body_type(&method.sig.output);
    let output_name = LitStr::new(&output_ty.to_token_stream().to_string(), output_ty.span());

    let wrapper_ident = format_ident!(
        "__overseer_rpc_{}_{}",
        self_ident.to_string().to_lowercase(),
        method_ident
    );

    // A `Result` return dispatches through `FallibleHandler` (which enforces
    // `E: ResponseError`); any other `Responder` return goes through
    // `Handler`. Both erase to the same `RpcHandler` fn pointer.
    let dispatch = if attr::returns_result(&method.sig.output) {
        overseer_path("dispatch_fallible")
    } else {
        overseer_path("dispatch_with")
    };
    let error = overseer_path("Error");
    let operation_kind = overseer_path("OperationKind");
    let rpc_call_context = overseer_path("RpcCallContext");
    let rpc_descriptor = overseer_path("RpcDescriptor");
    let type_descriptor = overseer_path("TypeDescriptor");
    let ret = handler_return_type();

    let wrapper = if takes_self {
        let arg_idents: Vec<_> = (0..param_types.len())
            .map(|i| format_ident!("__a{i}"))
            .collect();

        quote! {
            fn #wrapper_ident(ctx: #rpc_call_context) -> #ret {
                ::std::boxed::Box::pin(async move {
                    let __svc = ctx
                        .component::<#self_ty>()
                        .ok_or(#error::MissingComponent(#self_name))?;

                    #dispatch(
                        move |#(#arg_idents: #param_types),*| {
                            let __svc = ::std::sync::Arc::clone(&__svc);

                            async move { <#self_ty>::#method_ident(&__svc, #(#arg_idents),*).await }
                        },
                        ctx,
                    )
                    .await
                })
            }
        }
    } else {
        quote! {
            fn #wrapper_ident(ctx: #rpc_call_context) -> #ret {
                #dispatch(<#self_ty>::#method_ident, ctx)
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

    let client = client_method(
        self_ident,
        method_ident,
        &param_types,
        &method.sig.output,
        streamed_input,
        streamed_output,
    );

    Ok((wrapper, descriptor, client))
}

/// A generated client method's pieces: the call name, its argument list
/// (including `&self`), its return type, and the body that drives the connection.
struct ClientMethod {
    ident: syn::Ident,
    args: TokenStream,
    ret: TokenStream,
    body: TokenStream,
}

/// Derives the typed client method mirroring one `#[rpc]`. Request types come from
/// the `Payload<T>`/`Streaming<T>` parameters and response/error types from the
/// return shape; the operation kind selects which `ClientConnection` call to make.
fn client_method(
    self_ident: &syn::Ident,
    method_ident: &syn::Ident,
    param_types: &[&Type],
    output: &ReturnType,
    streamed_input: bool,
    streamed_output: bool,
) -> ClientMethod {
    let path = LitStr::new(
        &format!("{}.{}", self_ident, method_ident),
        method_ident.span(),
    );
    let client_error = overseer_path("transport::ClientError");
    let client_transport = overseer_path("transport::ClientTransport");
    let response_error = overseer_path("ResponseError");
    let raw = overseer_path("transport::Raw");

    let payload_ty = param_types.iter().find_map(|ty| attr::payload_inner(ty));
    let streaming_ty = param_types.iter().find_map(|ty| attr::streaming_inner(ty));
    let (result_ok, result_err) = match attr::result_type_args(output) {
        Some((ok, err)) => (Some(ok), err),
        None => (None, None),
    };

    // The client decodes the *body* the error serializes, which may differ from
    // the handler's error type — so it tracks `<E as ResponseError>::Body`, not `E`.
    // A one-argument `Result<T>` alias hides its error, so it falls back to `Raw`.
    let err_ty = match &result_err {
        Some(e) => quote!(<#e as #response_error>::Body),
        None => quote!(#raw),
    };

    // The success type carried on the wire: the `Ok` of a `Result`, else the bare
    // return type, else `()` for an absent return. `Option<T>` is left intact.
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
    let req_item = streaming_ty
        .as_ref()
        .map(|t| quote!(#t))
        .unwrap_or_else(|| quote!(()));

    let (args, ret, body) = match (streamed_input, streamed_output) {
        (false, false) => {
            let ret = quote!(::core::result::Result<#success_ty, #client_error<#err_ty>>);
            let body = quote!(self.conn.call(#path, #call_arg).await);

            (quote!(&self #req_arg), ret, body)
        }

        (false, true) => {
            let server_stream = overseer_path("transport::ServerStream");
            let item = attr::response_stream_inner(&success_ty).unwrap_or_else(|| success_ty.clone());
            let ret = quote! {
                ::core::result::Result<
                    #server_stream<<T as #client_transport>::Call, #item, #err_ty>,
                    #client_error<#err_ty>,
                >
            };
            let body = quote!(self.conn.server_stream(#path, #call_arg).await);

            (quote!(&self #req_arg), ret, body)
        }

        (true, false) => {
            let client_upstream = overseer_path("transport::ClientUpstream");
            let ret = quote! {
                ::core::result::Result<
                    #client_upstream<<T as #client_transport>::Call, #req_item, #success_ty, #err_ty>,
                    #client_error<#err_ty>,
                >
            };
            let body = quote!(self.conn.client_stream(#path).await);

            (quote!(&self), ret, body)
        }

        (true, true) => {
            let bidi_stream = overseer_path("transport::BidiStream");
            let item = attr::response_stream_inner(&success_ty).unwrap_or_else(|| success_ty.clone());
            let ret = quote! {
                ::core::result::Result<
                    #bidi_stream<<T as #client_transport>::Call, #req_item, #item, #err_ty>,
                    #client_error<#err_ty>,
                >
            };
            let body = quote!(self.conn.bidi_stream(#path).await);

            (quote!(&self), ret, body)
        }
    };

    ClientMethod {
        ident: method_ident.clone(),
        args,
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
    let client_connection = overseer_path("transport::ClientConnection");
    let client_transport = overseer_path("transport::ClientTransport");

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
            let async_trait = overseer_path("async_trait::async_trait");
            let signatures = methods.iter().map(|m| {
                let ClientMethod {
                    ident, args, ret, ..
                } = m;

                quote!(async fn #ident(#args) -> #ret;)
            });
            let implementations = methods.iter().map(|m| {
                let ClientMethod {
                    ident,
                    args,
                    ret,
                    body,
                } = m;

                quote! {
                    async fn #ident(#args) -> #ret {
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

/// The `#[init]` constructor: its `Arc<T>` parameters are injected dependencies.
struct InitInfo {
    ident: syn::Ident,
    is_async: bool,
    fallible: bool,
    param_types: Vec<Type>,
    dep_types: Vec<Type>,
    output: ReturnType,
}

fn parse_init(method: &ImplItemFn) -> syn::Result<InitInfo> {
    let mut param_types = Vec::new();
    let mut dep_types = Vec::new();

    for arg in &method.sig.inputs {
        match arg {
            FnArg::Receiver(receiver) => {
                return Err(syn::Error::new_spanned(
                    receiver,
                    "#[init] is a constructor and cannot take `self`",
                ));
            }
            FnArg::Typed(typed) => {
                param_types.push((*typed.ty).clone());
                dep_types.push(attr::arc_inner_type(&typed.ty)?);
            }
        }
    }

    Ok(InitInfo {
        ident: method.sig.ident.clone(),
        is_async: method.sig.asyncness.is_some(),
        fallible: attr::returns_result(&method.sig.output),
        param_types,
        dep_types,
        output: method.sig.output.clone(),
    })
}

/// Generates the fixed-name `init` marker/wrapper (module scope) and the
/// singleton component factory (const-block scope).
fn generate_init(
    self_ty: &Type,
    self_name: &LitStr,
    info: &InitInfo,
) -> (TokenStream, TokenStream) {
    let marked = &info.ident;
    let boxed_component = overseer_path("BoxedComponent");
    let component_construction_context = overseer_path("ComponentConstructionContext");
    let component_descriptor = overseer_path("ComponentDescriptor");
    let component_scope = overseer_path("ComponentScope");
    let components_slice = overseer_path("COMPONENTS");
    let dependency_descriptor = overseer_path("DependencyDescriptor");
    let distributed_slice = overseer_path("linkme::distributed_slice");
    let linkme_crate = overseer_path("linkme");
    let error = overseer_path("Error");
    let result = overseer_path("Result");
    let type_descriptor = overseer_path("TypeDescriptor");

    // The fixed `init` name is the compile-time uniqueness guard. If the marked
    // method is already named `init`, it is its own marker and needs no wrapper.
    let marker = if marked == "init" {
        quote!()
    } else {
        let fresh: Vec<_> = (0..info.param_types.len())
            .map(|i| format_ident!("__p{i}"))
            .collect();
        let param_types = &info.param_types;
        let output = &info.output;
        let asyncness = if info.is_async {
            quote!(async)
        } else {
            quote!()
        };
        let dotawait = if info.is_async {
            quote!(.await)
        } else {
            quote!()
        };

        quote! {
            impl #self_ty {
                #[doc(hidden)]
                #asyncness fn init(#(#fresh: #param_types),*) #output {
                    <#self_ty>::#marked(#(#fresh),*)#dotawait
                }
            }
        }
    };

    let resolved = info.dep_types.iter().map(|t| {
        let dep_name = LitStr::new(&t.to_token_stream().to_string(), t.span());

        quote! {
            cx.resolve::<::std::sync::Arc<#t>>()
                .ok_or(#error::MissingComponent(#dep_name))?
        }
    });

    let mut call = quote!(<#self_ty>::init(#(#resolved),*));

    if info.is_async {
        call = quote!(#call.await);
    }

    if info.fallible {
        call = quote!(#call?);
    }

    let cardinality = overseer_path("Cardinality");
    let dependency_descriptors = info.dep_types.iter().map(|t| {
        let dep_name = LitStr::new(&t.to_token_stream().to_string(), t.span());

        quote! {
            #dependency_descriptor {
                name: #dep_name,
                ty: #type_descriptor::of::<#t>(#dep_name),
                cardinality: #cardinality::One,
                optional: false,
                dynamic: false,
            }
        }
    });
    let dependency_count = info.dep_types.len();

    let component = quote! {
        #[allow(unused_variables)]
        fn __overseer_init_factory(
            cx: &mut #component_construction_context,
        ) -> ::core::pin::Pin<
            ::std::boxed::Box<
                dyn ::core::future::Future<
                    Output = #result<#boxed_component>,
                > + ::core::marker::Send + '_,
            >,
        > {
            ::std::boxed::Box::pin(async move {
                let __instance = #call;

                ::core::result::Result::Ok(#boxed_component {
                    ty: #type_descriptor::of::<#self_ty>(#self_name),
                    value: ::std::boxed::Box::new(::std::sync::Arc::new(__instance)),
                })
            })
        }

        static __OVERSEER_INIT_DEPS: [#dependency_descriptor; #dependency_count] = [
            #(#dependency_descriptors),*
        ];

        #[#distributed_slice(#components_slice)]
        #[linkme(crate = #linkme_crate)]
        static __OVERSEER_INIT_COMPONENT: #component_descriptor =
            #component_descriptor {
                id: #self_name,
                name: #self_name,
                ty: #type_descriptor::of::<#self_ty>(#self_name),
                scope: #component_scope::Singleton,
                dependencies: &__OVERSEER_INIT_DEPS,
                factory: ::core::option::Option::Some(__overseer_init_factory),
                default_factory: false,
            };
    };

    (marker, component)
}

/// The erased `RpcHandler` return type, repeated by both wrapper forms.
fn handler_return_type() -> TokenStream {
    let error_response = overseer_path("ErrorResponse");
    let rpc_outcome = overseer_path("RpcOutcome");

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
