use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{Attribute, Expr, Ident, Visibility};

use super::command::{self, ExpansionInput};
use super::model::CliDeclarations;
use super::policy::PolicyExpansion;
use crate::paths::Paths;

/// Inputs used to generate the named application's CLI surface.
pub(super) struct CliInput<'a> {
    pub(super) visibility: &'a Visibility,
    pub(super) ident: &'a Ident,
    pub(super) attributes: &'a [Attribute],
    pub(super) application_name: &'a Expr,
    pub(super) paths: &'a Paths,
    pub(super) declarations: &'a CliDeclarations,
    pub(super) policy: &'a PolicyExpansion,
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
    let cli_ident = format_ident!("{}Cli", ident, span = ident.span());
    let command_ident = format_ident!("{}Command", ident, span = ident.span());
    let parsed_command_ident =
        format_ident!("__{}SelectedApplicationCommand", ident, span = ident.span(),);
    let framework_cli_ident = format_ident!("__{}FrameworkCli", ident, span = ident.span());
    let framework_command_ident = format_ident!("__{}FrameworkCommand", ident, span = ident.span());
    let bootstrap_application_with_policy = input.paths.core("bootstrap_application_with_policy");
    let bootstrap_policy = input.paths.core("BootstrapPolicy");
    let app_host = input.paths.core("AppHost");
    let bootstrap_context = input.paths.core("BootstrapContext");
    let built = input.paths.core("Built");
    let cli_error = input.paths.core("CliError");
    let dispatch_cli_command = input.paths.core("dispatch_cli_command");
    let execution_mode = input.paths.core("ExecutionMode");
    let early_plugin_catalog = input.paths.core("EarlyPluginCatalog");
    let initial = input.paths.core("Initial");
    let resolve_host_plugin_catalog = input.paths.core("resolve_host_plugin_catalog");
    let retain_host_plugin_catalog = input.paths.core("retain_host_plugin_catalog");
    let prepare_cli_context = input.paths.core("prepare_cli_context");
    let selected_plugin_command = input.paths.core("SelectedPluginCliCommand");
    let clap: syn::Path = syn::parse_quote!(::clap);
    let host = quote!(#ident<#initial>);
    let bootstrap_ident = format_ident!("{}BootstrapArgs", ident, span = ident.span());
    let serve_definition = &input.policy.serve_definition;
    let serve_reference = format_ident!("Serve");
    let private_serve = format_ident!("Serve", span = ident.span());
    let bootstrap_type = &input.policy.bootstrap_type;
    let commands = command::expand(ExpansionInput {
        visibility,
        host_ident: ident,
        host: &host,
        entries: &input.declarations.commands,
        bootstrap_context: &bootstrap_context,
        cli_error: &cli_error,
        dispatch_cli_command: &dispatch_cli_command,
    })?;
    let command_variants = commands.variants;
    let command_dispatch_arms = commands.dispatch_arms;
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
    let has_application_commands =
        input.policy.serve_enabled || !input.declarations.commands.is_empty();
    let serve_default = input.policy.serve_default;
    let serve_attributes = &input.policy.serve_attributes;
    let serve_variant = input.policy.serve_enabled.then(|| {
        quote! {
            #serve_attributes
            #serve_definition,
        }
    });
    let framework_command_field = input.policy.serve_enabled.then(|| {
        quote! {
            #[command(subcommand)]
            command: Option<#framework_command_ident>,
        }
    });
    let framework_command_type = input.policy.serve_enabled.then(|| {
        quote! {
            #[derive(#clap::Subcommand)]
            enum #framework_command_ident {
                #serve_attributes
                #private_serve,
            }
        }
    });
    let serve_run_arm = input.policy.serve_enabled.then(|| {
        quote! {
            Self::#serve_reference => {
                let context = #prepare_cli_context::<#ident<#initial>, #built>(bootstrap).await?;
                let (context, app) = context.into_parts();
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

            impl #command_ident {
                async fn __dispatch(
                    &self,
                    bootstrap: #bootstrap_context,
                ) -> ::core::result::Result<(), #cli_error> {
                    match self {
                        #serve_run_arm
                        #command_dispatch_arms
                    }
                }
            }
        }
    });
    let parsed_command_type = if has_application_commands {
        quote!(type #parsed_command_ident = #command_ident;)
    } else {
        quote!(type #parsed_command_ident = ();)
    };
    let select_command = if input.policy.serve_default {
        quote!(command.or(::core::option::Option::Some(#command_ident::#serve_reference)))
    } else if has_application_commands {
        quote!(command)
    } else {
        quote!(::core::option::Option::<()>::None)
    };
    let command_destructure = has_application_commands.then(|| quote!(command,));
    let no_application_command = if has_application_commands {
        quote!(command.is_none())
    } else {
        quote!(true)
    };
    let missing_command_check = (!input.policy.serve_default).then(|| {
        quote! {
            if plugin_command.is_none() && #no_application_command {
                return Err(#clap::Error::new(#clap::error::ErrorKind::MissingSubcommand).into());
            }
        }
    });
    let application_dispatch = has_application_commands.then(|| {
        quote! {
            ::core::option::Option::None => {
                command
                    .as_ref()
                    .expect("Clap requires an application or plugin command")
                    .__dispatch(context)
                    .await?;
            }
        }
    });
    let absent_application_dispatch = (!has_application_commands).then(|| {
        quote! {
            ::core::option::Option::None => unreachable!("plugin-only CLI requires a plugin command"),
        }
    });
    let run_default_documentation = if input.policy.serve_default {
        " Omitting a command selects the generated `serve` command."
    } else if input.policy.serve_enabled {
        " The generated `serve` command is available but must be selected explicitly."
    } else {
        " This application has no generated framework `serve` command."
    };
    #[cfg(feature = "tooling")]
    let process_probe_dispatch = {
        let tooling_probe_argument = input.paths.core("tooling::TOOLING_PROBE_ARGUMENT");
        let probe_target_identity = input
            .paths
            .core("__private::probe_target_identity_from_env");
        let emit_probe_envelope = input.paths.core("__private::emit_probe_envelope_from_env");
        let install_panic_hook = input
            .paths
            .core("__private::install_process_probe_panic_hook");

        quote! {
            let arguments = ::std::env::args_os().collect::<::std::vec::Vec<_>>();

            if arguments.len() == 2
                && arguments[1] == ::std::ffi::OsStr::new(#tooling_probe_argument)
            {
                #install_panic_hook();

                let target = #probe_target_identity()
                    .map_err(|error| #cli_error::ToolingTarget(error))?;
                let envelope = Self::__upwell_tooling_probe(target)
                    .await
                    .map_err(|error| #cli_error::ToolingIdentity(error))?;
                let success = envelope.is_success();

                #emit_probe_envelope(&envelope)?;

                if success {
                    return Ok(());
                }

                ::std::process::exit(1);
            }
        }
    };
    #[cfg(not(feature = "tooling"))]
    let process_probe_dispatch = TokenStream::new();

    Ok(quote! {
        #bootstrap_type

        #(#documentation)*
        #[derive(#clap::Parser)]
        #[command(name = #cli_application_name, version)]
        #visibility struct #cli_ident {
            /// Framework bootstrap options.
            #[command(flatten)]
            pub bootstrap: #bootstrap_ident,

            #(#global_arg_fields)*

            #command_field
        }

        #[derive(#clap::Parser)]
        #[command(name = #cli_application_name, version)]
        struct #framework_cli_ident {
            #[command(flatten)]
            bootstrap: #bootstrap_ident,

            #framework_command_field
        }

        #framework_command_type

        #command_type

        #parsed_command_type

        #nested_command_types

        impl #ident<#initial> {
            fn __upwell_compose_cli(
                plugins: &mut #early_plugin_catalog,
            ) -> ::core::result::Result<#clap::Command, #cli_error> {
                let command = <#cli_ident as #clap::CommandFactory>::command();
                let framework = <#framework_cli_ident as #clap::CommandFactory>::command();
                let command = plugins.augment_cli(
                    command,
                    framework,
                    &[#(::core::any::TypeId::of::<#global_arg_types>()),*],
                    #serve_default,
                )?;

                Ok(command)
            }

            /// Parses the current process arguments and executes the selected generated command.
            ///
            /// Clap help, version, usage errors, suggestions, styling, and exit codes are rendered
            /// through Clap's normal process-facing `Error::exit` path. Successful parsing resolves
            /// framework bootstrap options, platform-native directories, selected config/profile,
            /// logging, and global app argument groups before any lifecycle hook runs. The selected
            /// command then drives only its required setup, prepared, or built lifecycle stage.
            #[doc = #run_default_documentation]
            ///
            /// # Errors
            ///
            /// Returns non-Clap `CliError` variants from bootstrap, lifecycle dispatch, command
            /// context validation, or an application command. Clap errors do not return: this
            /// method renders them and terminates the process with Clap's selected exit code.
            pub async fn run() -> ::core::result::Result<(), #cli_error> {
                #process_probe_dispatch

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
            /// same bootstrap resolution and lifecycle-aware dispatch as `run()`.
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
            /// phase; and `Command` for a typed leaf-command error, including a missing required
            /// bootstrap value, annotated with its full command path.
            pub async fn run_with<I, T>(args: I) -> ::core::result::Result<(), #cli_error>
            where
                I: ::core::iter::IntoIterator<Item = T>,
                T: ::core::convert::Into<::std::ffi::OsString> + ::core::clone::Clone,
            {
                let (context, command, plugin_command) = Self::__upwell_parse_cli(
                    args,
                    #execution_mode::Run,
                    true,
                )?;

                Self::__dispatch_cli(command, plugin_command, context).await
            }

            fn __upwell_parse_cli<I, T>(
                args: I,
                mode: #execution_mode,
                require_command: bool,
            ) -> ::core::result::Result<(
                #bootstrap_context,
                ::core::option::Option<#parsed_command_ident>,
                ::core::option::Option<#selected_plugin_command>,
            ), #cli_error>
            where
                I: ::core::iter::IntoIterator<Item = T>,
                T: ::core::convert::Into<::std::ffi::OsString> + ::core::clone::Clone,
            {
                let mut plugins = #resolve_host_plugin_catalog::<#ident<#initial>>()?;
                let mut parser = Self::__upwell_compose_cli(&mut plugins)?;
                let mut matches = parser.try_get_matches_from_mut(args)?;
                let bootstrap_sources = #bootstrap_ident::__sources(&matches);
                let plugin_args = plugins.parse_cli_args(&mut matches)?;
                let plugin_command = plugins.parse_cli_command(&mut matches)?;
                let cli = <#cli_ident as #clap::FromArgMatches>::from_arg_matches_mut(&mut matches)?;

                let #cli_ident {
                    bootstrap,
                    #(#global_arg_names,)*
                    #command_destructure
                } = cli;
                let command = #select_command;

                if require_command {
                    #missing_command_check
                }

                let bootstrap = bootstrap.__into_options(bootstrap_sources);
                let mut context = #bootstrap_application_with_policy(
                    #cli_application_name,
                    mode,
                    bootstrap,
                    #bootstrap_policy::new(
                        <#ident<#initial> as #app_host>::BOOTSTRAP_OWNS_DIRECTORIES,
                        <#ident<#initial> as #app_host>::BOOTSTRAP_OWNS_CONFIG,
                    ),
                )?;
                #retain_host_plugin_catalog(&mut context, plugins);
                #(context.insert(#global_arg_names);)*
                plugin_args.apply(&mut context);

                Ok((context, command, plugin_command))
            }

            async fn __dispatch_cli(
                command: ::core::option::Option<#parsed_command_ident>,
                plugin_command: ::core::option::Option<#selected_plugin_command>,
                context: #bootstrap_context,
            ) -> ::core::result::Result<(), #cli_error> {
                match plugin_command {
                    ::core::option::Option::Some(command) => command.run::<#ident<#initial>>(context).await?,
                    #application_dispatch
                    #absent_application_dispatch
                }

                Ok(())
            }
        }
    })
}
