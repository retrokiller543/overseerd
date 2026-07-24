use clap::{Arg, ArgGroup, Command};

use super::validate_cli;

#[test]
fn accepts_distinct_local_and_inherited_arguments() {
    let command = Command::new("example")
        .arg(Arg::new("profile").long("profile").global(true))
        .subcommand(Command::new("inspect").arg(Arg::new("format").long("format")));

    validate_cli(&command).expect("command definition is valid");
}

#[test]
fn rejects_duplicate_root_and_inherited_global_options() {
    let root_collision = Command::new("example")
        .arg(Arg::new("first").long("format"))
        .arg(Arg::new("second").long("format"));
    let inherited_collision = Command::new("example")
        .arg(Arg::new("profile").long("profile").global(true))
        .subcommand(Command::new("inspect").arg(Arg::new("local").long("profile")));

    assert_eq!(
        validate_cli(&root_collision)
            .expect_err("duplicate root option is rejected")
            .to_string(),
        "invalid command-line definition at `example`: duplicate long option `format`"
    );
    assert_eq!(
        validate_cli(&inherited_collision)
            .expect_err("inherited global option is rejected")
            .to_string(),
        "invalid command-line definition at `example inspect`: duplicate long option `profile`"
    );
}

#[test]
fn rejects_colliding_subcommand_aliases() {
    let command = Command::new("example")
        .subcommand(Command::new("inspect").alias("show"))
        .subcommand(Command::new("show"));

    assert_eq!(
        validate_cli(&command)
            .expect_err("colliding subcommand alias is rejected")
            .to_string(),
        "invalid command-line definition at `example`: duplicate subcommand name or alias `show`"
    );
}

#[test]
fn rejects_hidden_argument_alias_collisions() {
    let command = Command::new("example")
        .arg(Arg::new("format").long("format").alias("output"))
        .arg(Arg::new("output").long("output"));

    assert_eq!(
        validate_cli(&command)
            .expect_err("hidden argument alias collision is rejected")
            .to_string(),
        "invalid command-line definition at `example`: duplicate long option `output`"
    );
}

#[test]
fn rejects_automatic_help_and_version_collisions() {
    let help_argument = Command::new("example").arg(Arg::new("custom-help").long("help"));
    let version_argument = Command::new("example")
        .version("1.0.0")
        .arg(Arg::new("custom-version").short('V'));
    let help_alias = Command::new("example").subcommand(Command::new("inspect").alias("help"));

    assert!(validate_cli(&help_argument).is_err());
    assert!(validate_cli(&version_argument).is_err());
    assert!(validate_cli(&help_alias).is_err());
}

#[test]
fn permits_version_option_without_command_version_metadata() {
    let command = Command::new("example")
        .subcommand(Command::new("inspect").arg(Arg::new("version").long("version").short('V')));

    validate_cli(&command).expect("nested command has no automatic version flag");
}

#[test]
fn rejects_group_and_argument_id_collisions() {
    let command = Command::new("example")
        .arg(Arg::new("mode").long("mode"))
        .group(ArgGroup::new("mode"));

    assert_eq!(
        validate_cli(&command)
            .expect_err("group and argument IDs share one namespace")
            .to_string(),
        "invalid command-line definition at `example`: duplicate argument or group id `mode`"
    );
}

#[test]
fn rejects_command_flag_collisions() {
    let argument_collision = Command::new("example")
        .arg(Arg::new("profile").short('p').global(true))
        .subcommand(Command::new("prepare").short_flag('p'));
    let sibling_collision = Command::new("example")
        .subcommand(Command::new("inspect").long_flag("run"))
        .subcommand(Command::new("serve").long_flag_alias("run"));

    assert!(validate_cli(&argument_collision).is_err());
    assert!(validate_cli(&sibling_collision).is_err());
}
