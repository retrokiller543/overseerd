//! Jobs-aware tracing installation.
//!
//! [`init_tracing`] is the drop-in replacement for [`overseerd_app::builtins::init_tracing`]
//! when the `jobs` scheduler is in use: it installs the framework subscriber and, when capture
//! is enabled, layers per-run [`JobLogLayer`] capture onto it, returning the [`JobLogSink`] to
//! hand to [`JobScheduler::set_log_sink`](crate::JobScheduler::set_log_sink). Because layers
//! must be composed before the subscriber is installed, this is the single call that wires both
//! at once — driven entirely by the [`LoggingConfig`] and [`JobLogConfig`].

use std::sync::Arc;

use overseerd_app::builtins::{
    BoxedLayer, InitTracingError, LoggingConfig, init_tracing_with_layers,
};

use crate::log::{InMemoryJobLogStore, JobLogConfig, JobLogLayer, JobLogSink, NoopJobLogStore};

/// Installs the framework tracing subscriber, adding per-run job log capture when
/// [`JobLogConfig::enabled`] is set.
///
/// Returns the [`JobLogSink`] backing capture — a bounded [`InMemoryJobLogStore`] when enabled,
/// or a [`NoopJobLogStore`] otherwise — so the caller can pass it to
/// [`JobScheduler::set_log_sink`](crate::JobScheduler::set_log_sink). Both the fmt output and
/// the capture bounds/level come from configuration, so a daemon toggles capture without code
/// changes.
///
/// ```ignore
/// let sink = overseerd::jobs::init_tracing(&logging, JobLogConfig::default())?;
/// // … after building the app …
/// scheduler.set_log_sink(sink);
/// ```
pub fn init_tracing(
    config: &LoggingConfig,
    job_log: JobLogConfig,
) -> Result<Arc<dyn JobLogSink>, InitTracingError> {
    if !job_log.enabled {
        init_tracing_with_layers(config, Vec::new())?;

        return Ok(Arc::new(NoopJobLogStore));
    }

    let capture_level = job_log.capture_level;
    let store = Arc::new(InMemoryJobLogStore::new(job_log));
    let sink: Arc<dyn JobLogSink> = store;

    let layer: BoxedLayer = Box::new(JobLogLayer::new(Arc::clone(&sink), capture_level));

    init_tracing_with_layers(config, vec![layer])?;

    Ok(sink)
}
