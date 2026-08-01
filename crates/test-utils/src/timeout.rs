use std::future::Future;
use std::time::Duration;

/// Default upper bound for asynchronous test operations.
pub const DEFAULT_TEST_TIMEOUT: Duration = Duration::from_secs(5);

/// Awaits a named test operation using the default timeout.
pub async fn deadline<F>(operation: &str, future: F) -> F::Output
where
    F: Future,
{
    deadline_with_timeout(operation, DEFAULT_TEST_TIMEOUT, future).await
}

/// Awaits a named test operation using an explicit timeout.
pub async fn deadline_with_timeout<F>(operation: &str, timeout: Duration, future: F) -> F::Output
where
    F: Future,
{
    let result = tokio::time::timeout(timeout, future).await;

    result.unwrap_or_else(|_| panic!("{operation} timed out after {timeout:?}"))
}
