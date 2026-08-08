use upwell_test_utils::TempFixture;

use super::commit;
use crate::init::InitError;

#[test]
fn concurrent_destination_is_never_replaced() {
    let fixture = TempFixture::new("cargo-upwell-publish-race");
    let staging = tempfile::tempdir_in(fixture.path()).expect("staging directory is created");
    let destination = fixture.child("project");

    std::fs::write(staging.path().join("generated.txt"), "generated")
        .expect("staged file is written");
    std::fs::create_dir(&destination).expect("another actor reserves the destination");
    std::fs::write(destination.join("owner.txt"), "existing")
        .expect("other actor's file is written");

    let error = commit(staging, &destination).expect_err("publication does not replace");

    assert!(matches!(error, InitError::DestinationExists(path) if path == destination));
    assert_eq!(
        std::fs::read_to_string(destination.join("owner.txt"))
            .expect("other actor's file remains readable"),
        "existing"
    );
    assert!(!destination.join("generated.txt").exists());
}
