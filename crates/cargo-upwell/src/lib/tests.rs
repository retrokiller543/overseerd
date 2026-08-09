use upwell_test_utils::TempFixture;

use super::{CancellationToken, InvocationLock};

#[test]
fn invocation_lock_reports_uncontended_acquisition() {
    let fixture = TempFixture::new("cargo-upwell-invocation-lock-uncontended");
    let lock = InvocationLock::acquire(fixture.path(), &CancellationToken::default())
        .expect("uncontended invocation lock is acquired");

    assert!(!lock.was_contended());
}

#[test]
fn invocation_lock_reports_observed_contention() {
    let fixture = TempFixture::new("cargo-upwell-invocation-lock-contended");
    let first = InvocationLock::acquire(fixture.path(), &CancellationToken::default())
        .expect("first invocation lock is acquired");
    let mut first = Some(first);
    let second =
        InvocationLock::acquire_with_wait(fixture.path(), &CancellationToken::default(), || {
            drop(first.take())
        })
        .expect("waiting invocation lock is acquired");

    assert!(second.was_contended());
}
