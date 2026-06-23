use std::sync::Arc;

use tokio::sync::watch;

/// Coordinates graceful daemon shutdown across all tasks in the process.
pub struct ShutdownSignal {
    sender: Arc<watch::Sender<bool>>,
    receiver: watch::Receiver<bool>,
}

impl ShutdownSignal {
    pub fn new() -> Self {
        let (sender, receiver) = watch::channel(false);

        Self {
            sender: Arc::new(sender),
            receiver,
        }
    }

    /// Returns a handle that can trigger shutdown from any task.
    pub fn handle(&self) -> ShutdownHandle {
        ShutdownHandle {
            sender: self.sender.clone(),
        }
    }

    /// Waits until shutdown is triggered, either by a handle or by `Daemon::run`.
    pub async fn wait(&mut self) {
        let _ = self.receiver.wait_for(|v| *v).await;
    }

    /// Creates an independent listener on the same shutdown channel.
    ///
    /// Used to give background tasks (e.g. the watchdog loop) their own signal
    /// receiver that does not interfere with the primary receiver in `serve()`.
    pub fn subscribe(&self) -> ShutdownSignal {
        ShutdownSignal {
            sender: Arc::clone(&self.sender),
            receiver: self.sender.subscribe(),
        }
    }
}

impl Default for ShutdownSignal {
    fn default() -> Self {
        Self::new()
    }
}

/// A cloneable handle for triggering graceful shutdown from any task.
#[derive(Clone)]
pub struct ShutdownHandle {
    sender: Arc<watch::Sender<bool>>,
}

impl ShutdownHandle {
    pub fn shutdown(&self) {
        let _ = self.sender.send(true);
    }
}
