use std::sync::Arc;

use betterlite_db::database_failures::metrics::WallClock;
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_db::ptu::charges::records::RollupResult;
use betterlite_db::ptu::rollup::RollupError;
use betterlite_db::ptu::rollup::alert::OperatorAlert;
use betterlite_db::ptu::rollup::scheduled::{ScheduledRun, run_scheduled_ptu_rollup};
use betterlite_db::ptu::store::sleeper::{Sleeper, TokioSleeper};
use betterlite_db::queues::pod_lock::{LockCache, PodLockManager};
use futures_util::future::BoxFuture;

use super::attribution::PtuAttribution;
use super::snapshot::CatalogSnapshot;
use super::{JobTick, PTU_ROLLUP_SCHEDULE, PtuRollupRegistration};

// What the spend writer hands the daily job: its database, the startup catalog, its one lock owner, its alert channel and this process's clock.
pub struct PtuRollupInputs<L: LockCache, A: OperatorAlert> {
    pub store: PostgresStore,
    pub deployments: Arc<CatalogSnapshot>,
    pub lock: Arc<PodLockManager<L>>,
    pub alerts: Option<Arc<A>>,
    pub attribution: Arc<dyn PtuAttribution>,
    pub clock: Arc<dyn WallClock>,
}

// _scheduled_ptu_rollup: one uncancelled call per tick, borrowing the handles it was built with so no tick mints another lock owner.
pub struct ScheduledPtuRollup<L: LockCache, A: OperatorAlert> {
    pub inputs: PtuRollupInputs<L, A>,
}

impl<L: LockCache + 'static, A: OperatorAlert + 'static> ScheduledPtuRollup<L, A> {
    pub async fn run<S>(&self) -> Result<Option<RollupResult>, RollupError>
    where
        S: Sleeper + Default + Send,
    {
        let mut sleeper = S::default();
        run_scheduled_ptu_rollup(ScheduledRun {
            store: &self.inputs.store,
            entries: self.inputs.deployments.entries.as_slice(),
            clock: self.inputs.clock.as_ref(),
            sleeper: &mut sleeper,
            lock: Some(self.inputs.lock.as_ref()),
            alert: self.inputs.alerts.as_deref(),
            attribution_enabled: self.inputs.attribution.enabled(),
            target_date: None,
        })
        .await
    }

    // One scheduler tick, with the run's failure logged inside the future it returns.
    async fn tick(&self) {
        if let Err(error) = self.run::<TokioSleeper>().await {
            tracing::error!(error = %error, "PTU rollup: the scheduled run failed");
        }
    }

    // The registration the scheduler takes, absent when attribution is off at startup; a failed run is logged and never detached.
    #[must_use]
    pub fn registration(self: &Arc<Self>) -> Option<PtuRollupRegistration> {
        if !self.inputs.attribution.enabled() {
            return None;
        }
        tracing::info!(
            "PTU rollup job scheduled at 00:15 UTC daily (only models with PTU config accrue flat cost)"
        );
        let job = Arc::clone(self);
        let run: JobTick = Arc::new(move || -> BoxFuture<'static, ()> {
            let tick = Arc::clone(&job);
            Box::pin(async move { tick.tick().await })
        });
        Some(PtuRollupRegistration {
            schedule: PTU_ROLLUP_SCHEDULE,
            run,
        })
    }
}
