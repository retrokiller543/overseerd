//! The controller extension: `AxumRouter`, the [`ComponentExt`] that turns `#[component]`
//! into `#[controller]`.
//!
//! `#[controller]` is `ComponentArgs<AxumRouter>`. The base component macro emits the
//! singleton component; `AxumRouter` appends the **controller** surface: the
//! `{Controller}Routes` slice (where `#[handlers]` blocks register their route groups), a
//! [`Controller`] impl whose `router` resolves the controller singleton once and merges the
//! groups under the base path, and a `ControllerDescriptor` in the `CONTROLLERS` slice. It
//! captures the base-resolved identity via [`ParseItem<ComponentContext>`] so the descriptor's
//! id/name match the component.

use proc_macro2::TokenStream;
use quote::{ToTokens, format_ident, quote};
use syn::parse::ParseStream;
use syn::punctuated::Punctuated;
use syn::{Ident, LitStr, Path, Token, bracketed};

use overseerd_macros_core::attr::ComponentArgs;
use overseerd_macros_core::paths::Paths;
use overseerd_macros_core::{ComponentContext, ComponentExt, NoExt, ParseItem, ParseKeyed, eat_eq};

/// The `#[controller]` args: the base component args extended with the [`AxumRouter`].
pub type ControllerComponent<T = NoExt> = ComponentArgs<AxumRouter<T>>;

/// The controller extension. Adds the `path` / `routes_slice` keyed args, captures the
/// component identity, and emits the controller surface. Nests `T` for further extension.
#[derive(Default)]
pub struct AxumRouter<T: ComponentExt = NoExt> {
    /// `path = ".."` — the base path every route in this controller mounts under.
    base: Option<LitStr>,

    /// `routes_slice = Ident` — the per-controller route slice (default `{Controller}Routes`).
    routes_slice: Option<Ident>,

    /// `ws = WsProto` — marks this a WebSocket controller speaking the named protocol. When set,
    /// the controller surface is the `WebsocketController` flavour (message routing) instead of the
    /// HTTP `Controller` one. The protocol is a raw path resolved in the caller's scope.
    ws: Option<syn::Path>,

    /// `middleware = [Type, ..]` — DI-backed `AxumMiddleware` singletons scoped to every route on
    /// this controller, first-listed outermost. Applied inside this controller's own router, so it
    /// nests inside global middleware and outside any per-route `middleware = [..]`.
    middleware: Vec<Path>,

    /// The base-resolved component identity (captured in the item pass).
    context: Option<ControllerContext>,

    /// The nested extension.
    inner: T,
}

/// The component identity (and resolved crate paths) `AxumRouter` keeps to emit its surface.
struct ControllerContext {
    ident: Ident,
    type_name: LitStr,
    id: LitStr,
    name: LitStr,
    paths: Paths,
    /// The controller type's `#[doc]` attributes, forwarded onto the generated client(s).
    docs: Vec<syn::Attribute>,
}

impl<T: ComponentExt> ParseKeyed for AxumRouter<T> {
    fn parse_keyed(&mut self, key: &Ident, input: ParseStream) -> syn::Result<bool> {
        match key.to_string().as_str() {
            "path" => {
                eat_eq(input)?;
                self.base = Some(input.parse()?);

                Ok(true)
            }

            "routes_slice" => {
                eat_eq(input)?;
                self.routes_slice = Some(input.parse()?);

                Ok(true)
            }

            "ws" => {
                eat_eq(input)?;
                self.ws = Some(input.parse()?);

                Ok(true)
            }

            "middleware" => {
                eat_eq(input)?;

                let content;
                bracketed!(content in input);
                self.middleware = Punctuated::<Path, Token![,]>::parse_terminated(&content)?
                    .into_iter()
                    .collect();

                Ok(true)
            }

            // Unknown to the controller — offer it to the nested extension.
            _ => self.inner.parse_keyed(key, input),
        }
    }

    fn expected_keys() -> &'static [&'static str] {
        &["path", "routes_slice", "ws", "middleware"]
    }
}

impl<T: ComponentExt> ParseItem<ComponentContext> for AxumRouter<T> {
    fn parse_item(&mut self, cx: &ComponentContext, paths: &Paths) -> syn::Result<()> {
        // A controller is always a singleton; reject a non-singleton scope on the component.
        if let Some(scope) = &cx.scope
            && scope
                .segments
                .last()
                .is_none_or(|seg| seg.ident != "Singleton")
        {
            return Err(syn::Error::new_spanned(
                scope,
                "#[controller] components are always singletons; `scope` is only valid on #[component]",
            ));
        }

        self.context = Some(ControllerContext {
            ident: cx.ident.clone(),
            type_name: cx.type_name.clone(),
            id: cx.id.clone(),
            name: cx.name.clone(),
            paths: paths.clone(),
            docs: cx.docs.clone(),
        });

        self.inner.parse_item(cx, paths)
    }
}

impl<T: ComponentExt> ComponentExt for AxumRouter<T> {
    fn defers_factory(&self) -> bool {
        // A controller's factory may be overridden by an `#[init]` in a `#[handlers]` impl, so
        // the base defers its eager field-DI assertion.
        true
    }

    fn asserts_wired(&self) -> bool {
        // A controller is a router-class component: force its `Wired` graph check at its own
        // definition, so a missing provider is caught there rather than at an `app!` listing.
        true
    }
}

impl<T: ComponentExt> ToTokens for AxumRouter<T> {
    fn to_tokens(&self, out: &mut TokenStream) {
        let Some(cx) = &self.context else {
            return;
        };

        // A `ws = P` controller emits the WebSocket surface (message routing, `WebsocketController`);
        // a plain controller emits the HTTP surface (`Controller`). They are mutually exclusive.
        match &self.ws {
            Some(protocol) => self.ws_tokens(cx, protocol, out),

            None => self.http_tokens(cx, out),
        }
    }
}

impl<T: ComponentExt> AxumRouter<T> {
    /// Emits the HTTP controller surface: the `{Controller}Routes` slice, the [`Controller`] impl,
    /// and the `ControllerDescriptor` registration.
    fn http_tokens(&self, cx: &ControllerContext, out: &mut TokenStream) {
        let ControllerContext {
            ident,
            type_name,
            id,
            name,
            paths,
            docs,
        } = cx;

        let base = match &self.base {
            Some(base) => quote!(#base),
            None => quote!(""),
        };
        let routes_slice = self
            .routes_slice
            .clone()
            .unwrap_or_else(|| format_ident!("{}Routes", ident));

        let app_runtime = paths.core("AppRuntime");
        let descriptor_trait = paths.core("Descriptor");
        let type_descriptor = paths.core("TypeDescriptor");
        let distributed_slice = paths.core("linkme::distributed_slice");
        let linkme_crate = paths.core("linkme");
        let axum = paths.plugin("axum");
        let controller_trait = paths.plugin("Controller");
        let controller_descriptor = paths.plugin("ControllerDescriptor");
        let controllers_slice = paths.plugin("CONTROLLERS");

        let controller_static = format_ident!(
            "__OVERSEERD_CONTROLLER_{}",
            ident.to_string().to_uppercase()
        );
        let client_struct = client_struct(ident, docs);
        // The wasm `#[wasm_bindgen]` wrapper *struct* + constructor is emitted once here (like the
        // generic client struct); each `#[handlers]` block contributes its methods onto it. Both
        // carry the controller's own `#[doc]`s, so the client is documented like its controller.
        let wasm_client_struct = if cfg!(feature = "client") {
            crate::client::wasm_client_struct(
                &format_ident!("{}Client", ident),
                docs,
                crate::client::WasmBackend::Http,
                paths,
            )
        } else {
            quote!()
        };
        let inner = &self.inner;

        let as_layer = paths.plugin("middleware::as_layer");

        // First-listed controller middleware is outermost within this controller: fold in reverse
        // so `.route_layer` (applied later) ends up wrapping earlier ones (axum's `Router::layer`
        // stacks last-applied-outermost).
        let middleware_tokens = self.middleware.iter().rev().map(|mw| {
            quote! {
                router = router.route_layer(#as_layer(
                    runtime.root().get::<#mw>().expect(
                        "middleware component missing from DI root — did you register it?",
                    ),
                ));
            }
        });

        // The controller's route base lives on the *client* too (ungated), so a generated client
        // method builds its URI from `Self::BASE` without depending on the server `Controller` trait
        // (whose `router()` returns an `axum::Router` — server-only). Emitted with the struct.
        let client_base = client_base(ident, &base);

        // The server surface — the route slice, the `Controller` impl, and the `CONTROLLERS`
        // registration — is gated out on wasm; the client struct + `BASE` above carry across.
        let server = overseerd_macros_core::gate::native_only(quote! {
            #[#distributed_slice]
            #[linkme(crate = #linkme_crate)]
            #[allow(non_upper_case_globals)]
            pub static #routes_slice: [fn(::std::sync::Arc<#ident>, & #app_runtime) -> #axum::Router];

            impl #controller_trait for #ident {
                const BASE: &'static str = #base;

                fn router(runtime: & #app_runtime) -> #axum::Router {
                    // The controller is a singleton built into the root scope at app build, so
                    // it resolves once here and is captured (cheaply, by `Arc`) in the route
                    // handlers — no per-request controller lookup.
                    let svc = runtime
                        .root()
                        .get::<#ident>()
                        .expect("controller singleton missing from the root scope");

                    let mut router = #axum::Router::new();

                    for group in #routes_slice {
                        router = router.merge(group(::std::sync::Arc::clone(&svc), runtime));
                    }

                    #(#middleware_tokens)*

                    if #base.is_empty() || #base == "/" {
                        router
                    } else {
                        #axum::Router::new().nest(#base, router)
                    }
                }
            }

            const _: () = {
                const __OVERSEERD_CONTROLLER_DESCRIPTOR: #controller_descriptor =
                    #controller_descriptor {
                        id: #id,
                        name: #name,
                        ty: #type_descriptor::of::<#ident>(#type_name),
                        base: #base,
                        router: <#ident as #controller_trait>::router,
                    };

                impl #descriptor_trait<#controller_descriptor> for #ident {
                    const DESCRIPTOR: #controller_descriptor = __OVERSEERD_CONTROLLER_DESCRIPTOR;
                }

                #[#distributed_slice(#controllers_slice)]
                #[linkme(crate = #linkme_crate)]
                static #controller_static: #controller_descriptor = __OVERSEERD_CONTROLLER_DESCRIPTOR;
            };

            #inner
        });

        out.extend(quote! {
            #client_struct

            #client_base

            #wasm_client_struct

            #server
        });
    }

    /// Emits the WebSocket controller surface: the `{Controller}WsRoutes` slice (message-route
    /// builders contributed by `#[handlers]` `#[message]` blocks), the [`WebsocketController`] impl
    /// naming `protocol`, and the `WsControllerDescriptor` registration. The upgrade path is *not*
    /// emitted here — it is supplied by `register_ws`.
    fn ws_tokens(&self, cx: &ControllerContext, protocol: &syn::Path, out: &mut TokenStream) {
        let ControllerContext {
            ident,
            type_name,
            id,
            name,
            paths,
            docs,
        } = cx;

        let ws_routes_slice = self
            .routes_slice
            .clone()
            .unwrap_or_else(|| format_ident!("{}WsRoutes", ident));

        let app_runtime = paths.core("AppRuntime");
        let descriptor_trait = paths.core("Descriptor");
        let type_descriptor = paths.core("TypeDescriptor");
        let distributed_slice = paths.core("linkme::distributed_slice");
        let linkme_crate = paths.core("linkme");
        let ws_controller_trait = paths.plugin("WebsocketController");
        let ws_descriptor = paths.plugin("WsControllerDescriptor");
        let ws_controllers_slice = paths.plugin("WS_CONTROLLERS");
        let ws_route = paths.plugin("WsRoute");

        // Every message route is typed to this controller's protocol `P`. The per-controller slice
        // and the `ws_routes` builder are monomorphic in `P`; only the link-time `WS_CONTROLLERS`
        // slice (which can't hold a generic descriptor) erases the routes vector to `Box<dyn Any>`.
        let ws_route_p = quote!(#ws_route<#protocol>);

        let controller_static = format_ident!(
            "__OVERSEERD_WS_CONTROLLER_{}",
            ident.to_string().to_uppercase()
        );
        let client_struct = client_struct(ident, docs);
        // A `#[controller(ws = Stomp)]` gets a wasm SEND client over the shared STOMP socket. A
        // JsonWs controller has no wasm ws transport yet, so it emits no wasm binding struct.
        let wasm_client_struct =
            if cfg!(feature = "client") && crate::handlers::is_stomp_protocol(Some(protocol)) {
                crate::client::wasm_client_struct(
                    &format_ident!("{}Client", ident),
                    docs,
                    crate::client::WasmBackend::Stomp,
                    paths,
                )
            } else {
                quote!()
            };
        let inner = &self.inner;

        // The ws controller's server surface (the route slice, the `WebsocketController` impl, and
        // the `WS_CONTROLLERS` registration) is gated out on wasm; the client structs above carry
        // across, so a wasm client gets the generated SEND client with no server code.
        let server = overseerd_macros_core::gate::native_only(quote! {
            #[#distributed_slice]
            #[linkme(crate = #linkme_crate)]
            #[allow(non_upper_case_globals)]
            pub static #ws_routes_slice:
                [fn(::std::sync::Arc<#ident>) -> ::std::vec::Vec<#ws_route_p>];

            impl #ws_controller_trait for #ident {
                type Protocol = #protocol;

                fn ws_routes(runtime: & #app_runtime) -> ::std::vec::Vec<#ws_route_p> {
                    // The controller is a singleton built into the root scope at app build, so it
                    // resolves once here and is captured (cheaply, by `Arc`) in each message
                    // handler — no per-message controller lookup.
                    let svc = runtime
                        .root()
                        .get::<#ident>()
                        .expect("ws controller singleton missing from the root scope");

                    let mut routes = ::std::vec::Vec::new();

                    for group in #ws_routes_slice {
                        routes.extend(group(::std::sync::Arc::clone(&svc)));
                    }

                    routes
                }
            }

            const _: () = {
                // Erases the typed `ws_routes` product to `Box<dyn Any>` for the non-generic
                // `WS_CONTROLLERS` slice; `WsControllerDescriptor::routes_for::<P>` recovers it.
                fn __overseerd_ws_routes_erased(
                    runtime: & #app_runtime,
                ) -> ::std::boxed::Box<dyn ::std::any::Any + ::std::marker::Send> {
                    ::std::boxed::Box::new(
                        <#ident as #ws_controller_trait>::ws_routes(runtime),
                    )
                }

                const __OVERSEERD_WS_CONTROLLER_DESCRIPTOR: #ws_descriptor =
                    #ws_descriptor {
                        id: #id,
                        name: #name,
                        ty: #type_descriptor::of::<#ident>(#type_name),
                        protocol: || ::std::any::TypeId::of::<#protocol>(),
                        protocol_name: || ::std::any::type_name::<#protocol>(),
                        routes: __overseerd_ws_routes_erased,
                    };

                impl #descriptor_trait<#ws_descriptor> for #ident {
                    const DESCRIPTOR: #ws_descriptor = __OVERSEERD_WS_CONTROLLER_DESCRIPTOR;
                }

                #[#distributed_slice(#ws_controllers_slice)]
                #[linkme(crate = #linkme_crate)]
                static #controller_static: #ws_descriptor = __OVERSEERD_WS_CONTROLLER_DESCRIPTOR;
            };

            #inner
        });

        out.extend(quote! {
            #client_struct

            #wasm_client_struct

            #server
        });
    }
}

/// The per-controller client struct (its methods are contributed by `#[handlers]` blocks). One
/// per controller, gated on the `client` feature; the generated methods land in capability
/// `impl` blocks the framework emits. `docs` are the controller type's own `#[doc]` attributes,
/// emitted on the struct so the client documents identically to the controller it came from.
fn client_struct(ident: &Ident, docs: &[syn::Attribute]) -> TokenStream {
    if !cfg!(feature = "client") {
        return quote!();
    }

    let client_ident = format_ident!("{}Client", ident);

    quote! {
        #(#docs)*
        pub struct #client_ident<C>(pub C);

        impl<C> #client_ident<C> {
            /// Wraps an HTTP client transport (e.g. `ReqwestClient`).
            pub fn new(transport: C) -> Self {
                Self(transport)
            }
        }
    }
}

/// The client-side route base: `impl<C> {Controller}Client<C> { const BASE }`. Emitted (ungated)
/// alongside [`client_struct`] so a generated client method builds its URI from `Self::BASE`,
/// decoupled from the server-only `Controller` trait. Gated on the `client` feature like the struct.
fn client_base(ident: &Ident, base: &TokenStream) -> TokenStream {
    if !cfg!(feature = "client") {
        return quote!();
    }

    let client_ident = format_ident!("{}Client", ident);

    quote! {
        impl<C> #client_ident<C> {
            /// The controller's route base, prepended to each generated method's path.
            pub const BASE: &'static str = #base;
        }
    }
}
