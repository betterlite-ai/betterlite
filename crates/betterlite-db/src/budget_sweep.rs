pub mod error;
pub mod lease;
pub mod lease_guard;
pub mod phase;
pub mod production;
pub mod settings;

mod tests;

use std::sync::Arc;

use tokio_util::task::TaskTracker;

use crate::postgres_store::session::PostgresStore;
use crate::queues::pod_lock::{LockCache, PodLockManager};

use self::error::SweepError;
use self::lease::{Lease, RESET_JOB_NAME, elect};
use self::lease_guard::LeaseGuard;
use self::phase::{PhaseRunner, ResetPhase};
use self::settings::BudgetResetSettings;

pub struct BudgetSweep<Cache, Phases>
where
    Cache: LockCache,
{
    pub database: PostgresStore,
    pub settings: BudgetResetSettings,
    pub lock_manager: Option<Arc<PodLockManager<Cache>>>,
    pub phases: Phases,
    pub tasks: TaskTracker,
}

impl<Cache, Phases> BudgetSweep<Cache, Phases>
where
    Cache: LockCache + 'static,
    Phases: PhaseRunner + Send,
{
    // reset_budget: no database sweeps nothing, a follower skips the tick, and a leader owes its lease release.
    pub async fn reset_budget(&mut self) -> Result<(), SweepError<Phases::Failure>> {
        if self.database.pool.is_none() {
            return Ok(());
        }
        let lease = elect(self.lock_manager.as_ref()).await;
        if lease == Lease::Follower {
            return Ok(());
        }
        let release = self.leader_release(lease);
        let outcome = self.run_phases().await;
        if let Some(pending_release) = release {
            pending_release.release().await;
        }
        outcome
    }

    // Only an invocation that acquired the lease releases it; unguarded and follower runs never touch another pod's lock.
    fn leader_release(&self, lease: Lease) -> Option<LeaseGuard> {
        let Lease::Leader = lease else {
            return None;
        };
        let manager = Arc::clone(self.lock_manager.as_ref()?);
        Some(LeaseGuard::start(manager, RESET_JOB_NAME, &self.tasks))
    }

    async fn run_phases(&mut self) -> Result<(), SweepError<Phases::Failure>> {
        for phase in ResetPhase::ALL {
            self.phases
                .run(phase)
                .await
                .map_err(|source| SweepError::Phase { phase, source })?;
        }
        Ok(())
    }
}
