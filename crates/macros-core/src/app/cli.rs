use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{Attribute, Expr, Ident, Visibility};

use super::command::{self, ExpansionInput};
use super::model::CliDeclarations;
use crate::paths::Paths;

/// Inputs used to generate the named application's CLI surface.
pub(super) struct CliInput<'a> {
    pub(super) visibility: &'a Visibility,
    pub(super) ident: &'a Ident,
    pub(super) attributes: &'a [Attribute],
    pub(super) application_name: &'a Expr,
    pub(super) paths: &'a Paths,
    pub(super) has_serve: bool,
    pub(super) declarations: &'a CliDeclarations,
}

/// Expands the generated application CLI and typed command dispatcher.
pub(super) fn expand(input: CliInput<'_>) -> syn::Result<TokenStream> {
    if !input.has_serve && input.declarations.commands.is_empty() {
        if !input.declarations.args.is_empty() {
            return Err(syn::Error::new_spanned(
                input.application_name,
                "global CLI arguments require at least one command or a `serve` phase",
            ));
        }

        return Ok(TokenStream::new());
    }

    let cli_application_name = match input.application_name {
        Expr::Lit(expression) => match &expression.lit {
            syn::Lit::Str(name) => name,
            _ => {
                return Err(syn::Error::new_spanned(
                    input.application_name,
                    "named apps with a generated CLI require a string literal `name`",
                ));
            }
        },
        _ => {
            return Err(syn::Error::new_spanned(
                input.application_name,
                "named apps with a generated CLI require a string literal `name`",
            ));
        }
    };
    let visibility = input.visibility;
    let ident = input.ident;
    let documentation = input
        .attributes
        .iter()
        .filter(|attribute| attribute.path().is_ident("doc"));
    let cli_ident = format_ident!("{}Cli", ident);
    let command_ident = format_ident!("{}Command", ident);
    let bootstrap_application_with_policy = input.paths.core("bootstrap_application_with_policy");
    let bootstrap_options = input.paths.core("BootstrapOptions");
    let bootstrap_policy = input.paths.core("BootstrapPolicy");
    let app_host = input.paths.core("AppHost");
    let build_host_context = input.paths.core("build_host_context");
    let built = input.paths.core("Built");
    let cli_error = input.paths.core("CliError");
    let validate_cli = input.paths.core("validate_cli");
    let cli_command = input.paths.core("CliCommand");
    let command_context = input.paths.core("CommandContext");
    let command_error = input.paths.core("CommandError");
    let command_phase = input.paths.core("CommandPhase");
    let execution_mode = input.paths.core("ExecutionMode");
    let initial = input.paths.core("Initial");
    let prepare_host_context = input.paths.core("prepare_host_context");
    let setup_host_context = input.paths.core("setup_host_context");
    let clap: syn::Path = syn::parse_quote!(::clap);
    let host = quote!(#ident<#initial>);
    let commands = command::expand(ExpansionInput {
        visibility,
        host_ident: ident,
        host: &host,
        entries: &input.declarations.commands,
        cli_command: &cli_command,
        command_context: &command_context,
        command_error: &command_error,
        command_phase: &command_phase,
    })?;
    let command_variants = commands.variants;
    let command_phase_arms = commands.phase_arms;
    let command_run_arms = commands.run_arms;
    let nested_command_types = commands.nested_types;
    let global_arg_fields = input.declarations.args.iter().map(|entry| {
        let attributes = &entry.attributes;
        let alias = &entry.alias;
        let ty = &entry.ty;

        quote! {
            #(#attributes)*
            #[command(flatten)]
            pub #alias: #ty,
        }
    });
    let global_arg_names = input
        .declarations
        .args
        .iter()
        .map(|entry| &entry.alias)
        .collect::<Vec<_>>();
    let command_field = if input.has_serve {
        quote! {
            /// Application command. Defaults to `serve` when omitted.
            #[command(subcommand)]
            pub command: Option<#command_ident>,
        }
    } else {
        quote! {
            /// Application command.
            #[command(subcommand)]
            pub command: #command_ident,
        }
    };
    let select_command = if input.has_serve {
        quote!(command.unwrap_or(#command_ident::Serve))
    } else {
        quote!(command)
    };
    let serve_variant = input.has_serve.then(|| {
        quote! {
            /// Build and serve the application.
            Serve,
        }
    });
    let serve_phase_arm = input
        .has_serve
        .then(|| quote!(Self::Serve => #command_phase::Built,));
    let serve_run_arm = input.has_serve.then(|| {
        quote! {
            Self::Serve => {
                let (context, app) = context.into_built()?;
                let application = #ident::<#built>::from_state((context, app));

                application.serve().await?;

                Ok(())
            }
        }
    });

    Ok(quote! {
        #(#documentation)*
        #[derive(#clap::Parser)]
        #[command(name = #cli_application_name, version)]
        #visibility struct #cli_ident {
            /// Framework bootstrap options.
            #[command(flatten)]
            pub bootstrap: #bootstrap_options,

            #(#global_arg_fields)*

            #command_field
        }

        /// Generated application commands.
        #[derive(#clap::Subcommand)]
        #visibility enum #command_ident {
            #serve_variant
            #command_variants
        }

        impl #cli_command<#ident<#initial>> for #command_ident {
            type Error = #cli_error;

            fn phase(&self) -> #command_phase {
                match self {
                    #serve_phase_arm
                    #command_phase_arms
                }
            }

            async fn run(
                &self,
                context: #command_context<#ident<#initial>>,
            ) -> ::core::result::Result<(), Self::Error> {
                match self {
                    #serve_run_arm
                    #command_run_arms
                }
            }
        }

        #nested_command_types

        impl #ident<#initial> {
            /// Parses the current process arguments and executes the selected generated command.
            ///
            /// Clap help, version, usage errors, suggestions, styling, and exit codes are rendered
            /// through Clap's normal process-facing `Error::exit` path. Successful parsing resolves
            /// framework bootstrap options, platform-native directories, selected config/profile,
            /// logging, and global app argument groups before any lifecycle hook runs. The selected
            /// command then drives only its required setup, prepared, or built lifecycle stage.
            /// Omitting a command selects `serve` when the app declares a serve phase.
            ///
            /// # Errors
            ///
            /// Returns non-Clap `CliError` variants from bootstrap, lifecycle dispatch, command
            /// context validation, or an application command. Clap errors do not return: this
            /// method renders them and terminates the process with Clap's selected exit code.
            pub async fn run() -> ::core::result::Result<(), #cli_error> {
                match Self::run_with(::std::env::args_os()).await {
                    Err(#cli_error::Clap(error)) => error.exit(),
                    result => result,
                }
            }

            /// Parses an explicit argument iterator and executes the selected generated command.
            ///
            /// Unlike `run()`, this never prints or exits on Clap errors; it returns typed
            /// `CliError::Clap` values for tests, embedding, or custom process policies. Before
            /// parsing, it validates generated and flattened command names, aliases, argument/group
            /// IDs, long/short options, and inherited global options. On success it performs the
            /// same bootstrap resolution and lifecycle-aware dispatch as `run_cli()`.
            ///
            /// # Type parameters
            ///
            /// - `I` is any iterable argument source.
            /// - `T` is each argument value and must convert into `OsString`; `Clone` is required by
            ///   Clap's non-exiting parser API.
            ///
            /// # Errors
            ///
            /// Returns `Definition` for conflicting generated or flattened Clap declarations;
            /// `Clap` for invalid arguments or requested help/version; `Bootstrap` for directory,
            /// config/profile, logging, color, or tracing resolution; `Lifecycle` for a tagged app
            /// phase; `CommandContext` for inconsistent generated phase state; and `Command` for a
            /// typed leaf-command error annotated with its full command path.
            pub async fn run_with<I, T>(args: I) -> ::core::result::Result<(), #cli_error>
            where
                I: ::core::iter::IntoIterator<Item = T>,
                T: ::core::convert::Into<::std::ffi::OsString> + ::core::clone::Clone,
            {
                let mut command = <#cli_ident as #clap::CommandFactory>::command();

                #validate_cli(&command)?;

                let mut matches = command.try_get_matches_from_mut(args)?;
                let cli = <#cli_ident as #clap::FromArgMatches>::from_arg_matches_mut(&mut matches)?;

                Self::run_cli(cli).await
            }

            /// Resolves bootstrap state and dispatches an already parsed generated CLI value.
            ///
            /// This does not run Clap parsing or command-definition validation. It resolves CLI,
            /// environment, profile, base-config, and default precedence into `BootstrapContext`,
            /// uses platform-native project directories unless an explicit manager is declared,
            /// inserts each flattened global argument group by type, and selects the command's
            /// required lifecycle phase. Setup commands run setup and tracing finalization only;
            /// configured commands additionally perform discovery, registration, config
            /// resolution, graph/config validation, and construction planning; built commands also
            /// construct components, root DI, runtime, and protocol and run `after_build`.
            /// Non-serve commands never enter the serve phase.
            ///
            /// # Errors
            ///
            /// Returns `Bootstrap` when directories, config/profile precedence, logging, color, or
            /// tracing cannot be resolved; `Lifecycle` when setup, configure, prepare, build,
            /// after-build, or serve fails; `CommandContext` when generated dispatch receives the
            /// wrong lifecycle state; and `Command` when the selected leaf returns its typed error.
            /// `Definition` and `Clap` are not produced because parsing already completed.
            pub async fn run_cli(cli: #cli_ident) -> ::core::result::Result<(), #cli_error> {
                let #cli_ident {
                    bootstrap,
                    #(#global_arg_names,)*
                    command,
                } = cli;
                let command = #select_command;
                let phase = <#command_ident as #cli_command<#ident<#initial>>>::phase(&command);
                let mut context = #bootstrap_application_with_policy(
                    #cli_application_name,
                    #execution_mode::Run,
                    bootstrap,
                    #bootstrap_policy::new(
                        <#ident<#initial> as #app_host>::BOOTSTRAP_OWNS_DIRECTORIES,
                        <#ident<#initial> as #app_host>::BOOTSTRAP_OWNS_CONFIG,
                    ),
                )?;
                #(context.insert(#global_arg_names);)*
                let context = match phase {
                    #command_phase::Setup => {
                        let context = #setup_host_context::<#ident<#initial>>(context).await?;

                        #command_context::from_setup(context)
                    }
                    #command_phase::Configured => {
                        let (context, app) = #prepare_host_context::<#ident<#initial>>(context).await?;

                        #command_context::from_configured(context, app)
                    }
                    #command_phase::Built => {
                        let (context, app) = #build_host_context::<#ident<#initial>>(context).await?;

                        #command_context::from_built(context, app)
                    }
                };

                <#command_ident as #cli_command<#ident<#initial>>>::run(&command, context).await?;

                Ok(())
            }
        }
    })
}
