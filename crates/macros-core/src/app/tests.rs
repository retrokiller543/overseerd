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
            overseerd: ::framework,
            crate: ::plugin,
        }
    };

    parse2::<AppInput>(input).expect("complete named app parses");
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
    assert!(output.contains("pub fn builder () -> :: overseerd :: AppBuilder < Protocol >"));
    assert_eq!(output.matches("with_component").count(), 1);
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
