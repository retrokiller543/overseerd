use std::panic;

use upwell_test_utils::TempFixture;

#[test]
fn writes_are_scoped_to_the_fixture() {
    let fixture = TempFixture::new("upwell-temp-fixture-");
    let path = fixture.write("nested/value.txt", b"value");

    assert_eq!(path, fixture.child("nested/value.txt"));
    assert_eq!(
        std::fs::read(path).expect("fixture file is readable"),
        b"value"
    );
}

#[test]
fn child_rejects_paths_that_escape_the_fixture() {
    let fixture = TempFixture::new("upwell-temp-fixture-");

    assert!(panic::catch_unwind(|| fixture.child("../outside")).is_err());
    assert!(panic::catch_unwind(|| fixture.child(std::env::temp_dir())).is_err());
}
