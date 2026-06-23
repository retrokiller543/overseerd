use std::{sync::Arc, time::Duration};

use tokio::{task::JoinHandle, time::sleep};
use tracing::{debug, warn};

use crate::health::{HealthCheck, HealthStatus};

/// Which service manager, if any, is managing this process.
///
/// Detected from environment variables at startup — no configuration required.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Detected {
    /// Running under systemd (Linux). `NOTIFY_SOCKET` is set.
    Systemd,
    /// Running under launchd (macOS). `LAUNCH_DAEMON_SOCKET_NAME` or
    /// `LAUNCH_AGENT_SOCKET_NAME` is set.
    Launchd,
    /// Running directly (terminal, development, or unsupported platform).
    Direct,
}

/// Detects which service manager, if any, is managing this process.
pub fn detect() -> Detected {
    if std::env::var_os("NOTIFY_SOCKET").is_some() {
        return Detected::Systemd;
    }

    if std::env::var_os("LAUNCH_DAEMON_SOCKET_NAME").is_some()
        || std::env::var_os("LAUNCH_AGENT_SOCKET_NAME").is_some()
    {
        return Detected::Launchd;
    }

    Detected::Direct
}

/// Notifies the service manager that the daemon is ready to accept connections.
///
/// Sends `READY=1` on systemd; no-op on other platforms or when not managed.
pub fn notify_ready() {
    #[cfg(target_os = "linux")]
    {
        use sd_notify::{NotifyState, notify};
        if let Err(e) = notify(false, &[NotifyState::Ready]) {
            warn!(error = %e, "failed to send READY notification to systemd");
        } else {
            debug!("sent READY=1 to systemd");
        }
    }
}

/// Notifies the service manager that the daemon is beginning a clean shutdown.
///
/// Sends `STOPPING=1` on systemd; no-op on other platforms or when not managed.
pub fn notify_stopping() {
    #[cfg(target_os = "linux")]
    {
        use sd_notify::{NotifyState, notify};
        if let Err(e) = notify(false, &[NotifyState::Stopping]) {
            warn!(error = %e, "failed to send STOPPING notification to systemd");
        } else {
            debug!("sent STOPPING=1 to systemd");
        }
    }
}

/// Returns the watchdog interval requested by the service manager (`WATCHDOG_USEC / 2`),
/// so pings are sent at twice the required frequency.
///
/// Returns `None` when no watchdog is configured or on non-Linux platforms.
pub fn watchdog_interval() -> Option<Duration> {
    #[cfg(target_os = "linux")]
    {
        let mut usec: u64 = 0;
        if sd_notify::watchdog_enabled(false, &mut usec) && usec > 0 {
            return Some(Duration::from_micros(usec) / 2);
        }
        return None;
    }
    #[cfg(not(target_os = "linux"))]
    None
}

/// Sends a watchdog ping to systemd, with an optional STATUS message.
///
/// No-op when `NOTIFY_SOCKET` is not set or on non-Linux platforms.
fn ping_watchdog(status: Option<&str>) {
    #[cfg(target_os = "linux")]
    {
        use sd_notify::{NotifyState, notify};
        let mut states = vec![NotifyState::Watchdog];
        if let Some(s) = status {
            states.push(NotifyState::Status(s));
        }
        if let Err(e) = notify(false, &states) {
            warn!(error = %e, "failed to send watchdog ping to systemd");
        }
    }
    // Suppress unused variable warning on non-Linux
    #[cfg(not(target_os = "linux"))]
    let _ = status;
}

/// Spawns the watchdog task if the service manager requests one.
///
/// Returns `None` if no watchdog interval is configured. The returned handle
/// is aborted when dropped; pass the daemon's shutdown signal to let the task
/// exit cleanly before the handle is dropped.
pub fn spawn_watchdog(
    health_checks: Vec<Arc<dyn HealthCheck>>,
    shutdown: crate::lifecycle::ShutdownSignal,
) -> Option<JoinHandle<()>> {
    let interval = watchdog_interval()?;

    debug!(interval_ms = interval.as_millis(), "starting watchdog task");

    Some(tokio::spawn(watchdog_loop(health_checks, interval, shutdown)))
}

async fn watchdog_loop(
    checks: Vec<Arc<dyn HealthCheck>>,
    interval: Duration,
    mut shutdown: crate::lifecycle::ShutdownSignal,
) {
    loop {
        tokio::select! {
            _ = sleep(interval) => {
                let statuses = poll_checks(&checks).await;
                let has_unhealthy = statuses.iter().any(|(_, s)| s.is_unhealthy());

                if has_unhealthy {
                    debug!("health check failed — suppressing watchdog ping");
                    continue;
                }

                let degraded: Vec<&str> = statuses
                    .iter()
                    .filter(|(_, s)| matches!(s, HealthStatus::Degraded))
                    .map(|(name, _)| *name)
                    .collect();

                if degraded.is_empty() {
                    ping_watchdog(None);
                } else {
                    let msg = format!("degraded: {}", degraded.join(", "));
                    ping_watchdog(Some(&msg));
                }
            }

            _ = shutdown.wait() => {
                debug!("watchdog task shutting down");
                break;
            }
        }
    }
}

async fn poll_checks(checks: &[Arc<dyn HealthCheck>]) -> Vec<(&str, HealthStatus)> {
    let mut results = Vec::with_capacity(checks.len());
    for check in checks {
        let status = check.check().await;
        results.push((check.name(), status));
    }
    results
}

/// Resolves into `()` on Unix SIGTERM, or never on other platforms.
///
/// Used in `tokio::select!` to handle SIGTERM alongside ctrl-c without
/// `#[cfg]` attrs inside the select block.
pub async fn sigterm() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        match signal(SignalKind::terminate()) {
            Ok(mut s) => {
                let _ = s.recv().await;
            }
            Err(e) => {
                warn!(error = %e, "failed to install SIGTERM handler; only ctrl-c will trigger shutdown");
                std::future::pending::<()>().await
            }
        }
    }
    #[cfg(not(unix))]
    std::future::pending::<()>().await
}
