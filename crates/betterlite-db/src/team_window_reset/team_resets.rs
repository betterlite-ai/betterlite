use betterlite_models::decisions::Json;
use chrono::{DateTime, Utc};
use serde_json::Number;

use crate::budget_reset_policy::cache_invalidation::ResetCache;
use crate::budget_reset_policy::chunk_outcome::{ChunkOutcome, NO_PROGRESS};
use crate::budget_reset_policy::entity_reset::ResetSchedule;
use crate::budget_reset_policy::invalidation_order::commit_then_invalidate;
use crate::budget_reset_policy::lazy_json::LazyJson;
use crate::budget_reset_policy::phase_draining::{ResetLimits, phase_is_drained};
use crate::budget_reset_policy::phase_failure::{
    EventMetadata, PhaseFailureSink, PhaseTiming, report_phase_failure,
};
use crate::budget_reset_policy::rollover::RolloverPolicy;
use crate::budget_reset_policy::row_failures::RowFailures;
use crate::database_faults::reconnect::DbReconnect;
use crate::database_refresh_loop::clock::RefreshClock;
use crate::identity_budget_reset::chunk_outcome::progress;
use crate::identity_budget_reset::faults::{Fault, describe};
use crate::identity_budget_reset::rows::DueIdentity;
use crate::identity_budget_reset::service_hooks::{PhaseSuccessSink, report_phase_success};
use crate::identity_budget_reset::spend_decrement::reset_due_row;
use crate::identity_budget_reset::write_transactional::ResetWrite;
use crate::postgres_store::session::PostgresStore;

use super::team_reads::read_due_teams;
use super::team_writes::{team_counter_keys, write_team_resets};

const CALL_TYPE: &str = "reset_budget_teams";
const SUBJECT: &str = "teams";
const FOUND_KEY: &str = "num_teams_found";
const UPDATED_KEY: &str = "num_teams_updated";
const FAILED_KEY: &str = "num_teams_failed";

// reset_budget_for_litellm_teams: B3009's reset helper per due team, one unit-of-work write per chunk, then the spend:team counters drop.
pub struct TeamBudgetReset<'ports, Schedule, Cache, Failures, Successes, Reconnect, Clock> {
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
    TeamBudgetReset<'_, Schedule, Cache, Failures, Successes, Reconnect, Clock>
where
    Schedule: ResetSchedule + Sync,
    Cache: ResetCache,
    Failures: PhaseFailureSink + Sync,
    Successes: PhaseSuccessSink + Sync,
    Reconnect: DbReconnect + Send,
    Clock: RefreshClock,
{
    // _run_phase_in_chunks: at most RESET_BUDGET_JOB_MAX_CHUNKS_PER_RUN chunks, stopping at the first one that comes back drained.
    pub async fn reset_teams(&mut self) {
        for _ in 0..self.limits.max_chunks_per_run {
            let outcome = self.chunk().await;
            if phase_is_drained(outcome, self.limits.batch_size) {
                return;
            }
        }
    }

    async fn chunk(&mut self) -> ChunkOutcome {
        let started = self.clock.now();
        let Some(teams) = self.read_teams(started).await else {
            return NO_PROGRESS;
        };
        tracing::debug!(teams = ?LazyJson(&teams), "Teams due for a budget reset");
        let mut resets = TeamResets::default();
        resets.collect(&teams, self.rollover, self.schedule);
        let found = teams.len();
        let updated = resets.writes.len();
        let reset_ats: Vec<Option<DateTime<Utc>>> = resets
            .writes
            .iter()
            .map(|write| write.budget_reset_at)
            .collect();
        let committed = if resets.writes.is_empty() {
            Ok(())
        } else {
            self.commit(resets.writes).await
        };
        if let Err(error) = committed {
            let message = error.to_string();
            self.report_failure(started, &message, found);
            return NO_PROGRESS;
        }
        self.report_outcome(&resets.failures, found, updated, started);
        progress(found, reset_ats, self.clock.now())
    }

    async fn read_teams(&mut self, now: DateTime<Utc>) -> Option<Vec<DueIdentity>> {
        let store = &self.store;
        let reconnect = self.reconnect.as_deref_mut();
        let outcome = read_due_teams(store, now, self.limits.batch_size, reconnect).await;
        match outcome {
            Ok(teams) => Some(teams),
            Err(error) => {
                let message = error.to_string();
                self.report_failure(now, &message, 0);
                None
            }
        }
    }

    async fn commit(&mut self, writes: Vec<ResetWrite>) -> Result<(), Fault> {
        let counter_keys = team_counter_keys(&writes);
        let store = &self.store;
        let reconnect = self.reconnect.as_deref_mut();
        let commit = move || write_team_resets(store, writes, reconnect);
        commit_then_invalidate(self.cache, commit, &counter_keys, &[]).await
    }

    fn report_outcome(
        &self,
        failures: &RowFailures,
        found: usize,
        updated: usize,
        started: DateTime<Utc>,
    ) {
        let timing = self.timing(started);
        let Some(summary) = failures.summary(SUBJECT) else {
            let counts = team_counts(found, updated, failures.rows.len());
            report_phase_success(self.success, CALL_TYPE, timing, counts);
            return;
        };
        report_phase_failure(
            self.failure,
            CALL_TYPE,
            &summary,
            timing,
            EventMetadata::from([(FOUND_KEY, Json::Number(Number::from(found)))]),
        );
    }

    fn report_failure(&self, started: DateTime<Utc>, message: &str, found: usize) {
        report_phase_failure(
            self.failure,
            CALL_TYPE,
            message,
            self.timing(started),
            EventMetadata::from([(FOUND_KEY, Json::Number(Number::from(found)))]),
        );
    }

    fn timing(&self, started: DateTime<Utc>) -> PhaseTiming {
        PhaseTiming {
            start_time: started,
            end_time: self.clock.now(),
        }
    }
}

fn team_counts(found: usize, updated: usize, failed: usize) -> EventMetadata {
    EventMetadata::from([
        (FOUND_KEY, Json::Number(Number::from(found))),
        (UPDATED_KEY, Json::Number(Number::from(updated))),
        (FAILED_KEY, Json::Number(Number::from(failed))),
    ])
}

#[derive(Debug, Default)]
struct TeamResets {
    writes: Vec<ResetWrite>,
    failures: RowFailures,
}

impl TeamResets {
    // A team whose reset raised is collected and the chunk moves on: the rows that did reset are written, and the failure hook reports the rest.
    fn collect<Schedule>(
        &mut self,
        rows: &[DueIdentity],
        rollover: RolloverPolicy,
        schedule: &Schedule,
    ) where
        Schedule: ResetSchedule,
    {
        for row in rows {
            match reset_due_row(row, rollover, schedule) {
                Ok(write) => self.writes.push(write),
                Err(failure) => {
                    let message = describe(&failure);
                    tracing::error!(
                        team = %row.identifier,
                        error = %message,
                        "Failed to reset budget for team"
                    );
                    self.failures.record(&row.identifier, &message);
                }
            }
        }
    }
}

mod tests;
