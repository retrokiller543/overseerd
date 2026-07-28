use proc_macro2::TokenStream;
use quote::quote;
use syn::{Expr, Ident};

use crate::paths::Paths;

/// Expands the target-local tooling identity and preparation entry.
pub(super) fn expand(
    ident: &Ident,
    application_name: &Expr,
    paths: &Paths,
) -> syn::Result<TokenStream> {
    let application_name = match application_name {
        Expr::Lit(expression) => match &expression.lit {
            syn::Lit::Str(name) => name.clone(),
            _ => return Err(literal_name_error(application_name)),
        },
        _ => return Err(literal_name_error(application_name)),
    };
    let probe_target_identity = paths.core("tooling::ProbeTargetIdentity");
    let source_location = paths.core("tooling::SourceLocation");
    let probe_envelope = paths.core("tooling::ProbeEnvelope");
    let identity_error = paths.core("tooling::IdentityValidationError");
    let catch_probe_panic = paths.core("__private::catch_probe_panic");
    let initial = paths.core("Initial");
    #[cfg(feature = "cli")]
    let run_probe = {
        let app_host = paths.core("AppHost");
        let bootstrap_application = paths.core("bootstrap_application_with_policy");
        let bootstrap_policy = paths.core("BootstrapPolicy");
        let cli_error = paths.core("CliError");
        let execution_mode = paths.core("ExecutionMode");
        let probe_host = paths.core("__private::probe_bootstrapped_host");
        let resolve_host_plugin_catalog = paths.core("resolve_host_plugin_catalog");
        let bootstrap_ident = quote::format_ident!("{}BootstrapArgs", ident, span = ident.span());
        let framework_cli_ident =
            quote::format_ident!("__{}FrameworkCli", ident, span = ident.span());

        quote! {
            let context = (|| -> ::core::result::Result<_, #cli_error> {
                let mut command = <#framework_cli_ident as ::clap::CommandFactory>::command();
                let mut matches = command.try_get_matches_from_mut([#application_name])?;
                let sources = #bootstrap_ident::__sources(&matches);
                let cli = <#framework_cli_ident as ::clap::FromArgMatches>::from_arg_matches_mut(
                    &mut matches,
                )?;

                Ok(#bootstrap_application(
                    #application_name,
                    #execution_mode::Tooling,
                    cli.bootstrap.__into_options(sources),
                    #bootstrap_policy::new(
                        <#ident<#initial> as #app_host>::BOOTSTRAP_OWNS_DIRECTORIES,
                        <#ident<#initial> as #app_host>::BOOTSTRAP_OWNS_CONFIG,
                    ),
                )?)
            })();
            let plugins = #resolve_host_plugin_catalog::<#ident<#initial>>()
                .map_err(#cli_error::from)
                .and_then(|mut plugins| {
                    Self::__overseerd_compose_cli(&mut plugins)?;

                    Ok(plugins)
                });

            #probe_host::<#ident<#initial>>(identity, context, plugins).await
        }
    };
    #[cfg(not(feature = "cli"))]
    let run_probe = {
        let probe_host = paths.core("__private::probe_host");

        quote!(#probe_host::<#ident<#initial>>(identity).await)
    };

    Ok(quote! {
        impl #ident<#initial> {
            /// Prepares and projects this application for an explicitly selected Cargo target.
            ///
            /// This target-local seam is independent of Clap. A tooling-only thin binary may call
            /// it directly and emit the returned envelope through the response-file contract.
            /// Its unwind boundary does not replace an embedding process's global panic hook, so
            /// callers requiring panic-payload secrecy must use the generated dedicated process
            /// invocation instead.
            ///
            /// # Errors
            ///
            /// Returns an identity error when generated declaration identity is incomplete.
            #[doc(hidden)]
            pub async fn tooling_probe(
                target: #probe_target_identity,
            ) -> ::core::result::Result<#probe_envelope, #identity_error> {
                let identity = target.document_identity(
                    (#application_name).to_string(),
                    #source_location {
                        file: ::std::file!().to_string(),
                        line: ::core::option::Option::Some(::std::line!()),
                        column: ::core::option::Option::Some(::std::column!()),
                    },
                )?;
                let panic_identity = identity.clone();
                let probe = async move { #run_probe };

                Ok(#catch_probe_panic(panic_identity, probe).await)
            }

            /// Internal generated alias used by the process runner and crate-local tests.
            #[doc(hidden)]
            pub(crate) async fn __overseerd_tooling_probe(
                target: #probe_target_identity,
            ) -> ::core::result::Result<#probe_envelope, #identity_error> {
                Self::tooling_probe(target).await
            }
        }
    })
}

fn literal_name_error(application_name: &Expr) -> syn::Error {
    syn::Error::new_spanned(
        application_name,
        "named apps with tooling support require a string literal `name`",
    )
}
