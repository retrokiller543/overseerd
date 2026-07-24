use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{Expr, Ident, Visibility};

use crate::paths::Paths;

/// Inputs used to generate the named application's CLI surface.
pub(super) struct CliInput<'a> {
    pub(super) visibility: &'a Visibility,
    pub(super) ident: &'a Ident,
    pub(super) application_name: &'a Expr,
    pub(super) paths: &'a Paths,
    pub(super) bootstrap_owns_config: bool,
    pub(super) bootstrap_owns_directories: bool,
    pub(super) has_serve: bool,
}

/// Expands the generated CLI when the application defines a serve phase.
pub(super) fn expand(input: CliInput<'_>) -> syn::Result<TokenStream> {
    if !input.has_serve {
        return Ok(TokenStream::new());
    }

    let cli_application_name = match input.application_name {
        Expr::Lit(expression) => match &expression.lit {
            syn::Lit::Str(name) => name,
            _ => {
                return Err(syn::Error::new_spanned(
                    input.application_name,
                    "named apps with `serve` require a string literal `name` for generated CLI metadata",
                ));
            }
        },
        _ => {
            return Err(syn::Error::new_spanned(
                input.application_name,
                "named apps with `serve` require a string literal `name` for generated CLI metadata",
            ));
        }
    };
    let visibility = input.visibility;
    let ident = input.ident;
    let bootstrap_owns_config = input.bootstrap_owns_config;
    let bootstrap_owns_directories = input.bootstrap_owns_directories;
    let cli_ident = format_ident!("{}Cli", ident);
    let command_ident = format_ident!("{}Command", ident);
    let bootstrap_application_with_policy = input.paths.core("bootstrap_application_with_policy");
    let bootstrap_options = input.paths.core("BootstrapOptions");
    let bootstrap_policy = input.paths.core("BootstrapPolicy");
    let cli_error = input.paths.core("CliError");
    let execution_mode = input.paths.core("ExecutionMode");
    let clap: syn::Path = syn::parse_quote!(::clap);

    Ok(quote! {
        /// Generated application command line.
        #[derive(Debug, #clap::Parser)]
        #[command(name = #cli_application_name, version, about = env!("CARGO_PKG_DESCRIPTION"))]
        #visibility struct #cli_ident {
            /// Framework bootstrap options.
            #[command(flatten)]
            pub bootstrap: #bootstrap_options,

            /// Application command. Defaults to `serve` when omitted.
            #[command(subcommand)]
            pub command: Option<#command_ident>,
        }

        /// Generated application commands.
        #[derive(Clone, Copy, Debug, Eq, PartialEq, #clap::Subcommand)]
        #visibility enum #command_ident {
            /// Build and serve the application.
            Serve,
        }

        impl #ident {
            /// Parses and dispatches process arguments.
            pub async fn run() -> ::core::result::Result<(), #cli_error> {
                match Self::run_with(::std::env::args_os()).await {
                    Err(#cli_error::Clap(error))
                        if matches!(
                            error.kind(),
                            #clap::error::ErrorKind::DisplayHelp
                                | #clap::error::ErrorKind::DisplayVersion
                        ) => {
                            error.print()?;

                            Ok(())
                        }
                    result => result,
                }
            }

            /// Parses and dispatches an explicit argument iterator without exiting.
            pub async fn run_with<I, T>(args: I) -> ::core::result::Result<(), #cli_error>
            where
                I: ::core::iter::IntoIterator<Item = T>,
                T: ::core::convert::Into<::std::ffi::OsString> + ::core::clone::Clone,
            {
                let cli = <#cli_ident as #clap::Parser>::try_parse_from(args)?;

                Self::run_cli(cli).await
            }

            /// Dispatches an already parsed generated CLI value.
            pub async fn run_cli(cli: #cli_ident) -> ::core::result::Result<(), #cli_error> {
                let context = #bootstrap_application_with_policy(
                    #cli_application_name,
                    #execution_mode::Run,
                    cli.bootstrap,
                    #bootstrap_policy::new(
                        #bootstrap_owns_directories,
                        #bootstrap_owns_config,
                    ),
                )?;
                let (context, app) = Self::__overseerd_build_context(context).await?;

                match cli.command.unwrap_or(#command_ident::Serve) {
                    #command_ident::Serve => Self::serve_with(context, app).await?,
                }

                Ok(())
            }
        }
    })
}
