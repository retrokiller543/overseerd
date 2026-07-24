use quote::quote;
use syn::parse2;

use super::super::AppInput;
#[cfg(feature = "cli")]
use super::super::expand;

fn parse_error(input: proc_macro2::TokenStream) -> String {
    match parse2::<AppInput>(input) {
        Ok(_) => panic!("input unexpectedly parsed"),
        Err(error) => error.to_string(),
    }
}

#[test]
fn parses_global_args_and_nested_commands() {
    parse2::<AppInput>(quote! {
        app Example {
            name: "example",
            protocol: Protocol,
            args: {
                /// Shared output options.
                output: OutputArgs,
            },
            commands: {
                /// Database migrations.
                #[command(alias = "db", display_order = 10)]
                migrate: MigrateCommand,
                api: {
                    users: {
                        list: ListUsersCommand,
                    },
                },
            },
        }
    })
    .expect("CLI declarations parse");
}

#[test]
fn rejects_structural_and_unknown_command_attributes() {
    assert!(
        parse_error(quote! {
            app Example {
                name: "example",
                protocol: Protocol,
                commands: {
                    #[command(flatten)]
                    inspect: InspectCommand,
                },
            }
        })
        .contains("setting `flatten` would change generated command dispatch")
    );
    assert!(
        parse_error(quote! {
            app Example {
                name: "example",
                protocol: Protocol,
                commands: {
                    #[command(multicall = true)]
                    inspect: InspectCommand,
                },
            }
        })
        .contains("setting `multicall` is not supported")
    );
}

#[test]
fn rejects_duplicate_and_reserved_command_names() {
    assert!(
        parse_error(quote! {
            app Example {
                name: "example",
                protocol: Protocol,
                commands: { print_config: First, print_config: Second },
            }
        })
        .contains("duplicate command name `print-config`")
    );
    assert!(
        parse_error(quote! {
            app Example {
                name: "example",
                protocol: Protocol,
                commands: { serve: ServeCommand },
            }
        })
        .contains("command name `serve` is reserved")
    );
    assert!(
        parse_error(quote! {
            app Example {
                name: "example",
                protocol: Protocol,
                commands: { api: { help: HelpCommand } },
            }
        })
        .contains("command name `help` is reserved by Clap")
    );
}

#[test]
fn rejects_empty_namespaces_and_duplicate_global_types() {
    assert!(
        parse_error(quote! {
            app Example {
                name: "example",
                protocol: Protocol,
                commands: { api: {} },
            }
        })
        .contains("command namespace `api` cannot be empty")
    );
    assert!(
        parse_error(quote! {
            app Example {
                name: "example",
                protocol: Protocol,
                commands: {},
            }
        })
        .contains("`commands` cannot be empty")
    );
    assert!(
        parse_error(quote! {
            app Example {
                name: "example",
                protocol: Protocol,
                args: { first: SharedArgs, second: SharedArgs },
            }
        })
        .contains("global argument type can only be registered once")
    );
}

#[test]
fn rejects_generated_parser_field_collisions() {
    assert!(
        parse_error(quote! {
            app Example {
                name: "example",
                protocol: Protocol,
                args: { command: SharedArgs },
                commands: { inspect: InspectCommand },
            }
        })
        .contains("global argument alias `command` is reserved")
    );
}

#[cfg(feature = "cli")]
#[test]
fn generates_one_parser_subcommand_field_and_nested_delegation() {
    let input = parse2::<AppInput>(quote! {
        app Example {
            name: "example",
            protocol: Protocol,
            args: { output: OutputArgs },
            commands: {
                api: {
                    users: { list: ListUsersCommand },
                },
            },
        }
    })
    .expect("command app parses");
    let output = expand(input).to_string();
    let parser_start = output
        .find("struct ExampleCli")
        .expect("parser is generated");
    let parser_end = output[parser_start..]
        .find("enum ExampleCommand")
        .map(|offset| parser_start + offset)
        .expect("top command follows parser");
    let parser = &output[parser_start..parser_end];

    assert_eq!(parser.matches("command (subcommand)").count(), 1);
    assert!(parser.contains("command (flatten)"));
    assert!(output.contains("enum ExampleApiCommand"));
    assert!(output.contains("enum ExampleApiUsersCommand"));
    assert!(
        output.contains(
            "CliCommand < Example < :: overseerd :: Initial > > for ExampleApiUsersCommand"
        )
    );
    assert!(!output.contains("derive (Clone"));
}

#[test]
fn rejects_cli_declarations_in_legacy_apps() {
    assert!(
        parse_error(quote! {
            name: "legacy",
            protocol: Protocol,
            commands: { migrate: MigrateCommand },
        })
        .contains("CLI declarations require a named app definition")
    );
}
