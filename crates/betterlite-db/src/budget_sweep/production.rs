use std::sync::Arc;

use tokio_util::task::TaskTracker;

use crate::budget_reset_policy::cache_invalidation::ResetCache;
use crate::budget_reset_policy::entity_reset::ResetSchedule;
use crate::budget_reset_policy::phase_failure::PhaseFailureSink;
use crate::database_faults::reconnect::DbReconnect;
use crate::database_refresh_loop::clock::RefreshClock;
use crate::identity_budget_reset::service_hooks::PhaseSuccessSink;
use crate::postgres_store::session::PostgresStore;
use crate::queues::pod_lock::{LockCache, PodLockManager};
use crate::team_window_reset::cache::WindowCounterCache;
use crate::team_window_reset::window_cache_roll::WindowDurationSeconds;

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

impl<Lock, Schedule, Cache, Failures, Successes, Reconnect, Clock, Durations>
    BudgetSweep<
        Lock,
        SweepPhases<Schedule, Cache, Failures, Successes, Reconnect, Clock, Durations>,
    >
where
    Lock: LockCache + 'static,
    Schedule: ResetSchedule + Send + Sync,
    Cache: ResetCache + WindowCounterCache,
    Failures: PhaseFailureSink + Send + Sync,
    Successes: PhaseSuccessSink + Send + Sync,
    Reconnect: DbReconnect + Send,
    Clock: RefreshClock,
    Durations: WindowDurationSeconds + Send + Sync,
{
    // reset_settings or get_budget_reset_settings(): the explicit settings win and the injected snapshot is read once, at construction.
    #[must_use]
    pub fn production<Snapshot>(
        inputs: SweepInputs<
            Lock,
            Schedule,
            Cache,
            Failures,
            Successes,
            Reconnect,
            Clock,
            Durations,
        >,
        snapshot: &Snapshot,
    ) -> Self
    where
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
        Self {
            database: inputs.database,
            settings,
            lock_manager: inputs.lock_manager,
            phases,
            tasks: inputs.tasks,
        }
    }
}
