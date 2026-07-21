use std::sync::Arc;

use super::*;

#[test]
fn deferred_panics_before_scope_hydration() {
    let deferred = Deferred::<u8>::capture(ScopeResolverSlot::default(), None)
        .expect("deferred slot registers");

    assert!(deferred.try_get().is_none());
    assert!(std::panic::catch_unwind(|| deferred.get()).is_err());
}

#[tokio::test]
async fn unattached_scope_returns_typed_error_for_lazy() {
    let lazy = Lazy::<Arc<u8>>::capture(ScopeResolverSlot::default());

    assert!(matches!(
        lazy.get_or_create().await,
        Err(Error::ScopeUnavailable)
    ));
}
