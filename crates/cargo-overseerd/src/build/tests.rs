use std::process::Command;

use super::configure_cargo_output;

#[test]
fn interactive_cargo_output_forces_native_progress() {
    let mut command = Command::new("cargo");

    configure_cargo_output(&mut command);

    let arguments = command
        .get_args()
        .map(|argument| argument.to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    let environment = command
        .get_envs()
        .filter_map(|(name, value)| {
            value.map(|value| {
                (
                    name.to_string_lossy().into_owned(),
                    value.to_string_lossy().into_owned(),
                )
            })
        })
        .collect::<std::collections::BTreeMap<_, _>>();

    assert!(
        arguments
            .iter()
            .any(|argument| argument.starts_with("--color="))
    );
    assert_eq!(environment["CARGO_TERM_PROGRESS_WHEN"], "always");
    assert!(
        environment["CARGO_TERM_PROGRESS_WIDTH"]
            .parse::<u16>()
            .is_ok_and(|width| width > 0)
    );
}
