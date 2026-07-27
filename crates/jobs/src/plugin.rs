//! The `JobsPlugin`: registers the scheduler so a daemon runs its `#[job]`s.

use overseerd_app::{Plugin, PluginContributions, PluginId};

use crate::JobScheduler;

/// The job-scheduler plugin.
///
/// A non-protocol [`Plugin`]: it serves no traffic, it registers the
/// [`JobScheduler`] singleton whose `Startup` hook spawns a
/// loop per registered `#[job]`. Apply it alongside any protocol with
/// `AppBuilder::register_plugin::<JobsPlugin>()`.
///
/// Jobs are discovered at link time from the [`JOBS`](crate::descriptor::JOBS) slice the
/// `#[job]` macro appends to, so nothing needs to be listed here — registering the plugin is
/// enough for every `#[job]` in the binary to run.
#[derive(Default)]
pub struct JobsPlugin;

impl Plugin for JobsPlugin {
    const ID: PluginId = overseerd_app::namespaced_id!(PluginId, "jobs/scheduler");

    fn contribute(self, contributions: &mut PluginContributions) {
        overseerd_app::contribute! {
            to contributions,
            components: [
                "jobs/scheduler-component" => type JobScheduler,
            ],
        }
    }
}
