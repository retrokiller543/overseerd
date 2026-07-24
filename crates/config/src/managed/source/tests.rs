use std::fs;

use super::{ConfigManager, Toml};
use crate::managed::ConfigError;

fn temp_config_dir(test: &str) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!("overseerd-config-{test}-{}", std::process::id()));

    let _ = fs::remove_dir_all(&path);
    fs::create_dir_all(&path).expect("create temporary config directory");

    path
}

#[test]
fn exact_file_profiles_override_in_order_and_retain_sources() {
    let directory = temp_config_dir("exact-profile-order");
    let base = directory.join("custom.toml");
    let first = directory.join("custom-first.toml");
    let second = directory.join("custom-second.toml");

    fs::write(&base, "[app]\nvalue = 1\nbase = true\n").expect("write base config");
    fs::write(&first, "[app]\nvalue = 2\nfirst = true\n").expect("write first profile");
    fs::write(&second, "[app]\nvalue = 3\n").expect("write second profile");

    let manager = ConfigManager::<Toml>::load_file(
        &base,
        &[
            String::from("first"),
            String::from("missing"),
            String::from("second"),
        ],
    )
    .expect("load exact config and profiles");

    assert_eq!(manager.get::<i64>("app.value").expect("read value"), 3);
    assert!(manager.get::<bool>("app.base").expect("read base value"));
    assert!(
        manager
            .get::<bool>("app.first")
            .expect("read profile value")
    );
    assert_eq!(manager.sources(), [base, first, second]);

    fs::remove_dir_all(directory).expect("remove temporary config directory");
}

#[test]
fn exact_file_requires_a_supported_extension() {
    let directory = temp_config_dir("unsupported-extension");
    let path = directory.join("application.json");

    fs::write(&path, "{}").expect("write unsupported config");

    let error = match ConfigManager::<Toml>::load_file(&path, &[]) {
        Ok(_) => panic!("unsupported config extension was accepted"),
        Err(error) => error,
    };

    assert!(
        matches!(error, ConfigError::UnsupportedFormat { path: error_path } if error_path == path)
    );

    fs::remove_dir_all(directory).expect("remove temporary config directory");
}

#[test]
fn explicit_directory_profiles_ignore_environment_resolution() {
    let directory = temp_config_dir("explicit-profile-order");

    fs::write(directory.join("application.toml"), "value = 1\n").expect("write base config");
    fs::write(directory.join("application-cli.toml"), "value = 2\n").expect("write CLI profile");

    let manager = ConfigManager::<Toml>::load_in_explicit(&directory, &[String::from("cli")])
        .expect("load explicit profiles");

    assert_eq!(manager.get::<i64>("value").expect("read value"), 2);
    assert_eq!(manager.sources().len(), 2);

    fs::remove_dir_all(directory).expect("remove temporary config directory");
}
