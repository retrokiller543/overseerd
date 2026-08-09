use std::time::{Duration, SystemTime};

use upwell_test_utils::TempFixture;

use super::{
    CANDIDATE_SUFFIX, COMPLETE_SUFFIX, DISPLACED_SUFFIX, INDETERMINATE_SUFFIX, MAX_RECOVERIES,
    PENDING_SUFFIX, RECOVERY_PREFIX, SNAPSHOT_SUFFIX, cleanup_recoveries, recovery_id,
    register_with,
};

#[test]
fn recovery_names_are_strictly_recognized() {
    assert_eq!(
        recovery_id(".cargo-upwell-workspace-recovery-0123-ab.snapshot"),
        Some("0123-ab")
    );
    assert_eq!(
        recovery_id(".cargo-upwell-workspace-recovery-0123-ab.unrelated"),
        None
    );
    assert_eq!(
        recovery_id(".cargo-upwell-workspace-recovery-../x.snapshot"),
        None
    );
}

#[test]
fn cleanup_expires_pairs_and_enforces_hard_cap() {
    let fixture = TempFixture::new("cargo-upwell-workspace-recovery-bounds");
    for index in 0..(MAX_RECOVERIES + 3) {
        for suffix in [SNAPSHOT_SUFFIX, DISPLACED_SUFFIX] {
            std::fs::write(
                fixture.child(format!("{RECOVERY_PREFIX}{index:02x}{suffix}")),
                index.to_string(),
            )
            .expect("recognized recovery file exists");
        }
    }
    let unrelated = fixture.child(format!("{RECOVERY_PREFIX}00.unrelated"));
    std::fs::write(&unrelated, "untouched").expect("unrelated file exists");
    std::fs::create_dir(fixture.child(format!("{RECOVERY_PREFIX}00{CANDIDATE_SUFFIX}")))
        .expect("recognized-looking directory exists");

    let error = cleanup_recoveries(fixture.path(), SystemTime::now(), MAX_RECOVERIES)
        .expect_err("live recovery cap blocks another transaction");

    let recognized = std::fs::read_dir(fixture.path())
        .expect("fixture is readable")
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_file()))
        .filter(|entry| recovery_id(&entry.file_name().to_string_lossy()).is_some())
        .count();
    assert_eq!(recognized, (MAX_RECOVERIES + 3) * 2);
    assert!(error.to_string().contains("live recovery transactions"));
    assert!(unrelated.exists());
}

#[test]
fn cleanup_removes_expired_recognized_files_only() {
    let fixture = TempFixture::new("cargo-upwell-workspace-recovery-expiry");
    let expired = fixture.child(format!("{RECOVERY_PREFIX}01{SNAPSHOT_SUFFIX}"));
    std::fs::write(&expired, "snapshot").expect("snapshot exists");
    let future = SystemTime::now() + Duration::from_secs(25 * 60 * 60);

    cleanup_recoveries(fixture.path(), future, MAX_RECOVERIES).expect("cleanup succeeds");

    assert!(!expired.exists());
}

#[test]
fn cleanup_keeps_unresolved_and_uses_newest_group_timestamp() {
    let fixture = TempFixture::new("cargo-upwell-workspace-recovery-state");
    let pending = fixture.child(format!("{RECOVERY_PREFIX}01{PENDING_SUFFIX}"));
    let indeterminate = fixture.child(format!("{RECOVERY_PREFIX}02{INDETERMINATE_SUFFIX}"));
    let old_snapshot = fixture.child(format!("{RECOVERY_PREFIX}03{SNAPSHOT_SUFFIX}"));
    let fresh_complete = fixture.child(format!("{RECOVERY_PREFIX}03{COMPLETE_SUFFIX}"));

    std::fs::write(&pending, "pending").expect("pending state exists");
    std::fs::write(&indeterminate, "indeterminate").expect("indeterminate state exists");
    std::fs::write(&old_snapshot, "snapshot").expect("snapshot exists");
    std::fs::write(&fresh_complete, "complete").expect("complete state exists");
    let future = SystemTime::now() + Duration::from_secs(25 * 60 * 60);

    let error = cleanup_recoveries(fixture.path(), future, 1)
        .expect_err("unresolved groups continue to count against the cap");

    assert!(error.to_string().contains("live recovery transactions"));
    assert!(pending.exists());
    assert!(indeterminate.exists());
    assert!(old_snapshot.exists());
    assert!(fresh_complete.exists());
}

#[test]
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
