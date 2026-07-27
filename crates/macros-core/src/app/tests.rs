use quote::quote;
use syn::parse2;

use super::{AppInput, expand};

fn parse_error(input: proc_macro2::TokenStream) -> String {
    match parse2::<AppInput>(input) {
        Ok(_) => panic!("input unexpectedly parsed"),
        Err(error) => error.to_string(),
    }
}

#[test]
fn parses_named_app_visibilities() {
    for input in [
        quote!(app Private { name: "private", protocol: Protocol }),
        quote!(pub app Public { name: "public", protocol: Protocol }),
        quote!(pub(crate) app CrateOnly { name: "crate", protocol: Protocol }),
    ] {
        parse2::<AppInput>(input).expect("named app parses");
    }
}

#[test]
fn rejects_non_documentation_attributes_on_named_apps() {
    let error = match parse2::<AppInput>(quote! {
        #[cfg(feature = "disabled")]
        app Example {
            name: "example",
            protocol: Protocol,
        }
    }) {
        Ok(_) => panic!("partial generated cfg attributes unexpectedly parsed"),
        Err(error) => error,
    };

    assert!(
        error
            .to_string()
            .contains("only documentation attributes are supported on generated applications")
    );
}

#[test]
fn parses_complete_named_app() {
    let input = quote! {
        pub app Example {
            name: "example",
            protocol: Protocol,
            services: [Service],
            components: [component()],
            configs: [Config => "app.config"],
            managers: {
                directories: directories(),
                config: config(),
            },
            middleware: [middleware()],
            guards: [guard()],
            error_handler: error_handler(),
            plugins: [Plugin, replace SLOT => Replacement, suppress OPTIONAL_SLOT],
            overseerd: ::framework,
            crate: ::plugin,
        }
    };

    parse2::<AppInput>(input).expect("complete named app parses");
}

#[test]
fn expands_static_plugin_declarations_on_the_host() {
    let input = parse2::<AppInput>(quote! {
        app Example {
            name: "example",
            protocol: Protocol,
            plugins: [Plugin, replace SLOT => Replacement, suppress OPTIONAL_SLOT],
        }
    })
    .expect("static plugins parse");
    let output = expand(input).to_string();

    assert!(output.contains("fn declare_plugins"));
    assert!(output.contains("plugins . register :: < Plugin >"));
    assert!(output.contains("plugins . replace_with :: < Replacement > (SLOT)"));
    assert!(output.contains("plugins . suppress (OPTIONAL_SLOT)"));
}

#[test]
fn rejects_duplicate_named_app_keys() {
    for (input, key) in [
        (
            quote!(app Example { name: "one", name: "two", protocol: Protocol }),
            "name",
        ),
        (
            quote!(app Example { name: "one", protocol: First, protocol: Second }),
            "protocol",
        ),
        (
            quote!(app Example { name: "one", protocol: Protocol, components: [], components: [] }),
            "components",
        ),
    ] {
        let error = parse_error(input);

        assert_eq!(error, format!("duplicate app key `{key}`"));
    }
}

#[test]
fn rejects_incomplete_and_unknown_named_apps() {
    assert!(parse_error(quote!(app Example { protocol: Protocol })).contains("requires a `name`"));
    assert!(parse_error(quote!(app Example { name: "example" })).contains("requires a `protocol"));
    assert!(
        parse_error(quote!(app Example {
            name: "example",
            protocol: Protocol,
            unknown: true,
        }))
        .contains("unknown `app!` key `unknown`")
    );
}

#[test]
fn expands_named_host_and_builder() {
    let input = parse2::<AppInput>(quote! {
        pub app Example {
            name: "example",
            protocol: Protocol,
            components: [component()],
        }
    })
    .expect("named app parses");
    let output = expand(input).to_string();

    assert!(output.contains("pub struct Example"));
    assert!(
        output.contains("Stage : :: overseerd :: AppStage < Protocol > = :: overseerd :: Initial")
    );
    assert!(output.contains("impl Example < :: overseerd :: Initial >"));
    assert!(output.contains("impl Example < :: overseerd :: Setup >"));
    assert!(output.contains("impl Example < :: overseerd :: PreBuild >"));
    assert!(output.contains("impl Example < :: overseerd :: Built >"));
    assert!(!output.contains("__overseerd_setup"));
    assert!(!output.contains("__overseerd_prepare"));
    assert!(!output.contains("__overseerd_build"));
    assert!(output.contains(
        "pub fn builder () -> :: core :: result :: Result < :: overseerd :: AppBuilder < Protocol > , :: overseerd :: ConfigError >"
    ));
    assert!(output.contains("Result :: Ok"));
    assert_eq!(output.matches("with_component").count(), 1);
}

#[test]
fn named_builder_propagates_directory_config_errors() {
    let input = parse2::<AppInput>(quote! {
        pub app Example {
            name: "example",
            protocol: Protocol,
            managers: {
                directories: { root: root() },
                config: {},
            },
        }
    })
    .expect("named app parses");
    let output = expand(input).to_string();

    assert!(output.contains("ConfigManager :: < :: overseerd :: config :: Dynamic > :: load_from"));
    assert!(output.contains("?"));
    assert!(output.contains("Result < :: overseerd :: AppBuilder"));
}

#[test]
fn keeps_legacy_expression_form() {
    let input = parse2::<AppInput>(quote! {
        name: "legacy",
        protocol: Protocol,
    })
    .expect("legacy app parses");
    let output = expand(input).to_string();

    assert!(output.starts_with('{'));
    assert!(output.contains("App :: < Protocol > :: builder"));
}

#[test]
fn rejects_static_plugins_in_legacy_apps() {
    assert!(
        parse_error(quote! {
            name: "legacy",
            protocol: Protocol,
            plugins: [Plugin],
        })
        .contains("static plugin declarations require a named app definition")
    );
}

#[test]
fn parses_external_and_inline_lifecycle_phases() {
    let input = quote! {
        pub app Example {
            name: "example",
            protocol: Protocol,
            setup = setup,
            configure(context, builder) { Ok(builder) },
            before_build = before_build,
            after_build(context, app) { Ok(app) },
            serve = serve,
        }
    };

    parse2::<AppInput>(input).expect("lifecycle phases parse");
}

#[test]
fn rejects_invalid_lifecycle_phase_forms() {
    assert!(
        parse_error(quote! {
            app Example {
                name: "example",
                protocol: Protocol,
                setup: {},
            }
        })
        .contains("declarative lifecycle settings are reserved")
    );
    assert!(
        parse_error(quote! {
            app Example {
                name: "example",
                protocol: Protocol,
                setup(context, extra) { Ok(context) },
            }
        })
        .contains("`setup` expects 1 argument")
    );
}

#[cfg(feature = "cli")]
#[test]
fn generated_cli_requires_literal_application_name() {
    let input = parse2::<AppInput>(quote! {
        app DynamicName {
            name: String::from("dynamic"),
            protocol: Protocol,
            serve = serve,
        }
    })
    .expect("app syntax parses");
    let output = expand(input).to_string();

    assert!(output.contains("require a string literal `name`"));
}

#[test]
fn generated_cli_preserves_explicit_manager_policy() {
    let input = parse2::<AppInput>(quote! {
        app ExplicitManagers {
            name: "explicit-managers",
            protocol: Protocol,
            managers: {
                directories: directories,
                config: config,
            },
            serve = serve,
        }
    })
    .expect("app syntax parses");
    let output = expand(input).to_string();

    assert!(!output.contains("configure_bootstrap_directories"));
    assert!(!output.contains("configure_bootstrap_config"));
}

#[cfg(not(feature = "cli"))]
#[test]
fn named_app_without_cli_omits_bootstrap_helpers() {
    let input = parse2::<AppInput>(quote! {
        app NoCli {
            name: "no-cli",
            protocol: Protocol,
        }
    })
    .expect("app syntax parses");
    let output = expand(input).to_string();

    assert!(!output.contains("finalize_bootstrap"));
    assert!(!output.contains("configure_bootstrap"));
}

#[cfg(feature = "cli")]
#[test]
fn named_app_without_application_commands_generates_plugin_cli_shell() {
    let input = parse2::<AppInput>(quote! {
        app PluginOnly {
            name: "plugin-only",
            protocol: Protocol,
            plugins: [Plugin],
        }
    })
    .expect("plugin-only app parses");
    let output = expand(input).to_string();

    assert!(output.contains("struct PluginOnlyCli"));
    assert!(output.contains("fn run_with"));
    assert!(!output.contains("enum PluginOnlyCommand"));
    assert!(!output.contains("fn run_cli"));
}
