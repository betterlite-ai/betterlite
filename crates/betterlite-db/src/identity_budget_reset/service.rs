use chrono::{DateTime, Utc};

use crate::budget_reset_policy::cache_invalidation::ResetCache;
use crate::budget_reset_policy::chunk_outcome::{ChunkOutcome, NO_PROGRESS};
use crate::budget_reset_policy::entity_reset::ResetSchedule;
use crate::budget_reset_policy::invalidation_order::commit_then_invalidate;
use crate::budget_reset_policy::lazy_json::LazyJson;
use crate::budget_reset_policy::phase_draining::{ResetLimits, phase_is_drained};
use crate::budget_reset_policy::phase_failure::{
    PhaseFailureSink, PhaseTiming, report_phase_failure,
};
use crate::budget_reset_policy::retry::RetryMode;
use crate::budget_reset_policy::rollover::RolloverPolicy;
use crate::budget_reset_policy::row_failures::RowFailures;
use crate::database_faults::reconnect::{DbReconnect, call_with_db_reconnect_retry};
use crate::database_refresh_loop::clock::RefreshClock;
use crate::postgres_store::session::PostgresStore;

use super::chunk_outcome::progress;
use super::counter_invalidation::spend_counter_keys;
use super::faults::Fault;
use super::global_proxy::accumulator_cache_keys;
use super::per_row_failure::collect_resets;
use super::phase::IdentityPhase;
use super::rows::{DueIdentity, read_due};
use super::service_hooks::{
    PhaseSuccessSink, found_metadata, outcome_metadata, report_phase_success,
};
use super::write_transactional::{ResetWrite, write_resets};

// ResetBudgetJob's key and user phases: the ports arrive from the composition root, which owns the schedule, caches, hooks and reconnection.
pub struct IdentityBudgetReset<'ports, Schedule, Cache, Failures, Successes, Reconnect, Clock> {
    pub store: PostgresStore,
    pub limits: ResetLimits,
    pub rollover: RolloverPolicy,
    pub schedule: &'ports Schedule,
    pub cache: &'ports Cache,
    pub failure: &'ports Failures,
    pub success: &'ports Successes,
    pub reconnect: Option<&'ports mut Reconnect>,
    pub clock: &'ports Clock,
}

impl<Schedule, Cache, Failures, Successes, Reconnect, Clock>
    IdentityBudgetReset<'_, Schedule, Cache, Failures, Successes, Reconnect, Clock>
where
    Schedule: ResetSchedule + Sync,
    Cache: ResetCache,
    Failures: PhaseFailureSink + Sync,
    Successes: PhaseSuccessSink + Sync,
    Reconnect: DbReconnect + Send,
    Clock: RefreshClock,
{
    // _run_phase_in_chunks: at most RESET_BUDGET_JOB_MAX_CHUNKS_PER_RUN chunks, stopping at the first one that comes back drained.
    pub async fn reset(&mut self, phase: IdentityPhase) {
        for _ in 0..self.limits.max_chunks_per_run {
            let outcome = self.chunk(phase).await;
            if phase_is_drained(outcome, self.limits.batch_size) {
                return;
            }
        }
    }

    async fn chunk(&mut self, phase: IdentityPhase) -> ChunkOutcome {
        let started = self.clock.now();
        let read = self.read_chunk(phase, started).await;
        let Some(due) = read else {
            return NO_PROGRESS;
        };
        if phase == IdentityPhase::Keys {
            tracing::debug!(keys = ?LazyJson(&due), "Keys to reset");
        }
        let (writes, failures) = collect_resets(phase, &due, self.rollover, self.schedule);
        if !due.is_empty() {
            tracing::debug!(subject = phase.subject(), rows = ?LazyJson(&writes), "Updated budget rows");
        }
        let fetched = due.len();
        let report = ChunkReport {
            fetched,
            updated: writes.len(),
            failures,
        };
        let reset_ats: Vec<Option<DateTime<Utc>>> =
            writes.iter().map(|write| write.budget_reset_at).collect();
        let committed = if writes.is_empty() {
            Ok(())
        } else {
            self.write_chunk(phase, writes).await
        };
        if let Err(error) = committed {
            let timing = self.timing(started);
            let metadata = found_metadata(phase, fetched);
            let message = error.to_string();
            report_phase_failure(self.failure, phase.call_type(), &message, timing, metadata);
            return NO_PROGRESS;
        }
        self.report_outcome(phase, &report, started);
        progress(fetched, reset_ats, self.clock.now())
    }

    async fn read_chunk(
        &mut self,
        phase: IdentityPhase,
        now: DateTime<Utc>,
    ) -> Option<Vec<DueIdentity>> {
        let outcome = read_with_retry(
            &self.store,
            phase,
            now,
            self.limits.batch_size,
            self.reconnect.as_deref_mut(),
        )
        .await;
        match outcome {
            Ok(due) => Some(due),
            Err(error) => {
                let timing = self.timing(now);
                let metadata = found_metadata(phase, 0);
                let message = error.to_string();
                report_phase_failure(self.failure, phase.call_type(), &message, timing, metadata);
                None
            }
        }
    }

    async fn write_chunk(
        &mut self,
        phase: IdentityPhase,
        writes: Vec<ResetWrite>,
    ) -> Result<(), Fault> {
        let counter_keys = spend_counter_keys(phase, &writes);
        let cache_keys = accumulator_cache_keys(phase, &writes);
        let store = &self.store;
        let reconnect = self.reconnect.as_deref_mut();
        let commit = move || write_resets(store, phase, writes, reconnect);
        commit_then_invalidate(self.cache, commit, &counter_keys, &cache_keys).await
    }

    fn report_outcome(&self, phase: IdentityPhase, report: &ChunkReport, started: DateTime<Utc>) {
        let timing = self.timing(started);
        let Some(summary) = report.failures.summary(phase.subject()) else {
            let failed = report.failures.rows.len();
            let metadata = outcome_metadata(phase, report.fetched, report.updated, failed);
            report_phase_success(self.success, phase.call_type(), timing, metadata);
            return;
        };
        let metadata = found_metadata(phase, report.fetched);
        report_phase_failure(self.failure, phase.call_type(), &summary, timing, metadata);
    }

    fn timing(&self, started: DateTime<Utc>) -> PhaseTiming {
        PhaseTiming {
            start_time: started,
            end_time: self.clock.now(),
        }
    }
}

struct ChunkReport {
    fetched: usize,
    updated: usize,
    failures: RowFailures,
}

async fn read_with_retry<Reconnect>(
    store: &PostgresStore,
    phase: IdentityPhase,
    now: DateTime<Utc>,
    limit: usize,
    reconnect: Option<&mut Reconnect>,
) -> Result<Vec<DueIdentity>, Fault>
where
    Reconnect: DbReconnect + Send,
{
    let attempt = move || read_due(store, phase, now, limit);
    let policy = RetryMode::Read.policy(phase.read_reason());
    call_with_db_reconnect_retry(reconnect, attempt, policy).await
}

mod tests;
