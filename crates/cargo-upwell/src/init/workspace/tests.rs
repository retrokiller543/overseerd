use std::time::{Duration, SystemTime};

use upwell_test_utils::TempFixture;

use crate::init::InitError;

use super::{
    CANDIDATE_SUFFIX, COMPLETE_SUFFIX, DISPLACED_SUFFIX, INDETERMINATE_SUFFIX,
    MAX_COMPLETED_RECOVERIES, MAX_UNRESOLVED_RECOVERIES, PENDING_SUFFIX, RECOVERY_PREFIX,
    SNAPSHOT_SUFFIX, cleanup_recoveries, recovery_id, register_with,
};

#[test]
fn recovery_names_are_strictly_recognized() {
    let id = "0123456789abcdef0123456789abcdef-01234567-ab";

    assert_eq!(
        recovery_id(&format!(".cargo-upwell-workspace-recovery-{id}.snapshot")),
        Some(id)
    );
    assert_eq!(
        recovery_id(&format!(".cargo-upwell-workspace-recovery-{id}.unrelated")),
        None
    );
    assert_eq!(
        recovery_id(".cargo-upwell-workspace-recovery-dead.snapshot"),
        None
    );
}

#[test]
#[cfg(unix)]
fn cleanup_expires_completed_groups_and_caps_unresolved_groups() {
    let fixture = TempFixture::new("cargo-upwell-workspace-recovery-bounds");
    for index in 0..(MAX_UNRESOLVED_RECOVERIES + 3) {
        let id = format!("{index:032x}-00000001-00");

        for suffix in [SNAPSHOT_SUFFIX, PENDING_SUFFIX] {
            std::fs::write(
                fixture.child(format!("{RECOVERY_PREFIX}{id}{suffix}")),
                index.to_string(),
            )
            .expect("recognized recovery file exists");
        }
    }
    let unrelated = fixture.child(format!("{RECOVERY_PREFIX}dead.snapshot"));
    std::fs::write(&unrelated, "untouched").expect("unrelated file exists");
    std::fs::create_dir(fixture.child(format!(
        "{RECOVERY_PREFIX}00000000000000000000000000000000-00000001-00{CANDIDATE_SUFFIX}"
    )))
    .expect("recognized-looking directory exists");

    let error = cleanup_recoveries(fixture.path(), SystemTime::now(), MAX_UNRESOLVED_RECOVERIES)
        .expect_err("live recovery cap blocks another transaction");

    let recognized = std::fs::read_dir(fixture.path())
        .expect("fixture is readable")
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_file()))
        .filter(|entry| recovery_id(&entry.file_name().to_string_lossy()).is_some())
        .count();
    assert_eq!(recognized, (MAX_UNRESOLVED_RECOVERIES + 3) * 2);
    assert!(
        error
            .to_string()
            .contains("unresolved recovery transactions")
    );
    assert!(unrelated.exists());
}

#[test]
#[cfg(unix)]
fn cleanup_removes_expired_recognized_files_only() {
    let fixture = TempFixture::new("cargo-upwell-workspace-recovery-expiry");
    let expired = fixture.child(format!(
        "{RECOVERY_PREFIX}00000000000000000000000000000001-00000001-00{SNAPSHOT_SUFFIX}"
    ));
    std::fs::write(&expired, "snapshot").expect("snapshot exists");
    let future = SystemTime::now() + Duration::from_secs(25 * 60 * 60);

    cleanup_recoveries(fixture.path(), future, MAX_UNRESOLVED_RECOVERIES)
        .expect("cleanup succeeds");

    assert!(!expired.exists());
}

#[test]
#[cfg(unix)]
fn cleanup_bounds_fresh_completed_groups() {
    let fixture = TempFixture::new("cargo-upwell-workspace-completed-bounds");

    for sequence in 0..(MAX_COMPLETED_RECOVERIES + 3) {
        std::fs::write(
            recovery_path(&fixture, sequence, COMPLETE_SUFFIX),
            "complete",
        )
        .expect("completed state exists");
    }

    cleanup_recoveries(fixture.path(), SystemTime::now(), MAX_UNRESOLVED_RECOVERIES)
        .expect("completed cleanup succeeds");

    let remaining = std::fs::read_dir(fixture.path())
        .expect("fixture remains readable")
        .filter_map(Result::ok)
        .filter(|entry| recovery_id(&entry.file_name().to_string_lossy()).is_some())
        .count();
    assert_eq!(remaining, MAX_COMPLETED_RECOVERIES);
}

#[test]
#[cfg(unix)]
fn cleanup_keeps_unresolved_and_uses_newest_group_timestamp() {
    let fixture = TempFixture::new("cargo-upwell-workspace-recovery-state");
    let pending = recovery_path(&fixture, 1, PENDING_SUFFIX);
    let indeterminate = recovery_path(&fixture, 2, INDETERMINATE_SUFFIX);
    let old_snapshot = recovery_path(&fixture, 3, SNAPSHOT_SUFFIX);
    let fresh_complete = recovery_path(&fixture, 3, COMPLETE_SUFFIX);

    std::fs::write(&pending, "pending").expect("pending state exists");
    std::fs::write(&indeterminate, "indeterminate").expect("indeterminate state exists");
    std::fs::write(&old_snapshot, "snapshot").expect("snapshot exists");
    std::fs::write(&fresh_complete, "complete").expect("complete state exists");
    let future = SystemTime::now() + Duration::from_secs(25 * 60 * 60);

    let error = cleanup_recoveries(fixture.path(), future, 1)
        .expect_err("unresolved groups continue to count against the cap");

    assert!(
        error
            .to_string()
            .contains("unresolved recovery transactions")
    );
    assert!(pending.exists());
    assert!(indeterminate.exists());
    assert!(old_snapshot.exists());
    assert!(fresh_complete.exists());
}

#[test]
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn existing_member_is_a_zero_write_success() {
    let fixture = TempFixture::new("cargo-upwell-workspace-idempotent");
    let project = fixture.child("generated");
    let manifest = fixture.child("Cargo.toml");
    let source = "[workspace]\nmembers = [\"generated\"]\n";

    std::fs::create_dir_all(&project).expect("project directory exists");
    std::fs::write(&manifest, source).expect("workspace manifest exists");

    register_with(
        &project,
        || panic!("idempotent registration must not prepare publication"),
        || panic!("idempotent registration must not exchange"),
    )
    .expect("existing member is accepted");

    assert_eq!(
        std::fs::read_to_string(&manifest).expect("manifest remains readable"),
        source
    );
    assert!(
        !std::fs::read_dir(fixture.path())
            .expect("fixture directory is readable")
            .filter_map(Result::ok)
            .any(|entry| entry
                .file_name()
                .to_string_lossy()
                .starts_with(RECOVERY_PREFIX))
    );
}

#[test]
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn workspace_publication_is_explicitly_unsupported() {
    let fixture = TempFixture::new("cargo-upwell-workspace-unsupported");
    let project = fixture.child("generated");
    let manifest = fixture.child("Cargo.toml");

    std::fs::create_dir_all(&project).expect("project directory exists");
    std::fs::write(&manifest, "[workspace]\nmembers = []\n").expect("workspace manifest exists");

    let error = register_with(&project, || {}, || {}).expect_err("publication is unsupported");

    assert!(matches!(
        error,
        InitError::UnsupportedWorkspacePublication { .. }
    ));
}

#[test]
#[cfg(unix)]
fn concurrent_replacement_after_validation_is_retained() {
    let fixture = TempFixture::new("cargo-upwell-workspace-post-validation-race");
    let project = fixture.child("generated");
    let manifest = fixture.child("Cargo.toml");
    let replacement = fixture.child("Cargo.toml.editor");
    let original = "[workspace]\nmembers = []\n";
    let concurrent = "[workspace]\nmembers = []\n\n[workspace.metadata.editor]\nvalue = true\n";

    std::fs::create_dir_all(&project).expect("project directory exists");
    std::fs::write(&manifest, original).expect("workspace manifest exists");
    std::fs::write(&replacement, concurrent).expect("editor replacement exists");

    let error = register_with(
        &project,
        || {},
        || std::fs::rename(&replacement, &manifest).expect("editor atomically replaces manifest"),
    )
    .expect_err("stale publication is retained for recovery");

    assert!(
        matches!(&error, InitError::WorkspacePublicationIndeterminate { .. }),
        "unexpected publication error: {error:?}"
    );
    assert_eq!(
        std::fs::read_to_string(&replacement)
            .expect_err("replacement path was exchanged")
            .kind(),
        std::io::ErrorKind::NotFound
    );
    assert!(
        std::fs::read_to_string(&manifest)
            .expect("published manifest remains readable")
            .contains("generated")
    );
    assert!(
        std::fs::read_dir(fixture.path())
            .expect("fixture remains readable")
            .filter_map(Result::ok)
            .any(|entry| entry
                .file_name()
                .to_string_lossy()
                .ends_with(DISPLACED_SUFFIX)
                && std::fs::read_to_string(entry.path())
                    .is_ok_and(|contents| contents == concurrent))
    );
}

#[test]
#[cfg(unix)]
fn substituted_candidate_is_removed_from_the_live_manifest() {
    let fixture = TempFixture::new("cargo-upwell-workspace-candidate-substitution");
    let project = fixture.child("generated");
    let manifest = fixture.child("Cargo.toml");
    let substitute = fixture.child("attacker.toml");
    let original = "[workspace]\nmembers = []\n";
    let attacker = "[workspace]\nmembers = [\"attacker\"]\n";

    std::fs::create_dir_all(&project).expect("project directory exists");
    std::fs::write(&manifest, original).expect("workspace manifest exists");
    std::fs::write(&substitute, attacker).expect("substitute exists");

    let error = register_with(
        &project,
        || {},
        || {
            let candidate = std::fs::read_dir(fixture.path())
                .expect("fixture remains readable")
                .filter_map(Result::ok)
                .find(|entry| {
                    entry
                        .file_name()
                        .to_string_lossy()
                        .ends_with(DISPLACED_SUFFIX)
                })
                .expect("candidate path exists")
                .path();

            std::fs::rename(&substitute, candidate).expect("candidate path is substituted");
        },
    )
    .expect_err("candidate substitution makes publication indeterminate");

    assert!(matches!(
        &error,
        InitError::WorkspacePublicationIndeterminate { .. }
    ));
    assert_eq!(
        std::fs::read_to_string(&manifest).expect("original manifest is restored"),
        original
    );
    assert!(
        std::fs::read_dir(fixture.path())
            .expect("fixture remains readable")
            .filter_map(Result::ok)
            .any(|entry| entry
                .file_name()
                .to_string_lossy()
                .ends_with(DISPLACED_SUFFIX)
                && std::fs::read_to_string(entry.path())
                    .is_ok_and(|contents| contents == attacker))
    );
}

fn recovery_path(fixture: &TempFixture, sequence: usize, suffix: &str) -> std::path::PathBuf {
    fixture.child(format!(
        "{RECOVERY_PREFIX}{sequence:032x}-00000001-00{suffix}"
    ))
}
