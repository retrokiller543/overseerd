use std::future::Future;
use std::time::Duration;

#[cfg(feature = "app")]
use std::net::{Ipv4Addr, SocketAddr};
#[cfg(feature = "app")]
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

use crate::{DEFAULT_TEST_TIMEOUT, deadline_with_timeout};

/// Owns a Tokio task and aborts it if normal checked joining does not complete.
pub struct AbortOnDropTask<T> {
    name: &'static str,
    task: Option<JoinHandle<T>>,
}

impl<T: Send + 'static> AbortOnDropTask<T> {
    /// Spawns and owns a named Tokio task.
    pub fn spawn(name: &'static str, future: impl Future<Output = T> + Send + 'static) -> Self {
        Self {
            name,
            task: Some(tokio::spawn(future)),
        }
    }

    /// Returns whether the owned task has completed.
    pub fn is_finished(&self) -> bool {
        self.task.as_ref().is_none_or(JoinHandle::is_finished)
    }

    /// Joins the task using the default timeout.
    pub async fn join(&mut self) -> T {
        self.join_with_timeout(DEFAULT_TEST_TIMEOUT).await
    }

    /// Joins the task using an explicit timeout.
    pub async fn join_with_timeout(&mut self, timeout: Duration) -> T {
        let task = self.task.as_mut().expect("test task is present");
        let result = deadline_with_timeout(self.name, timeout, task).await;

        self.task = None;

        result.unwrap_or_else(|error| panic!("{} task failed: {error}", self.name))
    }
}

impl<T> Drop for AbortOnDropTask<T> {
    fn drop(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

/// Owns an ephemeral loopback listener's address and serving task.
#[cfg(feature = "app")]
pub struct LoopbackTask<T> {
    address: SocketAddr,
    task: AbortOnDropTask<T>,
}

#[cfg(feature = "app")]
impl<T: Send + 'static> LoopbackTask<T> {
    /// Binds an ephemeral loopback listener and starts serving it.
    pub async fn spawn<F, Fut>(name: &'static str, serve: F) -> Self
    where
        F: FnOnce(TcpListener) -> Fut,
        Fut: Future<Output = T> + Send + 'static,
    {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap_or_else(|error| panic!("bind {name} listener: {error}"));
        let address = listener
            .local_addr()
            .unwrap_or_else(|error| panic!("read {name} listener address: {error}"));
        let task = AbortOnDropTask::spawn(name, serve(listener));

        Self { address, task }
    }

    /// Returns the bound loopback address.
    pub fn address(&self) -> SocketAddr {
        self.address
    }

    /// Joins the serving task using the default timeout.
    pub async fn join(&mut self) -> T {
        self.task.join().await
    }
}
