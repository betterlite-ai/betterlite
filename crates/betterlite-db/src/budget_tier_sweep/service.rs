use chrono::{DateTime, Utc};

use crate::budget_cascade::BudgetCascade;
use crate::budget_permission_rows::LiteLLMBudgetTableFull;
use crate::budget_reset_policy::cache_invalidation::ResetCache;
use crate::budget_reset_policy::chunk_outcome::{ChunkOutcome, NO_PROGRESS};
use crate::budget_reset_policy::entity_reset::ResetSchedule;
use crate::budget_reset_policy::phase_draining::{ResetLimits, phase_is_drained};
use crate::budget_reset_policy::phase_failure::{
    PhaseFailureSink, PhaseTiming, report_phase_failure,
};
use crate::budget_reset_policy::rollover::RolloverPolicy;
use crate::database_faults::DbError;
use crate::database_faults::reconnect::DbReconnect;
use crate::database_refresh_loop::clock::RefreshClock;
use crate::identity_budget_reset::chunk_outcome::progress;
use crate::identity_budget_reset::service_hooks::{PhaseSuccessSink, report_phase_success};
use crate::postgres_store::session::PostgresStore;

use super::enduser_invalidation::{EndUserWalkRequest, invalidate_enduser_caches};
use super::enduser_page_truncation::EndUserWalk;
use super::tier_cascade::{TierCascadeFailure, TierCascadeRequest, run_tier_cascade};
use super::tier_failure::{FAILURE_CALL_TYPE, cascade_failure_message, failure_metadata};
use super::tier_progress::{SUCCESS_CALL_TYPE, cascade_metadata, success_metadata};
use super::tier_reads::read_due_tiers;

// ResetBudgetJob's budget-tier phase: the ports arrive from the composition root, which owns the schedule, caches, hooks and reconnection.
pub struct BudgetTierSweep<'ports, Schedule, Cache, Failures, Successes, Reconnect, Clock> {
    pub store: PostgresStore,
    pub limits: ResetLimits,
    pub rollover: RolloverPolicy,
    pub default_enduser_budget_id: Option<&'ports str>,
    pub schedule: &'ports Schedule,
    pub cache: &'ports Cache,
    pub failure: &'ports Failures,
    pub success: &'ports Successes,
    pub reconnect: Option<&'ports mut Reconnect>,
    pub clock: &'ports Clock,
}

impl<Schedule, Cache, Failures, Successes, Reconnect, Clock>
    BudgetTierSweep<'_, Schedule, Cache, Failures, Successes, Reconnect, Clock>
where
    Schedule: ResetSchedule + Sync,
    Cache: ResetCache,
    Failures: PhaseFailureSink + Sync,
    Successes: PhaseSuccessSink + Sync,
    Reconnect: DbReconnect + Send,
    Clock: RefreshClock,
{
    // _run_phase_in_chunks: at most RESET_BUDGET_JOB_MAX_CHUNKS_PER_RUN chunks, stopping at the first one that comes back drained.
    pub async fn reset(&mut self) {
        for _ in 0..self.limits.max_chunks_per_run {
            let outcome = self.chunk().await;
            if phase_is_drained(outcome, self.limits.batch_size) {
                return;
            }
        }
    }

    async fn chunk(&mut self) -> ChunkOutcome {
        let started = self.clock.now();
        let Some(budgets) = self.read_chunk(started).await else {
            return NO_PROGRESS;
        };
        let request = TierCascadeRequest {
            budgets: &budgets,
            rollover: self.rollover,
            schedule: self.schedule,
            default_enduser_budget_id: self.default_enduser_budget_id,
        };
        let cascade = match self.commit_chunk(request).await {
            Ok(cascade) => cascade,
            Err(failure) => {
                self.report_failure(&failure, started);
                return NO_PROGRESS;
            }
        };
        self.finish_chunk(&cascade, started).await
    }

    // A failed read reports the empty cascade's counts and returns no progress, so the phase stops and the tiers stay queued.
    async fn read_chunk(&mut self, now: DateTime<Utc>) -> Option<Vec<LiteLLMBudgetTableFull>> {
        let reconnect = self.reconnect.as_deref_mut();
        let read = read_due_tiers(&self.store, now, self.limits.batch_size, reconnect).await;
        match read {
            Ok(budgets) => Some(budgets),
            Err(error) => {
                self.report_read_failure(&error, now);
                None
            }
        }
    }

    async fn commit_chunk(
        &mut self,
        request: TierCascadeRequest<'_, Schedule>,
    ) -> Result<BudgetCascade, TierCascadeFailure<Schedule::Failure>> {
        let reconnect = self.reconnect.as_deref_mut();
        run_tier_cascade(&self.store, self.cache, request, reconnect).await
    }

    // _invalidate_budget_cascade_caches runs inside the commit, then the customer walk, the success hook and the progress count follow in that order.
    async fn finish_chunk(
        &mut self,
        cascade: &BudgetCascade,
        started: DateTime<Utc>,
    ) -> ChunkOutcome {
        let cutoff = self.clock.now();
        let walk = self.walk_customers(cascade).await;
        let timing = self.timing(started);
        let metadata = success_metadata(cascade.budgets.len(), &walk);
        report_phase_success(self.success, SUCCESS_CALL_TYPE, timing, metadata);
        progress(
            cascade.budgets.len(),
            cascade
                .budget_resets
                .iter()
                .map(|reset| Some(reset.reset_at)),
            cutoff,
        )
    }

    async fn walk_customers(&mut self, cascade: &BudgetCascade) -> EndUserWalk {
        let request = EndUserWalkRequest {
            budget_ids: &cascade.budget_ids,
            default_enduser_budget_id: self.default_enduser_budget_id,
            batch_size: self.limits.batch_size,
        };
        let reconnect = self.reconnect.as_deref_mut();
        invalidate_enduser_caches(&self.store, self.cache, request, reconnect).await
    }

    fn report_read_failure(&self, error: &DbError, started: DateTime<Utc>) {
        let timing = self.timing(started);
        let message = error.to_string();
        let metadata = cascade_metadata(0, 0);
        report_phase_failure(self.failure, FAILURE_CALL_TYPE, &message, timing, metadata);
    }

    fn report_failure(
        &self,
        failure: &TierCascadeFailure<Schedule::Failure>,
        started: DateTime<Utc>,
    ) {
        let timing = self.timing(started);
        let metadata = failure_metadata(failure);
        let message = cascade_failure_message(failure);
        report_phase_failure(self.failure, FAILURE_CALL_TYPE, &message, timing, metadata);
    }

    fn timing(&self, started: DateTime<Utc>) -> PhaseTiming {
        PhaseTiming {
            start_time: started,
            end_time: self.clock.now(),
        }
    }
}
