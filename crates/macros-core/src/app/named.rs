use proc_macro2::TokenStream;
use quote::quote;

use super::NamedApp;
use super::builder;
#[cfg(feature = "cli")]
use super::cli::{self, CliInput};
use super::phase;
use crate::paths::Paths;

/// Expands a reusable named application host and its lifecycle.
pub(super) fn expand(input: NamedApp) -> TokenStream {
    let NamedApp {
        visibility,
        ident,
        mut assembly,
    } = input;
    let phases = std::mem::take(&mut assembly.phases);
    let has_config_manager = assembly.config_manager.is_some();
    let has_directories_manager = assembly.directories_manager.is_some();
    let protocol = assembly.protocol.clone();
    let paths = Paths::overseerd().resolve(assembly.overseerd.clone(), assembly.krate.clone());
    let app_builder = paths.core("AppBuilder");
    let config_error = paths.core("ConfigError");
    let app = paths.core("App");
    let app_host = paths.core("AppHost");
    let bootstrap_context = paths.core("BootstrapContext");
    let execution_mode = paths.core("ExecutionMode");
    let host_error = paths.core("HostError");
    let lifecycle_phase = paths.core("LifecyclePhase");
    let phase_error = paths.core("PhaseError");
    let prepared_app = paths.core("PreparedApp");
    let configure_bootstrap_config = paths.core("configure_bootstrap_config");
    let configure_bootstrap_directories = paths.core("configure_bootstrap_directories");
    let finalize_bootstrap = paths.core("finalize_bootstrap");
    let setup_call = phase::result(
        phases.setup.as_ref(),
        quote!(context),
        &[quote!(context)],
        quote!(#lifecycle_phase::Setup),
        &phase_error,
    );
    let configure_call = phase::call(
        phases.configure.as_ref(),
        quote!(builder),
        &[quote!(&mut context), quote!(builder)],
        quote!(#lifecycle_phase::Configure),
        &phase_error,
    );
    let before_build_call = phase::call(
        phases.before_build.as_ref(),
        quote!(builder),
        &[quote!(&mut context), quote!(builder)],
        quote!(#lifecycle_phase::BeforeBuild),
        &phase_error,
    );
    let after_build_call = phase::call(
        phases.after_build.as_ref(),
        quote!(app),
        &[quote!(&mut context), quote!(app)],
        quote!(#lifecycle_phase::AfterBuild),
        &phase_error,
    );
    let serve_method = phases.serve.as_ref().map(|serve| {
        let serve_call = phase::call(
            Some(serve),
            quote!(()),
            &[quote!(context), quote!(app)],
            quote!(#lifecycle_phase::Serve),
            &phase_error,
        );

        quote! {
            /// Runs the application-defined serve lifecycle phase.
            pub async fn serve_with(
                context: #bootstrap_context,
                app: #app<#protocol>,
            ) -> ::core::result::Result<(), #phase_error> {
                let output = #serve_call;

                Ok(output)
            }
        }
    });
    let bootstrap_directories = (cfg!(feature = "cli") && !has_directories_manager).then(|| {
        quote! {
            let builder = #configure_bootstrap_directories(&mut context, builder);
        }
    });
    let bootstrap_config = (cfg!(feature = "cli") && !has_config_manager).then(|| {
        quote! {
            let builder = #configure_bootstrap_config(&mut context, builder);
        }
    });
    let bootstrap_finalize = cfg!(feature = "cli").then(|| {
        quote! {
            #finalize_bootstrap(&mut context)
                .map_err(|source| #phase_error::new(#lifecycle_phase::Setup, source))?;
        }
    });

    #[cfg(feature = "cli")]
    let cli = match cli::expand(CliInput {
        visibility: &visibility,
        ident: &ident,
        application_name: &assembly.name,
        paths: &paths,
        bootstrap_owns_config: !has_config_manager,
        bootstrap_owns_directories: !has_directories_manager,
        has_serve: phases.serve.is_some(),
    }) {
        Ok(cli) => cli,
        Err(error) => return error.into_compile_error(),
    };

    #[cfg(not(feature = "cli"))]
    let cli = TokenStream::new();

    let builder = builder::expand(assembly);

    quote! {
        #[doc = "Generated application host."]
        #visibility struct #ident;

        impl #ident {
            /// Creates a new configured application builder.
            pub fn builder() -> ::core::result::Result<#app_builder<#protocol>, #config_error> {
                ::core::result::Result::Ok(#builder)
            }

            /// Creates the lifecycle bootstrap context.
            pub async fn setup(mode: #execution_mode) -> ::core::result::Result<#bootstrap_context, #phase_error> {
                let context = #bootstrap_context::new(mode);

                Self::__overseerd_setup_context(context).await
            }

            async fn __overseerd_setup_context(
                context: #bootstrap_context,
            ) -> ::core::result::Result<#bootstrap_context, #phase_error> {
                #setup_call
            }

            /// Configures and validates the app without constructing ordinary components.
            pub async fn prepare(
                mode: #execution_mode,
            ) -> ::core::result::Result<(#bootstrap_context, #prepared_app<#protocol>), #phase_error> {
                let context = #bootstrap_context::new(mode);

                Self::__overseerd_prepare_context(context).await
            }

            async fn __overseerd_prepare_context(
                context: #bootstrap_context,
            ) -> ::core::result::Result<(#bootstrap_context, #prepared_app<#protocol>), #phase_error> {
                let mut context = Self::__overseerd_setup_context(context).await?;
                #bootstrap_finalize
                let builder = Self::builder()
                    .map_err(|source| #phase_error::new(#lifecycle_phase::Configure, source))?;
                #bootstrap_directories
                #bootstrap_config
                let builder = #configure_call;
                let builder = #before_build_call;
                let prepared = builder
                    .prepare()
                    .map_err(|source| #phase_error::new(#lifecycle_phase::Prepare, source))?;

                Ok((context, prepared))
            }

            /// Runs the host lifecycle through component and protocol construction.
            pub async fn build(
                mode: #execution_mode,
            ) -> ::core::result::Result<(#bootstrap_context, #app<#protocol>), #phase_error> {
                if mode.is_tooling() {
                    return Err(#phase_error::new(
                        #lifecycle_phase::Build,
                        #host_error::ToolingConstruction,
                    ));
                }

                let context = #bootstrap_context::new(mode);

                Self::__overseerd_build_context(context).await
            }

            async fn __overseerd_build_context(
                context: #bootstrap_context,
            ) -> ::core::result::Result<(#bootstrap_context, #app<#protocol>), #phase_error> {
                if context.mode().is_tooling() {
                    return Err(#phase_error::new(
                        #lifecycle_phase::Build,
                        #host_error::ToolingConstruction,
                    ));
                }

                let (mut context, prepared) = Self::__overseerd_prepare_context(context).await?;
                let app = prepared
                    .build()
                    .await
                    .map_err(|source| #phase_error::new(#lifecycle_phase::Build, source))?;
                let app = #after_build_call;

                Ok((context, app))
            }

            #serve_method
        }

        impl #app_host for #ident {
            type Protocol = #protocol;

            fn builder() -> ::core::result::Result<#app_builder<#protocol>, #config_error> {
                Self::builder()
            }
        }

        #cli
    }
}
