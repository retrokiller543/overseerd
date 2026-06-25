//! End-to-end tests for the `#[config]` macro's field defaults and enum support,
//! exercised through `ConfigManager::get_config` with the directory namespace wired in.
//! These cover the full path: macro-emitted `defaults()` -> merge -> templated
//! resolution -> typed value.

use std::path::PathBuf;

use overseerd::config::Toml;
use overseerd::{ConfigManager, DirectoriesManager, config};
use serde::Deserialize;

/// Resolves directory placeholders against a fixed root, so `${@runtime}` becomes
/// `<root>/runtime` deterministically.
fn manager(text: &str) -> ConfigManager {
    let dirs = DirectoriesManager::from_path(PathBuf::from("/base"));

    ConfigManager::<Toml>::from_str(text)
        .expect("parse config")
        .with_directories(&dirs)
        .into_dynamic()
}

#[config]
#[derive(Debug, Deserialize)]
struct ServerCfg {
    port: u16,

    #[default = "localhost"]
    host: String,

    #[default = "${@runtime}/srv.sock"]
    socket: PathBuf,
}

#[test]
fn struct_defaults_fill_missing_fields_and_resolve_namespace() {
    // Only `port` is in the file; `host` and the directory-templated `socket` fall back
    // to their `#[default]`s.
    let config = manager("[server]\nport = 8080\n");

    let server: ServerCfg = config.get_config::<ServerCfg>("server").unwrap();

    assert_eq!(server.port, 8080);
    assert_eq!(server.host, "localhost");
    assert_eq!(server.socket, PathBuf::from("/base/runtime/srv.sock"));
}

#[test]
fn file_value_overrides_a_field_default() {
    let config = manager("[server]\nport = 80\nhost = \"db.internal\"\n");

    let server: ServerCfg = config.get_config::<ServerCfg>("server").unwrap();

    assert_eq!(server.host, "db.internal");
}

#[config]
#[derive(Debug, Deserialize, PartialEq)]
enum Storage {
    Memory,
    Disk {
        #[default = "${@data}/blobs"]
        path: PathBuf,
    },
}

#[test]
fn enum_variant_default_applies_to_selected_variant() {
    // `Disk` is selected with its `path` omitted, so the variant default fills it.
    let config = manager("[storage]\nDisk = {}\n");

    let storage: Storage = config.get_config::<Storage>("storage").unwrap();

    assert_eq!(
        storage,
        Storage::Disk {
            path: PathBuf::from("/base/data/blobs"),
        }
    );
}

#[test]
fn enum_unit_variant_round_trips() {
    let config = manager("storage = \"Memory\"\n");

    let storage: Storage = config.get_config::<Storage>("storage").unwrap();

    assert_eq!(storage, Storage::Memory);
}
