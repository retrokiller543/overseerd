//! The `JobsPlugin`: registers the scheduler so a daemon runs its `#[job]`s.

use overseerd_app::{AppRegistry, Plugin};

use crate::scheduler::scheduler_descriptor;

/// The job-scheduler plugin.
///
/// A non-protocol [`Plugin`]: it serves no traffic, it registers the
/// [`JobScheduler`](crate::scheduler::JobScheduler) singleton whose `Startup` hook spawns a
/// loop per registered `#[job]`. Apply it alongside any protocol with
/// `AppBuilder::plugin(JobsPlugin::default())`.
///
/// Jobs are discovered at link time from the [`JOBS`](crate::descriptor::JOBS) slice the
/// `#[job]` macro appends to, so nothing needs to be listed here — registering the plugin is
/// enough for every `#[job]` in the binary to run.
#[derive(Default)]
pub struct JobsPlugin;

impl Plugin for JobsPlugin {
    fn register(&self, registry: &mut AppRegistry) {
        registry.components.push(scheduler_descriptor());
    }
}
