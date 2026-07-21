use std::sync::Arc;

use super::*;

#[tokio::test]
async fn unattached_scope_returns_typed_error() {
    let lazy = Lazy::<Arc<u8>>::capture(ScopeResolverSlot::default());
    let deferred = Deferred::<u8>::capture(ScopeResolverSlot::default(), None);

    assert!(matches!(
        lazy.get_or_create().await,
        Err(Error::ScopeUnavailable)
    ));
    assert!(matches!(
        deferred.get_or_resolve().await,
        Err(Error::ScopeUnavailable)
    ));
}
