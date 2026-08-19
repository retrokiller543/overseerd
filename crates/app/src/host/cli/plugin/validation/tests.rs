use clap::{Arg, Command};

use super::framework_owns_collision;
use crate::validate_cli;

#[test]
fn framework_globals_own_customized_aliases_in_application_children() {
    let framework = Command::new("example").arg(
        Arg::new("profiles")
            .long("environment")
            .alias("profile")
            .short('e')
            .global(true),
    );
    let long_collision = framework
        .clone()
        .subcommand(Command::new("inspect").arg(Arg::new("local-profile").long("profile")));
    let short_collision = framework
        .clone()
        .subcommand(Command::new("inspect").arg(Arg::new("local-environment").short('e')));
    let long_error = validate_cli(&long_collision).expect_err("long alias collides");
    let short_error = validate_cli(&short_collision).expect_err("short option collides");

    assert!(framework_owns_collision(
        &long_collision,
        &framework,
        &long_error
    ));
    assert!(framework_owns_collision(
        &short_collision,
        &framework,
        &short_error
    ));
}

#[test]
fn framework_serve_owns_its_customized_name_and_aliases() {
    let framework = Command::new("example").subcommand(Command::new("start").alias("run"));
    let collision = framework
        .clone()
        .subcommand(Command::new("inspect").alias("run"));
    let error = validate_cli(&collision).expect_err("serve alias collides");

    assert!(framework_owns_collision(&collision, &framework, &error));
}

#[test]
fn generated_help_is_framework_owned_in_application_children() {
    let framework = Command::new("example").version("1.0.0");
    let collision = Command::new("example")
        .version("1.0.0")
        .subcommand(Command::new("inspect").arg(Arg::new("custom-help").long("help")));
    let error = validate_cli(&collision).expect_err("generated nested help collides");

    assert!(framework_owns_collision(&collision, &framework, &error));
}
