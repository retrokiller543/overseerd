use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

use upwell_test_utils::TempFixture;

use super::{
    Candidate, CandidateKind, MAX_CANDIDATES, SNAPSHOT_SCHEMA, Snapshot, read_candidates_from,
    safe_text, workspace_key,
};

#[test]
fn candidate_text_rejects_control_characters_and_oversized_values() {
    assert!(safe_text("plugin:worker", 64));
    assert!(!safe_text("", 64));
    assert!(!safe_text("line\nbreak", 64));
    assert!(!safe_text(&"x".repeat(65), 64));
}

#[test]
fn workspace_cache_keys_are_stable_and_path_sensitive() {
    let first = workspace_key(std::path::Path::new("/workspace/first"));
    let same = workspace_key(std::path::Path::new("/workspace/first"));
    let second = workspace_key(std::path::Path::new("/workspace/second"));

    assert_eq!(first, same);
    assert_ne!(first, second);
    assert_eq!(first.len(), 16);
}

#[test]
fn candidate_kind_keys_are_stable() {
    assert_eq!(CandidateKind::Package.key(), "package");
    assert_eq!(CandidateKind::Resource.key(), "resource");
    assert_eq!(CandidateKind::Facet.key(), "facet");
}

#[test]
fn nested_member_reads_the_outer_workspace_snapshot() {
    let fixture = TempFixture::new("cargo-upwell-completion-nested-workspace");
    let member = fixture.child("crates/member");
    let cache = fixture.child("cache");

    fixture.write("Cargo.toml", "[workspace]\nmembers = [\"crates/member\"]\n");
    std::fs::create_dir_all(&member).expect("member directory exists");
    std::fs::write(
        member.join("Cargo.toml"),
        "[package]\nname = \"member\"\nversion = \"0.1.0\"\n",
    )
    .expect("member manifest is written");
    write_snapshot(&cache, fixture.path(), fixture.path(), "component:worker");

    assert_eq!(
        read_candidates_from(CandidateKind::Resource, &member, &cache).expect("snapshot reads"),
        [Candidate {
            value: String::from("component:worker"),
            help: None,
        }]
    );
}

#[test]
fn stale_nested_package_snapshot_does_not_shadow_outer_workspace() {
    let fixture = TempFixture::new("cargo-upwell-completion-stale-nested-cache");
    let member = fixture.child("crates/member");
    let cache = fixture.child("cache");

    fixture.write("Cargo.toml", "[workspace]\nmembers = [\"crates/member\"]\n");
    std::fs::create_dir_all(&member).expect("member directory exists");
    std::fs::write(
        member.join("Cargo.toml"),
        "[package]\nname = \"member\"\nversion = \"0.1.0\"\n",
    )
    .expect("member manifest is written");
    write_snapshot(&cache, &member, &member, "component:stale");
    write_snapshot(&cache, fixture.path(), fixture.path(), "component:active");

    assert_eq!(
        read_candidates_from(CandidateKind::Resource, &member, &cache)
            .expect("outer workspace snapshot reads"),
        [Candidate {
            value: String::from("component:active"),
            help: None,
        }]
    );
}

#[test]
fn excluded_nested_package_uses_its_own_snapshot() {
    let fixture = TempFixture::new("cargo-upwell-completion-excluded-package");
    let nested = fixture.child("tools/standalone");
    let cache = fixture.child("cache");

    fixture.write(
        "Cargo.toml",
        "[workspace]\nmembers = [\"crates/*\"]\nexclude = [\"tools/standalone\"]\n",
    );
    std::fs::create_dir_all(&nested).expect("nested package directory exists");
    std::fs::write(
        nested.join("Cargo.toml"),
        "[package]\nname = \"standalone\"\nversion = \"0.1.0\"\n",
    )
    .expect("nested package manifest is written");
    write_snapshot(&cache, fixture.path(), fixture.path(), "component:outer");
    write_snapshot(&cache, &nested, &nested, "component:standalone");

    assert_eq!(
        read_candidates_from(CandidateKind::Resource, &nested, &cache)
            .expect("standalone package snapshot reads"),
        [Candidate {
            value: String::from("component:standalone"),
            help: None,
        }]
    );
}

#[test]
fn nested_workspace_globs_select_and_exclude_packages() {
    let fixture = TempFixture::new("cargo-upwell-completion-workspace-globs");
    let member = fixture.child("crates/backend/apps/server");
    let excluded = fixture.child("crates/legacy/apps/server");
    let cache = fixture.child("cache");

    fixture.write(
        "Cargo.toml",
        "[workspace]\nmembers = [\"crates/*/apps/**\"]\nexclude = [\"crates/legacy/**\"]\n",
    );
    for package in [&member, &excluded] {
        std::fs::create_dir_all(package).expect("package directory exists");
        std::fs::write(
            package.join("Cargo.toml"),
            "[package]\nname = \"server\"\nversion = \"0.1.0\"\n",
        )
        .expect("package manifest is written");
    }
    write_snapshot(
        &cache,
        fixture.path(),
        fixture.path(),
        "component:workspace",
    );
    write_snapshot(&cache, &excluded, &excluded, "component:excluded");

    assert_eq!(
        read_candidates_from(CandidateKind::Resource, &member, &cache)
            .expect("nested member uses workspace snapshot"),
        [Candidate {
            value: String::from("component:workspace"),
            help: None,
        }]
    );
    assert_eq!(
        read_candidates_from(CandidateKind::Resource, &excluded, &cache)
            .expect("excluded package uses own snapshot"),
        [Candidate {
            value: String::from("component:excluded"),
            help: None,
        }]
    );
}

#[test]
fn mismatched_workspace_identity_is_ignored() {
    let fixture = TempFixture::new("cargo-upwell-completion-identity");
    let cache = fixture.child("cache");
    let other = fixture.child("other");

    fixture.write("Cargo.toml", "[workspace]\n");
    std::fs::create_dir_all(&other).expect("other workspace exists");
    write_snapshot(&cache, fixture.path(), &other, "component:worker");

    assert!(
        read_candidates_from(CandidateKind::Resource, fixture.path(), &cache)
            .expect("snapshot reads")
            .is_empty()
    );
}

#[test]
fn oversized_candidate_collection_is_ignored() {
    let fixture = TempFixture::new("cargo-upwell-completion-candidate-limit");
    let cache = fixture.child("cache");

    fixture.write("Cargo.toml", "[workspace]\n");
    let values = (0..=MAX_CANDIDATES)
        .map(|index| Candidate {
            value: format!("resource:{index}"),
            help: None,
        })
        .collect();
    write_snapshot_with_candidates(&cache, fixture.path(), fixture.path(), values);

    assert!(
        read_candidates_from(CandidateKind::Resource, fixture.path(), &cache)
            .expect("snapshot reads")
            .is_empty()
    );
}

#[test]
fn unsafe_deserialized_candidate_text_is_ignored() {
    let fixture = TempFixture::new("cargo-upwell-completion-candidate-text");
    let cache = fixture.child("cache");

    fixture.write("Cargo.toml", "[workspace]\n");
    write_snapshot_with_candidates(
        &cache,
        fixture.path(),
        fixture.path(),
        vec![Candidate {
            value: String::from("resource\nforged"),
            help: None,
        }],
    );

    assert!(
        read_candidates_from(CandidateKind::Resource, fixture.path(), &cache)
            .expect("snapshot reads")
            .is_empty()
    );
}

#[test]
fn far_future_snapshot_is_ignored() {
    let fixture = TempFixture::new("cargo-upwell-completion-future-snapshot");
    let cache = fixture.child("cache");

    fixture.write("Cargo.toml", "[workspace]\n");
    write_snapshot_with_time(
        &cache,
        fixture.path(),
        fixture.path(),
        vec![Candidate {
            value: String::from("component:worker"),
            help: None,
        }],
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time follows epoch")
            .as_secs()
            + 3600,
    );

    assert!(
        read_candidates_from(CandidateKind::Resource, fixture.path(), &cache)
            .expect("snapshot reads")
            .is_empty()
    );
}

#[cfg(unix)]
#[test]
fn symlinked_snapshot_is_ignored() {
    use std::os::unix::fs::symlink;

    let fixture = TempFixture::new("cargo-upwell-completion-symlink");
    let cache = fixture.child("cache");
    let directory = cache.join(workspace_key(fixture.path()));
    let target = fixture.child("target.json");

    fixture.write("Cargo.toml", "[workspace]\n");
    std::fs::create_dir_all(&directory).expect("cache directory exists");
    std::fs::write(&target, "{}").expect("symlink target exists");
    symlink(&target, directory.join("active.json")).expect("cache symlink exists");

    assert!(
        read_candidates_from(CandidateKind::Resource, fixture.path(), &cache)
            .expect("symlink is safely ignored")
            .is_empty()
    );
}

#[cfg(unix)]
#[test]
fn symlinked_workspace_cache_directory_is_ignored() {
    use std::os::unix::fs::symlink;

    let fixture = TempFixture::new("cargo-upwell-completion-directory-symlink");
    let cache = fixture.child("cache");
    let target = fixture.child("target");

    fixture.write("Cargo.toml", "[workspace]\n");
    std::fs::create_dir_all(&cache).expect("cache root exists");
    std::fs::create_dir_all(&target).expect("symlink target exists");
    std::fs::write(target.join("active.json"), "{}").expect("target snapshot exists");
    symlink(&target, cache.join(workspace_key(fixture.path())))
        .expect("workspace cache symlink exists");

    assert!(
        read_candidates_from(CandidateKind::Resource, fixture.path(), &cache)
            .expect_err("directory symlink is rejected")
            .to_string()
            .contains("symlink")
    );
}

fn write_snapshot(
    cache: &std::path::Path,
    key: &std::path::Path,
    identity: &std::path::Path,
    value: &str,
) {
    write_snapshot_with_candidates(
        cache,
        key,
        identity,
        vec![Candidate {
            value: value.to_owned(),
            help: None,
        }],
    );
}

fn write_snapshot_with_candidates(
    cache: &std::path::Path,
    key: &std::path::Path,
    identity: &std::path::Path,
    values: Vec<Candidate>,
) {
    write_snapshot_with_time(
        cache,
        key,
        identity,
        values,
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time follows epoch")
            .as_secs(),
    );
}

fn write_snapshot_with_time(
    cache: &std::path::Path,
    key: &std::path::Path,
    identity: &std::path::Path,
    values: Vec<Candidate>,
    generated_at: u64,
) {
    let directory = cache.join(workspace_key(key));
    let mut candidates = BTreeMap::new();

    candidates.insert(String::from("resource"), values);
    std::fs::create_dir_all(&directory).expect("cache directory exists");
    std::fs::write(
        directory.join("active.json"),
        serde_json::to_vec(&Snapshot {
            schema: SNAPSHOT_SCHEMA,
            cargo_upwell_version: env!("CARGO_PKG_VERSION").to_owned(),
            generated_at,
            workspace_root: identity
                .canonicalize()
                .unwrap_or_else(|_| identity.to_path_buf()),
            package_id: String::from("fixture 0.1.0"),
            binary: String::from("fixture"),
            no_default_features: false,
            all_features: false,
            features: Vec::new(),
            target: None,
            candidates,
        })
        .expect("snapshot serializes"),
    )
    .expect("snapshot is written");
}
