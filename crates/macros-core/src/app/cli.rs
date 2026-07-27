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
    let framework_cli_ident = format_ident!("__{}FrameworkCli", ident);
    let bootstrap_application_with_policy = input.paths.core("bootstrap_application_with_policy");
    let bootstrap_options = input.paths.core("BootstrapOptions");
    let bootstrap_policy = input.paths.core("BootstrapPolicy");
    let app_host = input.paths.core("AppHost");
    let build_host_context = input.paths.core("build_host_context");
    let built = input.paths.core("Built");
    let cli_error = input.paths.core("CliError");
    let cli_command = input.paths.core("CliCommand");
    let command_context = input.paths.core("CommandContext");
    let command_error = input.paths.core("CommandError");
    let command_phase = input.paths.core("CommandPhase");
    let execution_mode = input.paths.core("ExecutionMode");
    let early_plugin_catalog = input.paths.core("EarlyPluginCatalog");
    let initial = input.paths.core("Initial");
    let prepare_host_context = input.paths.core("prepare_host_context");
    let resolve_host_plugin_catalog = input.paths.core("resolve_host_plugin_catalog");
    let retain_host_plugin_catalog = input.paths.core("retain_host_plugin_catalog");
    let parsed_plugin_args = input.paths.core("ParsedPluginArgs");
    let selected_plugin_command = input.paths.core("SelectedPluginCliCommand");
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
    let global_arg_types = input
        .declarations
        .args
        .iter()
        .map(|entry| &entry.ty)
        .collect::<Vec<_>>();
    let has_application_commands = input.has_serve || !input.declarations.commands.is_empty();
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
    let command_field = has_application_commands.then(|| {
        quote! {
            /// Application command.
            #[command(subcommand)]
            pub command: Option<#command_ident>,
        }
    });
    let command_type = has_application_commands.then(|| {
        quote! {
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
        }
    });
    let select_command = if input.has_serve {
        quote!(command.or(::core::option::Option::Some(#command_ident::Serve)))
    } else if has_application_commands {
        quote!(command)
    } else {
        quote!(::core::option::Option::<()>::None)
    };
    let command_destructure = has_application_commands.then(|| quote!(command,));
    let no_application_command = if has_application_commands {
        quote!(cli.command.is_none())
    } else {
        quote!(true)
    };
    let missing_command_check = (!input.has_serve).then(|| {
        quote! {
            if plugin_command.is_none() && #no_application_command {
                return Err(#clap::Error::new(#clap::error::ErrorKind::MissingSubcommand).into());
            }
        }
    });
    let application_phase = has_application_commands.then(|| {
        quote! {
            ::core::option::Option::None => {
                <#command_ident as #cli_command<#ident<#initial>>>::phase(
                    command.as_ref().expect("Clap requires an application or plugin command")
                )
            }
        }
    });
    let absent_application_phase = (!has_application_commands).then(|| {
        quote! {
            ::core::option::Option::None => unreachable!("plugin-only CLI requires a plugin command"),
        }
    });
    let application_dispatch = has_application_commands.then(|| {
        quote! {
            ::core::option::Option::None => {
                <#command_ident as #cli_command<#ident<#initial>>>::run(
                    command.as_ref().expect("Clap requires an application or plugin command"),
                    context,
                ).await?;
            }
        }
    });
    let absent_application_dispatch = (!has_application_commands).then(|| {
        quote! {
            ::core::option::Option::None => unreachable!("plugin-only CLI requires a plugin command"),
        }
    });
    let run_cli = has_application_commands.then(|| {
        quote! {
            /// Dispatches an already parsed generated application command.
            ///
            /// Runtime-contributed plugin commands and plugin argument groups require `run_with`,
            /// because they are not representable in the statically generated CLI value.
            pub async fn run_cli(cli: #cli_ident) -> ::core::result::Result<(), #cli_error> {
                let plugins = #resolve_host_plugin_catalog::<#ident<#initial>>()?;

                Self::__run_cli(
                    cli,
                    plugins,
                    #parsed_plugin_args::default(),
                    ::core::option::Option::None,
                ).await
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

        #[derive(#clap::Parser)]
        #[command(name = #cli_application_name, version)]
        struct #framework_cli_ident {
            #[command(flatten)]
            bootstrap: #bootstrap_options,
        }

        #command_type

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
                let plugins = #resolve_host_plugin_catalog::<#ident<#initial>>()?;
                let mut command = <#cli_ident as #clap::CommandFactory>::command();
                let framework = <#framework_cli_ident as #clap::CommandFactory>::command();

                command = plugins.augment_cli(
                    command,
                    framework,
                    &[#(::core::any::TypeId::of::<#global_arg_types>()),*],
                )?;

                let mut matches = command.try_get_matches_from_mut(args)?;
                let plugin_args = plugins.parse_cli_args(&mut matches)?;
                let plugin_command = plugins.parse_cli_command(&mut matches)?;
                let cli = <#cli_ident as #clap::FromArgMatches>::from_arg_matches_mut(&mut matches)?;

                Self::__run_cli(cli, plugins, plugin_args, plugin_command).await
            }

            #run_cli

            async fn __run_cli(
                cli: #cli_ident,
                plugins: #early_plugin_catalog,
                plugin_args: #parsed_plugin_args,
                plugin_command: ::core::option::Option<#selected_plugin_command>,
            ) -> ::core::result::Result<(), #cli_error> {
                #missing_command_check

                let #cli_ident {
                    bootstrap,
                    #(#global_arg_names,)*
                    #command_destructure
                } = cli;
                let command = #select_command;
                let phase = match &plugin_command {
                    ::core::option::Option::Some(command) => command.phase(),
                    #application_phase
                    #absent_application_phase
                };
                let mut context = #bootstrap_application_with_policy(
                    #cli_application_name,
                    #execution_mode::Run,
                    bootstrap,
                    #bootstrap_policy::new(
                        <#ident<#initial> as #app_host>::BOOTSTRAP_OWNS_DIRECTORIES,
                        <#ident<#initial> as #app_host>::BOOTSTRAP_OWNS_CONFIG,
                    ),
                )?;
                #retain_host_plugin_catalog(&mut context, plugins);
                #(context.insert(#global_arg_names);)*
                plugin_args.apply(&mut context);
                let context = match phase {
                    #command_phase::Setup => {
                        let context = #setup_host_context::<#ident<#initial>>(context).await?;

                        #command_context::<#ident<#initial>>::from_setup(context)
                    }
                    #command_phase::Configured => {
                        let (context, app) = #prepare_host_context::<#ident<#initial>>(context).await?;

                        #command_context::<#ident<#initial>>::from_configured(context, app)
                    }
                    #command_phase::Built => {
                        let (context, app) = #build_host_context::<#ident<#initial>>(context).await?;

                        #command_context::<#ident<#initial>>::from_built(context, app)
                    }
                };

                match plugin_command {
                    ::core::option::Option::Some(command) => command.run(context).await?,
                    #application_dispatch
                    #absent_application_dispatch
                }

                Ok(())
            }
        }
    })
}
