use proc_macro2::{Span, TokenStream};
use quote::quote;

use super::AppAssembly;
use super::model::{ConfigSettings, DirSettings, ManagerSource};
use crate::{di, paths::Paths};

/// Expands the protocol-specific application builder.
pub(super) fn expand(input: AppAssembly) -> TokenStream {
    let AppAssembly {
        name,
        protocol,
        services,
        components,
        configs,
        config_manager,
        directories_manager,
        middleware,
        guards,
        error_handler,
        overseerd,
        krate,
        phases: _,
    } = input;
    let paths = Paths::overseerd().resolve(overseerd, krate);
    let config_tys = configs.iter().map(|entry| &entry.ty);
    let config_paths = configs.iter().map(|entry| &entry.path);
    let app_ty = paths.core("App");
    let assertion = expand_service_assertion(&services, &paths);
    let (directories_binding, directories_call, directories_available) =
        match expand_directories(&directories_manager, &paths) {
            Ok(expansion) => expansion,
            Err(error) => return error,
        };
    let (config_binding, config_call) =
        match expand_config(&config_manager, directories_available, &paths) {
            Ok(expansion) => expansion,
            Err(error) => return error,
        };
    let error_handler = error_handler.into_iter();

    quote! {
        {
            #assertion

            #directories_binding
            #config_binding

            #app_ty::<#protocol>::builder(#name)
                .auto_discover()
                #(.with_component(#components))*
                #(.config::<#config_tys>(#config_paths))*
                #config_call
                #directories_call
                #(.middleware(#middleware))*
                #(.guard(#guards))*
                #(.error_handler(#error_handler))*
        }
    }
}

fn expand_service_assertion(services: &[syn::Type], paths: &Paths) -> TokenStream {
    if di::enabled() && !services.is_empty() {
        let wired = paths.core("Wired");

        return quote! {
            const _: () = {
                fn __overseerd_assert_wired<T: #wired>() {}

                fn __overseerd_app_check() {
                    #(__overseerd_assert_wired::<#services>();)*
                }
            };
        };
    }

    TokenStream::new()
}

fn expand_directories(
    manager: &Option<ManagerSource<DirSettings>>,
    paths: &Paths,
) -> Result<(TokenStream, TokenStream, bool), TokenStream> {
    let directories_path = paths.core("DirectoriesManager");

    match manager {
        Some(ManagerSource::Instance(expression)) => Ok((
            quote!(let __overseerd_directories = #expression;),
            quote!(.directories(__overseerd_directories)),
            true,
        )),
        Some(ManagerSource::Configure(settings)) => {
            let expression = if let Some(root) = &settings.root {
                quote!(#directories_path::from_path(#root))
            } else if let Some(app) = &settings.app {
                quote!(#directories_path::for_app(#app))
            } else {
                return Err(error("a `directories` config block needs `app` or `root`"));
            };

            Ok((
                quote!(let __overseerd_directories = #expression;),
                quote!(.directories(__overseerd_directories)),
                true,
            ))
        }
        None => Ok((TokenStream::new(), TokenStream::new(), false)),
    }
}

fn expand_config(
    manager: &Option<ManagerSource<ConfigSettings>>,
    directories_available: bool,
    paths: &Paths,
) -> Result<(TokenStream, TokenStream), TokenStream> {
    let config_manager_path = paths.core("ConfigManager");
    let config_dynamic = paths.core("config::Dynamic");

    match manager {
        Some(ManagerSource::Instance(expression)) => Ok((
            quote!(let __overseerd_config = #expression;),
            quote!(.config_source(__overseerd_config)),
        )),
        Some(ManagerSource::Configure(settings)) => {
            let base = if let Some(source) = &settings.source {
                quote!(#source)
            } else if directories_available {
                let profiles = match &settings.profiles {
                    Some(profiles) => quote!(#profiles),
                    None => quote!(&[]),
                };

                quote!(#config_manager_path::<#config_dynamic>::load_from(&__overseerd_directories, #profiles)?)
            } else {
                return Err(error(
                    "a `config` block without `source` requires a `directories` manager to load from",
                ));
            };
            let mut chain = base;

            if settings.sighup {
                chain = quote!(#chain.reload_on_sighup());
            }

            if settings.watch {
                chain = quote!(#chain.watch_config());
            }

            if let Some(debounce) = &settings.debounce {
                chain = quote!(#chain.config_reload_debounce(#debounce));
            }

            Ok((
                quote!(let __overseerd_config = #chain;),
                quote!(.config_source(__overseerd_config)),
            ))
        }
        None => Ok((TokenStream::new(), TokenStream::new())),
    }
}

fn error(message: &str) -> TokenStream {
    syn::Error::new(Span::call_site(), message).to_compile_error()
}
