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
use syn::{Ident, LitStr};

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

            // Unknown to the controller — offer it to the nested extension.
            _ => self.inner.parse_keyed(key, input),
        }
    }

    fn expected_keys() -> &'static [&'static str] {
        &["path", "routes_slice"]
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

        let ControllerContext {
            ident,
            type_name,
            id,
            name,
            paths,
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
        let inner = &self.inner;

        out.extend(quote! {
            #[#distributed_slice]
            #[linkme(crate = #linkme_crate)]
            #[allow(non_upper_case_globals)]
            pub static #routes_slice: [fn(::std::sync::Arc<#ident>) -> #axum::Router];

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
                        router = router.merge(group(::std::sync::Arc::clone(&svc)));
                    }

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
    }
}
