use chrono::{DateTime, TimeDelta, Utc};
use tokio::time::Instant;
use tokio_postgres::Client;

use crate::cleanup_metrics::CleanupMetrics;
use crate::spend_retention::batches::cutoff::Cutoff;
use crate::spend_retention::batches::deadline::{MonotonicClock, group_deadline, remaining_budget};
use crate::spend_retention::batches::deleter::BatchDeleter;
use crate::spend_retention::batches::settings::{CleanupBounds, DurationParser, SettingValue};
use crate::spend_retention::batches::table::CleanupTable;
use crate::spend_retention::batches::{SpendLogCleanup, TableCleanupResult};

mod tests;

use super::clock::CleanupClock;
use super::error::RunFailure;
use super::partitions::PartitionMaintenance;
use super::progress::{RunProgress, SharedProgress};

// the collaborators and live bounds every group of one run reads, so each group receives them from one place
pub(super) struct GroupRun<'run, 'registry, Clock> {
    pub client: &'run mut Client,
    pub clock: &'run Clock,
    pub metrics: &'run CleanupMetrics<'registry>,
    pub bounds: CleanupBounds,
    pub progress: &'run RunProgress,
    pub started: Instant,
}

// the four _clean_* groups and the six table deletions they issue, in the order the source runs them
impl<Clock, Durations, Partitions, Lock> SpendLogCleanup<Clock, Durations, Partitions, Lock>
where
    Clock: CleanupClock,
    Durations: DurationParser,
    Partitions: PartitionMaintenance,
{
    // _clean_spend_log_tables: drop expired partitions first, then delete the stragglers they cannot reclaim
    pub(super) async fn clean_spend_log_tables(
        &self,
        run: &mut GroupRun<'_, '_, Clock>,
        retention_seconds: i64,
        deadline: Instant,
    ) -> Result<Vec<TableCleanupResult>, RunFailure> {
        let moment = retention_cutoff(self.clock.utc_now(), retention_seconds)?;
        let cutoff = Cutoff::Timestamp(moment);
        tracing::info!("Removing logs older than {}", cutoff.text());
        self.maintain_partitions(run, moment, deadline).await?;
        let logs_deadline = group_deadline(deadline, self.clock.now(), 2);
        let logs = self
            .delete_table(run, CleanupTable::SpendLogs, cutoff, logs_deadline)
            .await;
        tracing::info!("Deleted {} logs", logs.rows_deleted);
        let index = self
            .delete_table(run, CleanupTable::ToolIndex, cutoff, deadline)
            .await;
        tracing::info!("Deleted {} expired tool index rows", index.rows_deleted);
        Ok(vec![logs, index])
    }

    // _clean_session_rollup: both session tables carry their own retention horizon, split between them
    pub(super) async fn clean_session_rollup(
        &self,
        run: &mut GroupRun<'_, '_, Clock>,
        retention_seconds: i64,
        deadline: Instant,
    ) -> Result<Vec<TableCleanupResult>, RunFailure> {
        let moment = retention_cutoff(self.clock.utc_now(), retention_seconds)?;
        let cutoff = Cutoff::Timestamp(moment);
        let sessions_deadline = group_deadline(deadline, self.clock.now(), 2);
        let sessions = self
            .delete_table(
                run,
                CleanupTable::AutoRouterSessions,
                cutoff,
                sessions_deadline,
            )
            .await;
        tracing::info!(
            "Deleted {} expired auto-router session rollup rows",
            sessions.rows_deleted
        );
        let user_sessions = self
            .delete_table(run, CleanupTable::AutoRouterUserSessions, cutoff, deadline)
            .await;
        tracing::info!(
            "Deleted {} expired auto-router user session rollup rows",
            user_sessions.rows_deleted
        );
        Ok(vec![sessions, user_sessions])
    }

    // _clean_health_checks
    pub(super) async fn clean_health_checks(
        &self,
        run: &mut GroupRun<'_, '_, Clock>,
        retention_seconds: i64,
        deadline: Instant,
    ) -> Result<Vec<TableCleanupResult>, RunFailure> {
        let moment = retention_cutoff(self.clock.utc_now(), retention_seconds)?;
        let cutoff = Cutoff::Timestamp(moment);
        let result = self
            .delete_table(run, CleanupTable::HealthChecks, cutoff, deadline)
            .await;
        tracing::info!("Deleted {} expired health-check rows", result.rows_deleted);
        Ok(vec![result])
    }

    // _clean_daily_tag_spend: the horizon day itself is kept, and so is every row keyed by a non-ISO day
    pub(super) async fn clean_daily_tag_spend(
        &self,
        run: &mut GroupRun<'_, '_, Clock>,
        retention_seconds: i64,
        deadline: Instant,
    ) -> Result<Vec<TableCleanupResult>, RunFailure> {
        let horizon = retention_cutoff(self.clock.utc_now(), retention_seconds)?;
        let cutoff = Cutoff::Day(horizon.date_naive());
        let result = self
            .delete_table(run, CleanupTable::DailyTagSpend, cutoff, deadline)
            .await;
        tracing::info!(
            "Deleted {} expired daily tag spend rows",
            result.rows_deleted
        );
        Ok(vec![result])
    }

    // partition DDL is started only with budget left, only behind the switch, and only on a partitioned table
    async fn maintain_partitions(
        &self,
        run: &mut GroupRun<'_, '_, Clock>,
        moment: DateTime<Utc>,
        deadline: Instant,
    ) -> Result<(), RunFailure> {
        let mut remaining = remaining_budget(&self.clock, deadline, run.bounds.batch_timeout);
        if self.clock.now() >= deadline {
            tracing::info!("Run budget already spent, skipping partition maintenance this run");
            return Ok(());
        }
        if !matches!(
            self.settings.general.use_spend_logs_partitioning,
            Some(SettingValue::Flag(true))
        ) {
            return Ok(());
        }
        if !self
            .partitions
            .is_partitioned(run.client, &mut remaining)
            .await
        {
            return Ok(());
        }
        let ensured = self
            .partitions
            .ensure_partitions(run.client, &mut remaining)
            .await;
        drop(ensured);
        let dropped = self
            .partitions
            .drop_expired(run.client, moment, &mut remaining)
            .await?;
        tracing::info!(
            "Dropped {} expired spend-log partitions: {dropped:?}",
            dropped.len()
        );
        Ok(())
    }

    // one of the six _delete_old_* callers, told which table and horizon its group owns
    async fn delete_table(
        &self,
        run: &mut GroupRun<'_, '_, Clock>,
        table: CleanupTable,
        cutoff: Cutoff,
        deadline: Instant,
    ) -> TableCleanupResult {
        let mut progress = SharedProgress(run.progress);
        let mut deleter = BatchDeleter {
            client: &mut *run.client,
            bounds: run.bounds,
            clock: run.clock,
            metrics: run.metrics,
            progress: &mut progress,
        };
        deleter.delete_expired(table, cutoff, deadline).await
    }
}

// now - retention, or a run failure where the source raises OverflowError building the same subtraction
fn retention_cutoff(now: DateTime<Utc>, seconds: i64) -> Result<DateTime<Utc>, RunFailure> {
    let delta = match TimeDelta::try_seconds(seconds) {
        Some(parsed) => parsed,
        None => return Err(RunFailure::Cutoff),
    };
    now.checked_sub_signed(delta).ok_or(RunFailure::Cutoff)
}
