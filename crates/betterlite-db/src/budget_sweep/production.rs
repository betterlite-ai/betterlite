use std::sync::Arc;

use tokio_util::task::TaskTracker;

use crate::postgres_store::session::PostgresStore;
use crate::queues::pod_lock::{LockCache, PodLockManager};

use super::BudgetSweep;
use super::phase::{SweepPhases, SweepPorts};
use super::settings::{BudgetResetSettings, ResetSettingsSource};

pub struct SweepInputs<Lock, Schedule, Cache, Failures, Successes, Reconnect, Clock, Durations>
where
    Lock: LockCache,
{
    pub database: PostgresStore,
    pub settings: Option<BudgetResetSettings>,
    pub lock_manager: Option<Arc<PodLockManager<Lock>>>,
    pub ports: SweepPorts<Schedule, Cache, Failures, Successes, Reconnect, Clock, Durations>,
    pub tasks: TaskTracker,
}

// reset_settings or get_budget_reset_settings(): the explicit settings win and the injected snapshot is read once, at construction.
#[must_use]
pub fn production<
    Lock,
    Schedule,
    Cache,
    Failures,
    Successes,
    Reconnect,
    Clock,
    Durations,
    Snapshot,
>(
    inputs: SweepInputs<Lock, Schedule, Cache, Failures, Successes, Reconnect, Clock, Durations>,
    snapshot: &Snapshot,
) -> BudgetSweep<Lock, SweepPhases<Schedule, Cache, Failures, Successes, Reconnect, Clock, Durations>>
where
    Lock: LockCache,
    Snapshot: ResetSettingsSource,
{
    let settings = inputs
        .settings
        .unwrap_or_else(|| snapshot.budget_reset_settings());
    let phases = SweepPhases {
        settings: settings.clone(),
        store: inputs.database.clone(),
        ports: inputs.ports,
    };
    BudgetSweep {
        database: inputs.database,
        settings,
        lock_manager: inputs.lock_manager,
        phases,
        tasks: inputs.tasks,
    }
}
