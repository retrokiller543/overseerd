use proc_macro2::TokenStream;
use quote::quote;

use super::super::model::PhaseInput;

/// Expands the application-specific `AppHost::serve` implementation.
#[allow(clippy::too_many_arguments)]
pub(super) fn expand(
    serve: Option<&PhaseInput>,
    host: &syn::Ident,
    protocol: &syn::Type,
    app: &syn::Path,
    bootstrap_context: &syn::Path,
    lifecycle_phase: &syn::Path,
    phase_error: &syn::Path,
    resolve_host_dependency: &syn::Path,
) -> TokenStream {
    let Some(serve) = serve else {
        return TokenStream::new();
    };

    match serve {
        PhaseInput::Path(path) => quote! {
            async fn serve(
                context: #bootstrap_context,
                app: #app<#protocol>,
            ) -> ::core::result::Result<(), #phase_error> {
                #path(context, app)
                    .await
                    .map_err(|source| #phase_error::new(#lifecycle_phase::Serve, source))
            }
        },
        PhaseInput::Inline { arguments, body } => {
            let context = &arguments[0].ident;
            let app_name = &arguments[1].ident;
            let dependencies = &arguments[2..];
            let dependency_bindings = dependencies.iter().map(|argument| {
                let ident = &argument.ident;
                let ty = argument.ty.as_ref().expect("validated injected serve type");

                quote! {
                    let #ident: #ty = #resolve_host_dependency(&#app_name, stringify!(#host))
                        .await
                        .map_err(|source| #phase_error::new(#lifecycle_phase::Serve, source))?;
                }
            });

            quote! {
                async fn serve(
                    #context: #bootstrap_context,
                    #app_name: #app<#protocol>,
                ) -> ::core::result::Result<(), #phase_error> {
                    #(#dependency_bindings)*

                    (async move #body)
                        .await
                        .map_err(|source| #phase_error::new(#lifecycle_phase::Serve, source))
                }
            }
        }
    }
}
