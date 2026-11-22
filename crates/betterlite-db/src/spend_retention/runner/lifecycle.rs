mod tests;

use std::time::Duration;

use tokio::time::Instant;
use tokio_postgres::Client;

use crate::cleanup_metrics::{CleanupMetrics, RunOutcome};
use crate::spend_retention::batches::deadline::{MonotonicClock as _, group_deadline};
use crate::spend_retention::batches::settings::{DurationParser, Retention};
use crate::spend_retention::batches::{SpendLogCleanup, StopReason, TableCleanupResult};

use super::SPEND_LOG_CLEANUP_JOB_NAME;
use super::clock::CleanupClock;
use super::error::{RunError, RunFailure};
use super::groups::{GroupPlan, GroupRun, GroupWindow};
use super::lock::CleanupLock;
use super::partitions::PartitionMaintenance;
use super::progress::RunProgress;

// the four retention horizons one run reads at entry, each None meaning that group is disabled
#[derive(Debug, Clone, Copy)]
pub(super) struct Retentions {
    pub spend_logs: Option<i64>,
    pub sessions: Option<i64>,
    pub health_checks: Option<i64>,
    pub daily_tags: Option<i64>,
}

// _RunProgress reported on cancellation: dropping the run future is the cancellation, so Drop is the only reporter
struct Cancellation<'run, 'registry> {
    clock: &'run dyn CleanupClock,
    progress: &'run RunProgress,
    metrics: &'run CleanupMetrics<'registry>,
    started: Instant,
    finished: bool,
}

impl Drop for Cancellation<'_, '_> {
    fn drop(&mut self) {
        if self.finished {
            return;
        }
        let elapsed = self.clock.now().saturating_duration_since(self.started);
        let (rows_deleted, batches) = self.progress.counts();
        tracing::error!(
            "Spend log cleanup cancelled after {:.2}s (rows_deleted={rows_deleted}, batches={batches}); the next run resumes from here",
            elapsed.as_secs_f64()
        );
        self.metrics.record_run(RunOutcome::Aborted);
    }
}

impl<Clock, Durations, Partitions, Lock> SpendLogCleanup<Clock, Durations, Partitions, Lock>
where
    Clock: CleanupClock,
    Durations: DurationParser + Send + Sync,
    Partitions: PartitionMaintenance,
    Lock: CleanupLock,
{
    // cleanup_old_spend_logs: skipped_disabled before the lock, skipped_locked on a failed lease, then the groups
    pub async fn cleanup_old_spend_logs(
        &self,
        client: &mut Client,
        metrics: &CleanupMetrics<'_>,
    ) -> Result<(), RunError> {
        let started = self.clock.now();
        let progress = RunProgress::default();
        let mut cancellation = Cancellation {
            clock: &self.clock,
            progress: &progress,
            metrics,
            started,
            finished: false,
        };
        tracing::info!("Cleanup job triggered at {}", self.clock.utc_now());
        let retentions = self.live_retentions();
        if all_disabled(&retentions) {
            metrics.record_run(RunOutcome::SkippedDisabled);
            cancellation.finished = true;
            return Ok(());
        }
        let mut run = GroupRun {
            client,
            clock: &self.clock,
            metrics,
            bounds: self.settings.bounds(&self.durations),
            progress: &progress,
            started,
        };
        self.drive_run(&mut run, &retentions, &mut cancellation)
            .await
    }

    // _refresh_bounds and the four _retention_seconds_for calls: a long-lived instance rereads them every run
    fn live_retentions(&self) -> Retentions {
        Retentions {
            spend_logs: self
                .settings
                .retention_seconds(Retention::SpendLogs, &self.durations),
            sessions: self
                .settings
                .retention_seconds(Retention::AutorouterSessions, &self.durations),
            health_checks: self
                .settings
                .retention_seconds(Retention::HealthChecks, &self.durations),
            daily_tags: self
                .settings
                .retention_seconds(Retention::DailyTagSpend, &self.durations),
        }
    }

    // the lease is taken before the budget starts and released only when it was actually acquired
    async fn drive_run(
        &self,
        run: &mut GroupRun<'_, '_, Clock>,
        retentions: &Retentions,
        cancellation: &mut Cancellation<'_, '_>,
    ) -> Result<(), RunError> {
        let Some(lock) = self.pod_lock.as_ref().filter(|held| held.redis_backed()) else {
            self.run_enabled_groups(run, retentions).await;
            cancellation.finished = true;
            return Ok(());
        };
        if !self.acquire_pod_lock(lock, run.metrics).await {
            cancellation.finished = true;
            return Ok(());
        }
        self.run_enabled_groups(run, retentions).await;
        cancellation.finished = true;
        lock.release(SPEND_LOG_CLEANUP_JOB_NAME).await?;
        tracing::info!("Released cleanup lock");
        Ok(())
    }

    async fn acquire_pod_lock(&self, lock: &Lock, metrics: &CleanupMetrics<'_>) -> bool {
        let acquired = lock
            .acquire(SPEND_LOG_CLEANUP_JOB_NAME)
            .await
            .unwrap_or(false);
        tracing::info!(
            "Lock acquisition attempt: {} at {}",
            if acquired { "successful" } else { "failed" },
            self.clock.utc_now()
        );
        if acquired {
            return true;
        }
        tracing::info!("Another pod is already running cleanup");
        metrics.record_run(RunOutcome::SkippedLocked);
        false
    }

    // an ordinary error records aborted, logs its type and returns; only the release above reaches the caller
    async fn run_enabled_groups(&self, run: &mut GroupRun<'_, '_, Clock>, retentions: &Retentions) {
        match self.clean_groups(run, retentions).await {
            Ok(results) => {
                let outcome = run_outcome(&results);
                run.metrics.record_run(outcome);
                let elapsed = run.clock.now().saturating_duration_since(run.started);
                log_run_summary(outcome, &results, elapsed);
            }
            Err(failure) => {
                tracing::error!("Error during spend log cleanup: {failure}");
                run.metrics.record_run(RunOutcome::Aborted);
            }
        }
    }

    // the enabled groups in order, each taking an equal share of the time still pending when it starts
    async fn clean_groups(
        &self,
        run: &mut GroupRun<'_, '_, Clock>,
        retentions: &Retentions,
    ) -> Result<Vec<TableCleanupResult>, RunFailure> {
        let now = run.clock.now();
        let deadline = now.checked_add(run.bounds.run_budget).unwrap_or(now);
        let groups = [
            (retentions.spend_logs, GroupPlan::SpendLogs),
            (retentions.sessions, GroupPlan::Sessions),
            (retentions.health_checks, GroupPlan::HealthChecks),
            (retentions.daily_tags, GroupPlan::DailyTags),
        ];
        let enabled = groups
            .iter()
            .filter(|(retention, _)| retention.is_some())
            .count();
        let mut pending = u32::try_from(enabled).unwrap_or(u32::MAX);
        let mut results = Vec::new();
        for (retention, plan) in groups {
            let Some(seconds) = retention else {
                continue;
            };
            let window = GroupWindow {
                retention_seconds: seconds,
                deadline: group_deadline(deadline, run.clock.now(), pending),
            };
            results.extend(self.clean_group(run, plan, window).await?);
            pending = pending.saturating_sub(1);
        }
        Ok(results)
    }
}

const fn all_disabled(retentions: &Retentions) -> bool {
    retentions.spend_logs.is_none()
        && retentions.sessions.is_none()
        && retentions.health_checks.is_none()
        && retentions.daily_tags.is_none()
}

// _run_outcome: an abort outranks a bound, and a bound outranks a table that simply ran dry
#[must_use]
pub(super) fn run_outcome(results: &[TableCleanupResult]) -> RunOutcome {
    let reached = |reason: StopReason| results.iter().any(|result| result.stop_reason == reason);
    if reached(StopReason::Aborted) {
        return RunOutcome::Aborted;
    }
    if reached(StopReason::BudgetExhausted) {
        return RunOutcome::BudgetExhausted;
    }
    if reached(StopReason::BatchCapReached) {
        return RunOutcome::BatchCapReached;
    }
    RunOutcome::Completed
}

// _log_run_summary: completed runs are routine, so only the other outcomes warn
pub(super) fn log_run_summary(
    outcome: RunOutcome,
    results: &[TableCleanupResult],
    elapsed: Duration,
) {
    let per_table = results
        .iter()
        .map(|result| {
            format!(
                "{}: deleted={} stop_reason={}",
                result.table.spec().name,
                result.rows_deleted,
                stop_reason_text(result.stop_reason)
            )
        })
        .collect::<Vec<String>>()
        .join(", ");
    let elapsed_seconds = elapsed.as_secs_f64();
    let label = outcome.as_str();
    if outcome == RunOutcome::Completed {
        tracing::info!(
            "Spend log cleanup run finished: outcome={label} elapsed={elapsed_seconds:.1}s [{per_table}]"
        );
        return;
    }
    tracing::warn!(
        "Spend log cleanup run finished: outcome={label} elapsed={elapsed_seconds:.1}s [{per_table}]"
    );
}

const fn stop_reason_text(reason: StopReason) -> &'static str {
    match reason {
        StopReason::Exhausted => "exhausted",
        StopReason::BudgetExhausted => "budget_exhausted",
        StopReason::BatchCapReached => "batch_cap_reached",
        StopReason::Aborted => "aborted",
    }
}
