//! The controller handlers extension: `AxumHandlers`, the [`ParseMethod`] extension that
//! makes `#[handlers]` = `MethodArgs<AxumHandlers>` (`#[methods]` + route registration).
//!
//! `AxumHandlers` claims each route-attributed method, building typed axum handler closures and
//! emitting one route group for the controller. HTTP and WebSocket construction and emission live
//! in focused child modules; this facade retains the extension state and public path.

mod args;
mod emit;
mod http;
mod parse;
mod ws;
mod ws_client;
mod ws_emit;

use proc_macro2::TokenStream;
use quote::quote;
use syn::parse::ParseStream;
use syn::{GenericParam, Ident, ItemImpl, Type};

use overseerd_macros_core::client::ClientMethod;
use overseerd_macros_core::extend::{ParseItem, ParseKeyed, eat_eq};
use overseerd_macros_core::methods::self_ty_ident;
use overseerd_macros_core::paths::Paths;

use http::RouteSpec;
use ws::WsRouteSpec;

/// The controller handlers extension. Accumulates the impl's route specs and the captured impl
/// context, then emits a single route-group builder appended to the controller's route slice.
#[derive(Default)]
pub struct AxumHandlers {
    /// `routes_slice = ..` — the per-controller slice to append to (default `{Controller}Routes`).
    routes_slice: Option<Ident>,
    /// Captured during [`ParseItem`]: the impl's self type and resolved paths.
    context: Option<HandlerContext>,
    /// Accumulated per HTTP route-attributed method.
    routes: Vec<RouteSpec>,
    /// Wire types across this block's routes that must be `Dto`.
    wire_types: Vec<Type>,
    /// Unary HTTP `{method}_with_headers` client siblings.
    header_methods: Vec<ClientMethod>,
    /// Per-method status response enums emitted beside the generated client.
    response_types: Vec<TokenStream>,
    /// OpenAPI operation-registration token blocks for non-streaming HTTP routes.
    openapi_ops: Vec<TokenStream>,
    /// Accumulated per `#[message]` WebSocket handler method.
    ws_routes: Vec<WsRouteSpec>,
    /// `ws = P` — the WebSocket protocol this handlers block speaks.
    ws_protocol: Option<syn::Path>,
    /// `codec = C` — the body codec for this block's `#[message]`s.
    ws_codec: Option<syn::Path>,
}

/// The impl context `AxumHandlers` needs to emit, captured in the item pass.
pub(super) struct HandlerContext {
    pub(super) self_ty: Type,
    pub(super) self_ident: Ident,
    pub(super) paths: Paths,
    /// Generic type/const parameters for precise capture on streamed `impl Trait` returns.
    pub(super) capture: Vec<Ident>,
}

impl ParseKeyed for AxumHandlers {
    fn parse_keyed(&mut self, key: &Ident, input: ParseStream) -> syn::Result<bool> {
        match key.to_string().as_str() {
            "routes_slice" => {
                eat_eq(input)?;
                self.routes_slice = Some(input.parse()?);
                Ok(true)
            }
            "ws" => {
                eat_eq(input)?;
                self.ws_protocol = Some(input.parse()?);
                Ok(true)
            }
            "codec" => {
                eat_eq(input)?;
                self.ws_codec = Some(input.parse()?);
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    fn expected_keys() -> &'static [&'static str] {
        &["routes_slice", "ws", "codec"]
    }
}

impl ParseItem<ItemImpl> for AxumHandlers {
    fn parse_item(&mut self, item: &ItemImpl, paths: &Paths) -> syn::Result<()> {
        let self_ty = (*item.self_ty).clone();
        let self_ident = self_ty_ident(&self_ty)?;
        let capture = item
            .generics
            .params
            .iter()
            .filter_map(|param| match param {
                GenericParam::Type(ty) => Some(ty.ident.clone()),
                GenericParam::Const(konst) => Some(konst.ident.clone()),
                GenericParam::Lifetime(_) => None,
            })
            .collect();

        self.context = Some(HandlerContext {
            self_ty,
            self_ident,
            paths: paths.clone(),
            capture,
        });
        Ok(())
    }
}

impl AxumHandlers {
    /// The body codec for this block: its explicit `codec = C`, or `P::DefaultCodec`.
    fn resolve_ws_codec(&self, protocol: &syn::Path, paths: &Paths) -> TokenStream {
        match &self.ws_codec {
            Some(path) => quote!(#path),
            None => {
                let topic_protocol = paths.plugin("MessagingProtocol");
                quote!(<#protocol as #topic_protocol>::DefaultCodec)
            }
        }
    }
}

#[cfg(test)]
mod tests;
