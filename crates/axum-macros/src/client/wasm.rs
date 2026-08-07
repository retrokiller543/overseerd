use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{Ident, Type};
use upwell_macros_core::client::{Capability, ClientMethod};
use upwell_macros_core::paths::Paths;

use super::inputs::dto_assertions;
use super::path::tuple_elems;

fn wasm_wrapper_ident(client_ident: &Ident) -> Ident {
    format_ident!("__{}Wasm", client_ident)
}

/// Which shared transport a generated wasm client binds to.
#[derive(Clone)]
pub(crate) enum WasmBackend {
    Http,
    Message { protocol: syn::Path },
}

impl WasmBackend {
    fn available(&self) -> bool {
        match self {
            Self::Http => cfg!(feature = "reqwest"),
            Self::Message { .. } => cfg!(feature = "tungstenite"),
        }
    }

    fn transport(&self, paths: &Paths) -> TokenStream {
        match self {
            Self::Http => {
                let client = paths.plugin("client::ReqwestClient");
                quote!(#client)
            }
            Self::Message { protocol } => {
                let topic_wasm_client = paths.plugin("client::TopicWasmClient");
                quote!(<#protocol as #topic_wasm_client>::Transport)
            }
        }
    }

    fn build_from_connection(&self, client_ident: &Ident, paths: &Paths) -> TokenStream {
        match self {
            Self::Http => {
                quote!(::core::result::Result::Ok(Self(#client_ident::new(connection.http()))))
            }
            Self::Message { protocol } => {
                let topic_wasm_client = paths.plugin("client::TopicWasmClient");
                quote! {
                    ::core::result::Result::Ok(Self(#client_ident::new(
                        <#protocol as #topic_wasm_client>::transport(connection)?,
                    )))
                }
            }
        }
    }
}

/// Emits the wasm JavaScript wrapper struct and constructor.
pub(crate) fn wasm_client_struct(
    client_ident: &Ident,
    docs: &[syn::Attribute],
    backend: WasmBackend,
    paths: &Paths,
) -> TokenStream {
    if !backend.available() {
        return quote!();
    }

    let transport = backend.transport(paths);
    let connection = paths.plugin("client::Connection");
    let build = backend.build_from_connection(client_ident, paths);
    let js_name = client_ident.to_string();
    let wrapper = wasm_wrapper_ident(client_ident);

    quote! {
        #(#docs)*
        #[cfg(target_family = "wasm")]
        #[doc(hidden)]
        #[::wasm_bindgen::prelude::wasm_bindgen(js_name = #js_name)]
        pub struct #wrapper(#client_ident<#transport>);

        #[cfg(target_family = "wasm")]
        #[::wasm_bindgen::prelude::wasm_bindgen(js_class = #js_name)]
        impl #wrapper {
            #[::wasm_bindgen::prelude::wasm_bindgen(constructor)]
            pub fn new(
                connection: &#connection,
            ) -> ::core::result::Result<#wrapper, ::wasm_bindgen::JsError> {
                #build
            }
        }
    }
}

fn wasm_client_methods(
    client_ident: &Ident,
    methods: &[ClientMethod],
    backend: WasmBackend,
    paths: &Paths,
) -> TokenStream {
    let js_name = client_ident.to_string();
    let wrapper = wasm_wrapper_ident(client_ident);
    let headers_ty = paths.plugin("client::RequestHeaders");
    let ts = cfg!(feature = "wasm-ts");
    let fns = methods
        .iter()
        .filter(|method| method.capability == Capability::Unary)
        .map(|method| {
            let ident = &method.ident;
            let extra_params = method.extra_args.iter().map(|(name, ty)| {
                if ts {
                    quote!(, #name: ::tsify::Ts<#ty>)
                } else {
                    quote!(, #name: #ty)
                }
            });
            let extra_prep = method.extra_args.iter().map(|(name, _)| {
                if ts {
                    quote!(let #name = #name.to_rust().map_err(::wasm_bindgen::JsError::from)?;)
                } else {
                    quote!()
                }
            });
            let mut call_args = method
                .extra_args
                .iter()
                .map(|(name, _)| quote!(#name))
                .collect::<Vec<_>>();
            let (body_param, body_prep) = match &method.request {
                Some(request) => {
                    call_args.push(quote!(__request));

                    if ts {
                        (
                            quote!(, body: ::tsify::Ts<#request>),
                            quote!(let __request = body.to_rust().map_err(::wasm_bindgen::JsError::from)?;),
                        )
                    } else {
                        (quote!(, __request: #request), quote!())
                    }
                }
                None => (quote!(), quote!()),
            };
            let (header_param, header_prep, target) = match &backend {
                WasmBackend::Http => {
                    call_args.push(quote!(__headers));
                    (
                        quote!(, headers: ::core::option::Option<#headers_ty>),
                        quote!(let __headers = headers.map(#headers_ty::into_inner);),
                        format_ident!("{}_with_headers", ident),
                    )
                }
                WasmBackend::Message { .. } => (quote!(), quote!(), ident.clone()),
            };
            let response = &method.response;
            let response_is_unit = tuple_elems(response).is_some_and(|elems| elems.is_empty());
            let (ret, ret_expr) = match &backend {
                WasmBackend::Http if ts => (
                    quote!(::tsify::Ts<#response>),
                    quote!(__response.into_body().into_ts().map_err(::wasm_bindgen::JsError::from)?),
                ),
                WasmBackend::Http => (quote!(#response), quote!(__response.into_body())),
                WasmBackend::Message { .. } if response_is_unit => {
                    (quote!(()), quote!(__response))
                }
                WasmBackend::Message { .. } if ts => (
                    quote!(::tsify::Ts<#response>),
                    quote!(__response.into_ts().map_err(::wasm_bindgen::JsError::from)?),
                ),
                WasmBackend::Message { .. } => (quote!(#response), quote!(__response)),
            };

            quote! {
                pub async fn #ident(
                    &self #(#extra_params)* #body_param #header_param
                ) -> ::core::result::Result<#ret, ::wasm_bindgen::JsError> {
                    #(#extra_prep)*
                    #body_prep
                    #header_prep

                    let __response = self
                        .0
                        .#target(#(#call_args),*)
                        .await
                        .map_err(|e| ::wasm_bindgen::JsError::new(
                            &::std::string::ToString::to_string(&e),
                        ))?;

                    ::core::result::Result::Ok(#ret_expr)
                }
            }
        });

    quote! {
        #[cfg(target_family = "wasm")]
        #[::wasm_bindgen::prelude::wasm_bindgen(js_class = #js_name)]
        impl #wrapper {
            #(#fns)*
        }
    }
}

/// Emits wire assertions, response declarations, header methods, and wasm forwarding methods.
pub(crate) fn extra_client_tokens(
    client_ident: &Ident,
    methods: &[ClientMethod],
    header_methods: &[ClientMethod],
    wire_types: Vec<Type>,
    response_types: &[TokenStream],
    backend: Option<WasmBackend>,
    paths: &Paths,
) -> TokenStream {
    let assertions = dto_assertions(wire_types, paths);
    let with_headers = if cfg!(feature = "client") && !header_methods.is_empty() {
        let fns = header_methods
            .iter()
            .map(|method| upwell_macros_core::client::client_method_tokens(method, paths));
        quote! {
            impl<C> #client_ident<C> {
                #(#fns)*
            }
        }
    } else {
        quote!()
    };
    let wasm_methods = match backend {
        Some(backend) if backend.available() && !methods.is_empty() => {
            wasm_client_methods(client_ident, methods, backend, paths)
        }
        _ => quote!(),
    };

    quote! {
        #assertions

        #(#response_types)*

        #with_headers

        #wasm_methods
    }
}
