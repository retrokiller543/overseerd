use quote::quote;
use syn::parse2;

use super::super::NamedApp;
#[cfg(feature = "cli")]
use super::super::expand;

fn parse_error(input: proc_macro2::TokenStream) -> String {
    match parse2::<NamedApp>(input) {
        Ok(_) => panic!("input unexpectedly parsed"),
        Err(error) => error.to_string(),
    }
}

#[test]
fn parses_global_args_and_nested_commands() {
    parse2::<NamedApp>(quote! {
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
        .contains(
            "command name or alias `print-config` is claimed by application command `print-config` and application command `print-config`"
        )
    );
    assert!(
        parse_error(quote! {
            app Example {
                name: "example",
                protocol: Protocol,
                serve = serve,
                commands: { serve: ServeCommand },
            }
        })
        .contains(
            "command name or alias `serve` is claimed by framework reserved slot `serve` and application command `serve`"
        )
    );
    assert!(
        parse_error(quote! {
            app Example {
                name: "example",
                protocol: Protocol,
                commands: { api: { help: HelpCommand } },
            }
        })
        .contains(
            "command name or alias `help` is claimed by framework reserved slot `help` and application command `api help`"
        )
    );
}

#[test]
fn rejects_literal_framework_policy_collisions_with_canonical_owners() {
    for (input, expected) in [
        (
            quote! {
                app LongAliasCollision {
                    name: "long-alias-collision",
                    protocol: Protocol,
                    cli: { config: { aliases: ["profile"] } },
                }
            },
            "long option `profile` is claimed by framework reserved slot `config` and framework reserved slot `profile`",
        ),
        (
            quote! {
                app ShortCollision {
                    name: "short-collision",
                    protocol: Protocol,
                    cli: { config: { short: 'p' } },
                }
            },
            "short option `p` is claimed by framework reserved slot `config` and framework reserved slot `profile`",
        ),
        (
            quote! {
                app HelpCollision {
                    name: "help-collision",
                    protocol: Protocol,
                    cli: { config: { name: "help" } },
                }
            },
            "long option `help` is claimed by framework reserved slot `config` and framework reserved slot `help`",
        ),
        (
            quote! {
                app VersionCollision {
                    name: "version-collision",
                    protocol: Protocol,
                    cli: { log: { short: 'V' } },
                }
            },
            "short option `V` is claimed by framework reserved slot `log` and framework reserved slot `version`",
        ),
    ] {
        assert!(parse_error(input).contains(expected));
    }
}

#[test]
fn rejects_literal_application_command_metadata_collisions() {
    for (input, expected) in [
        (
            quote! {
                app ServeAliasCollision {
                    name: "serve-alias-collision",
                    protocol: Protocol,
                    cli: { serve: { aliases: ["run"] } },
                    commands: { run: RunCommand },
                    serve = serve,
                }
            },
            "command name or alias `run` is claimed by framework reserved slot `serve` and application command `run`",
        ),
        (
            quote! {
                app ApplicationAliasCollision {
                    name: "application-alias-collision",
                    protocol: Protocol,
                    commands: {
                        #[command(alias = "show")]
                        inspect: InspectCommand,
                        show: ShowCommand,
                    },
                }
            },
            "command name or alias `show` is claimed by application command `inspect` and application command `show`",
        ),
        (
            quote! {
                app LongFlagCollision {
                    name: "long-flag-collision",
                    protocol: Protocol,
                    commands: {
                        #[command(long_flag_alias = "profile")]
                        inspect: InspectCommand,
                    },
                }
            },
            "long option `profile` is claimed by framework reserved slot `profile` and application command `inspect`",
        ),
        (
            quote! {
                app ShortFlagCollision {
                    name: "short-flag-collision",
                    protocol: Protocol,
                    commands: {
                        #[command(short_flag = 'h')]
                        inspect: InspectCommand,
                    },
                }
            },
            "short option `h` is claimed by framework reserved slot `help` and application command `inspect`",
        ),
        (
            quote! {
                app ServeVariantCollision {
                    name: "serve-variant-collision",
                    protocol: Protocol,
                    cli: { serve: { name: "start" } },
                    commands: { serve: ServeCommand },
                    serve = serve,
                }
            },
            "generated Rust variant `Serve` is claimed by framework reserved slot `serve` and application command `serve`",
        ),
    ] {
        assert!(parse_error(input).contains(expected));
    }
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
    let input = parse2::<NamedApp>(quote! {
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
    assert!(output.contains("plugins . augment_cli"));
    assert!(output.contains("plugins . parse_cli_command"));
    assert!(output.contains("impl ExampleApiUsersCommand"));
    assert!(output.contains("dispatch_cli_command :: < Example < :: overseerd :: Initial >"));
    assert!(
        !output.contains(
            "CliCommand < Example < :: overseerd :: Initial > > for ExampleApiUsersCommand"
        )
    );
    assert!(!output.contains("derive (Clone"));
}
