use std::path::PathBuf;

use super::{BinaryCandidate, PackageCandidate, SelectedTarget, SelectionError, WorkspaceCatalog};

#[test]
fn selects_the_only_workspace_package_and_binary() {
    let catalog = catalog(vec![package(
        "server",
        false,
        vec![binary("server", &[], &[])],
    )]);

    let selected = catalog.select(None, None).expect("target selects");

    assert_eq!(selected, selected_target("server", "server"));
}

#[test]
fn workspace_default_member_wins_over_other_packages() {
    let catalog = catalog(vec![
        package("worker", false, vec![binary("worker", &[], &[])]),
        package("server", true, vec![binary("server", &[], &[])]),
    ]);

    let selected = catalog.select(None, None).expect("default member selects");

    assert_eq!(selected.package_name, "server");
}

#[test]
fn ambiguous_packages_are_sorted_and_machine_inspectable() {
    let catalog = catalog(vec![
        package("worker", false, vec![binary("worker", &[], &[])]),
        package("api", false, vec![binary("api", &[], &[])]),
    ]);

    let error = catalog
        .select(None, None)
        .expect_err("selection is ambiguous");
    let SelectionError::AmbiguousPackage { candidates } = error else {
        panic!("unexpected selection error");
    };

    assert_eq!(
        candidates
            .iter()
            .map(|package| package.name.as_str())
            .collect::<Vec<_>>(),
        ["api", "worker"]
    );
}

#[test]
fn package_default_run_wins_over_other_binaries() {
    let mut package = package(
        "server",
        false,
        vec![binary("admin", &[], &[]), binary("server", &[], &[])],
    );

    package.default_run = Some(String::from("server"));

    let selected = catalog(vec![package])
        .select(None, None)
        .expect("default run selects");

    assert_eq!(selected.binary_name, "server");
}

#[test]
fn required_feature_failure_exposes_exact_missing_features() {
    let catalog = catalog(vec![package(
        "server",
        false,
        vec![binary("server", &["cli", "tooling"], &["tooling"])],
    )]);

    let error = catalog
        .select(None, Some("server"))
        .expect_err("disabled binary rejects");

    assert_eq!(
        error,
        SelectionError::RequiredFeatures {
            package: String::from("server"),
            binary: String::from("server"),
            missing_features: vec![String::from("tooling")],
        }
    );
}

#[test]
fn sole_eligible_binary_wins_over_disabled_targets() {
    let catalog = catalog(vec![package(
        "server",
        false,
        vec![
            binary("server", &[], &[]),
            binary("tooling-only", &["tooling"], &["tooling"]),
        ],
    )]);

    let selected = catalog.select(None, None).expect("eligible target selects");

    assert_eq!(selected.binary_name, "server");
}

fn catalog(mut packages: Vec<PackageCandidate>) -> WorkspaceCatalog {
    packages.sort_by(|left, right| left.name.cmp(&right.name));

    WorkspaceCatalog {
        workspace_root: PathBuf::from("/workspace"),
        target_directory: PathBuf::from("/workspace/target"),
        packages,
    }
}

fn package(
    name: &str,
    default_member: bool,
    mut binaries: Vec<BinaryCandidate>,
) -> PackageCandidate {
    binaries.sort();

    PackageCandidate {
        id: format!("path+file:///workspace/{name}#0.1.0"),
        name: name.to_string(),
        version: String::from("0.1.0"),
        manifest_path: PathBuf::from(format!("/workspace/{name}/Cargo.toml")),
        binaries,
        default_run: None,
        default_member,
    }
}

fn binary(name: &str, required: &[&str], missing: &[&str]) -> BinaryCandidate {
    BinaryCandidate {
        name: name.to_string(),
        required_features: required.iter().map(ToString::to_string).collect(),
        missing_features: missing.iter().map(ToString::to_string).collect(),
    }
}

fn selected_target(package: &str, binary: &str) -> SelectedTarget {
    SelectedTarget {
        package_id: format!("path+file:///workspace/{package}#0.1.0"),
        package_name: package.to_string(),
        package_version: String::from("0.1.0"),
        manifest_path: PathBuf::from(format!("/workspace/{package}/Cargo.toml")),
        binary_name: binary.to_string(),
        required_features: Vec::new(),
    }
}
