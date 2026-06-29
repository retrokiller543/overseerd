//! The protocol-agnostic generated **client** — a framework responsibility.
//!
//! The generated `{Service}Client<C>` is transport-generic and capability-partitioned: it
//! roots entirely at `::overseerd::client::*` (the agnostic capability contract), so the same
//! client works over any protocol that supplies those capabilities. The framework therefore
//! **owns the client generation**; a protocol macro only describes each method as a
//! [`ClientMethod`] *hint* — returned as a byproduct of its
//! [`ParseMethod::parse_method`](crate::ParseMethod::parse_method) — and the framework
//! assembles the method bodies and emits the capability-partitioned client from those hints.
//!
//! Why a hint and not the finished code: most of the type extraction (the request body, the
//! streamed item types, the error model) flows through *protocol-specific* extractors — RPC's
//! `Payload<T>` / `Streaming<T>` / `ResponseError` — so the protocol resolves the types; the
//! framework owns the (non-trivial, agnostic) assembly of the call signature and body.

use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{Ident, Type};

use crate::paths::Paths;

/// Which client capability a method needs — selects the `impl<C: Cap>` block it lands in, so
/// the method exists only when the protocol `C` supports that call shape (an unsupported one is
/// simply absent, never a compile error).
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Capability {
    Unary,
    ServerStreaming,
    ClientStreaming,
    BidiStreaming,
}

/// A protocol's description ("hint") of one client method. The protocol fills it from its own
/// signature analysis (which extractors mean a request body, a stream, an error type); the
/// framework assembles the call signature and body and emits the capability-partitioned client.
pub struct ClientMethod {
    /// The generated method name.
    pub ident: Ident,
    /// The wire path the client calls (e.g. `"Service.method"`).
    pub path: String,
    /// The call shape.
    pub capability: Capability,
    /// The unary/server-streaming request value type the method *takes*, or `None` for a
    /// no-body call. For HTTP this is the raw `T` (e.g. `SumIn`), the ergonomic surface; the
    /// wire body type it is encoded as may differ — see [`encode_as`](Self::encode_as).
    pub request: Option<Type>,
    /// Overrides the wire body type (`Encodes<B>` and the request envelope's `B`) when it
    /// differs from the [`request`](Self::request) param type. `None` encodes the param type
    /// as-is (RPC). HTTP sets it to the `HttpBody` wrapper (`Json<T>`/`Form<T>`) while the
    /// param stays the raw `T`, and the `request_builder` wraps it.
    pub encode_as: Option<TokenStream>,
    /// The client/bidi-streaming request *item* type (`None` for non-streaming-input).
    pub req_item: Option<Type>,
    /// The server/bidi-streaming response *item* type (`None` for non-streaming-output).
    pub resp_item: Option<Type>,
    /// The unary/client-streaming success type.
    pub response: Type,
    /// The decoded error body type (protocol-specific — RPC uses `<E as ResponseError>::Body`).
    /// `None` leaves the framework default `::overseerd::client::Raw` (an opaque error body).
    pub error_ty: Option<TokenStream>,
    /// Extra leading method parameters before the request body (HTTP path/query params).
    /// Empty for RPC. Spliced into the signature and visible to [`request_builder`](Self::request_builder).
    pub extra_args: Vec<(Ident, TokenStream)>,
    /// The request envelope type pinned by the `Unary<Request<B> = ..>` bound. `None` means the
    /// body passes through unchanged (`Request<B> = B`, RPC); `Some(ty)` pins a richer envelope
    /// (HTTP's `http::Request<B>`), so the generated method may construct it.
    pub request_envelope: Option<TokenStream>,
    /// Builds the request envelope value passed to the capability call. `None` forwards the body
    /// argument as-is (passthrough); `Some(expr)` builds it (HTTP constructs the `http::Request`).
    pub request_builder: Option<TokenStream>,
    /// The response envelope type pinned by the `Unary<Response<R> = ..>` bound, and the method's
    /// success return type. `None` returns the decoded body unchanged (`Response<R> = R`, RPC);
    /// `Some(ty)` returns a richer envelope (HTTP's `HttpResponse<R>`, which derefs to `R`).
    pub response_envelope: Option<TokenStream>,
    /// Maps the call result before returning. `None` returns it unchanged; `Some(expr)` is applied
    /// as `.map(#expr)` — for a protocol whose response needs a post-step the codec can't express.
    pub response_mapper: Option<TokenStream>,
}

impl ClientMethod {
    /// Assembles this method's argument list, return type, and body from its hint.
    fn build(
        &self,
        paths: &Paths,
    ) -> (
        TokenStream,
        TokenStream,
        TokenStream,
        TokenStream,
        TokenStream,
        TokenStream,
    ) {
        let client_error = paths.client("ClientError");
        let stream_arg = paths.client("StreamArg");
        let server_streaming = paths.client("ServerStreaming");
        let bidi_streaming = paths.client("BidiStreaming");
        let unary = paths.client("Unary");

        let raw = paths.client("Raw");
        let path = &self.path;
        let response = &self.response;
        let err = self.error_ty.clone().unwrap_or_else(|| quote!(#raw));

        // The unary/server-stream request param and the value forwarded (a no-body call sends
        // the unit body, so the protocol must `Encodes<()>`). The *wire body* type
        // (`unary_encode`) may differ from the param type when `encode_as` is set — HTTP takes a
        // raw `T` but encodes `Json<T>`.
        let (req_arg, call_arg, request_ty) = match &self.request {
            Some(req) => (quote!(, request: #req), quote!(request), quote!(#req)),
            None => (quote!(), quote!(()), quote!(())),
        };
        let unary_encode = self.encode_as.clone().unwrap_or(request_ty);

        // Leading path/query parameters (HTTP); empty for RPC. Spliced before the body param
        // and visible to `request_builder`.
        let extra_params = self
            .extra_args
            .iter()
            .map(|(name, ty)| quote!(, #name: #ty));
        let extra_params = quote!(#(#extra_params)*);
        let req_item = self
            .req_item
            .as_ref()
            .map(|t| quote!(#t))
            .unwrap_or_else(|| quote!(()));
        let resp_item = self
            .resp_item
            .clone()
            .unwrap_or_else(|| self.response.clone());

        // (encode_ty, decode_ty, args, ret, body, where_extra)
        match self.capability {
            Capability::Unary => {
                // The value handed to `unary`: the protocol's builder (HTTP's `http::Request`),
                // or the body argument passed straight through (RPC).
                let call = self
                    .request_builder
                    .clone()
                    .unwrap_or_else(|| call_arg.clone());

                // The pinned envelopes: a richer type, or the body/decoded value itself.
                let req_env = self
                    .request_envelope
                    .clone()
                    .unwrap_or_else(|| unary_encode.clone());
                let resp_env = self
                    .response_envelope
                    .clone()
                    .unwrap_or_else(|| quote!(#response));

                let map = match &self.response_mapper {
                    Some(mapper) => quote!(.map(#mapper)),
                    None => quote!(),
                };

                (
                    unary_encode.clone(),
                    quote!(#response),
                    quote!(&self #extra_params #req_arg),
                    quote!(::core::result::Result<#resp_env, #client_error<#err>>),
                    quote!(self.0.unary(#path, #call).await #map),
                    quote!(, C: #unary<Request<#unary_encode> = #req_env, Response<#response> = #resp_env>),
                )
            }

            Capability::ServerStreaming => (
                unary_encode,
                quote!(#resp_item),
                quote!(&self #req_arg),
                quote! {
                    ::core::result::Result<
                        <C as #server_streaming>::Responses<#resp_item, #err>,
                        #client_error<#err>,
                    >
                },
                quote!(self.0.server_stream(#path, #call_arg).await),
                quote!(),
            ),

            Capability::ClientStreaming => (
                quote!(#req_item),
                quote!(#response),
                quote! {
                    &self,
                    input: impl ::core::convert::Into<#stream_arg<#req_item>> + ::core::marker::Send
                },
                quote!(::core::result::Result<#response, #client_error<#err>>),
                quote!(self.0.client_stream(#path, input).await),
                quote!(),
            ),

            Capability::BidiStreaming => (
                quote!(#req_item),
                quote!(#resp_item),
                quote! {
                    &self,
                    input: impl ::core::convert::Into<#stream_arg<#req_item>> + ::core::marker::Send
                },
                quote! {
                    ::core::result::Result<
                        <C as #bidi_streaming>::Responses<#resp_item, #err>,
                        #client_error<#err>,
                    >
                },
                quote!(self.0.bidi_stream(#path, input).await),
                quote!(),
            ),
        }
    }
}

/// Emits a service's client *methods* as capability-partitioned `impl` blocks. The
/// `{Service}Client<C>` struct itself is emitted once by the component-variant macro (e.g. the
/// RPC `Router`); each impl-block macro contributes inherent methods here, so multiple blocks
/// compose. Methods are grouped by the capability they need into separate
/// `impl<C: Cap> {Service}Client<C>` blocks, each adding its `C: Encodes<Req> + Decodes<Resp>`
/// message bounds. Emits nothing without the `client` feature or when no methods are
/// contributed.
pub fn generate_client(
    client_ident: &Ident,
    methods: &[ClientMethod],
    paths: &Paths,
) -> TokenStream {
    if !cfg!(feature = "client") || methods.is_empty() {
        return quote!();
    }

    let encodes = paths.client("Encodes");
    let decodes = paths.client("Decodes");

    let capabilities = [
        (Capability::Unary, paths.client("Unary")),
        (Capability::ServerStreaming, paths.client("ServerStreaming")),
        (Capability::ClientStreaming, paths.client("ClientStreaming")),
        (Capability::BidiStreaming, paths.client("BidiStreaming")),
    ];

    let blocks = capabilities.iter().filter_map(|(capability, cap_trait)| {
        let fns: Vec<TokenStream> = methods
            .iter()
            .filter(|m| m.capability == *capability)
            .map(|m| {
                let ident = &m.ident;
                let (encode_ty, decode_ty, args, ret, body, where_extra) = m.build(paths);

                quote! {
                    pub async fn #ident(#args) -> #ret
                    where
                        C: #encodes<#encode_ty> + #decodes<#decode_ty> #where_extra,
                    {
                        #body
                    }
                }
            })
            .collect();

        if fns.is_empty() {
            return None;
        }

        Some(quote! {
            impl<C: #cap_trait> #client_ident<C> {
                #(#fns)*
            }
        })
    });

    quote!( #(#blocks)* )
}

/// The conventional client struct name for a service type, `{Type}Client`.
pub fn client_ident(self_ident: &Ident) -> Ident {
    format_ident!("{}Client", self_ident)
}
