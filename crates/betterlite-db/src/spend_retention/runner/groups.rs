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

// the retention horizon and the deadline one enabled group works under
#[derive(Debug, Clone, Copy)]
pub(super) struct GroupWindow {
    pub retention_seconds: i64,
    pub deadline: Instant,
}

// one _delete_old_* call with the source's own per-table log line
#[derive(Clone, Copy)]
struct TableStep {
    table: CleanupTable,
    message: &'static str,
}

// the tables each group deletes and the source's log line for each, in the order it runs them
static SPEND_LOG_STEPS: [TableStep; 2] = [
    TableStep {
        table: CleanupTable::SpendLogs,
        message: "logs",
    },
    TableStep {
        table: CleanupTable::ToolIndex,
        message: "expired tool index rows",
    },
];
static SESSION_STEPS: [TableStep; 2] = [
    TableStep {
        table: CleanupTable::AutoRouterSessions,
        message: "expired auto-router session rollup rows",
    },
    TableStep {
        table: CleanupTable::AutoRouterUserSessions,
        message: "expired auto-router user session rollup rows",
    },
];
static HEALTH_CHECK_STEPS: [TableStep; 1] = [TableStep {
    table: CleanupTable::HealthChecks,
    message: "expired health-check rows",
}];
static DAILY_TAG_STEPS: [TableStep; 1] = [TableStep {
    table: CleanupTable::DailyTagSpend,
    message: "expired daily tag spend rows",
}];

// the four _clean_* groups the source runs, each over its own member tables
#[derive(Debug, Clone, Copy)]
pub(super) enum GroupPlan {
    SpendLogs,
    Sessions,
    HealthChecks,
    DailyTags,
}

// the four _clean_* groups and the six table deletions they issue, in the order the source runs them
impl<Clock, Durations, Partitions, Lock> SpendLogCleanup<Clock, Durations, Partitions, Lock>
where
    Clock: CleanupClock,
    Durations: DurationParser,
    Partitions: PartitionMaintenance,
{
    // one of the source's four _clean_* groups: its cutoff, the partition work it starts first, then its tables
    pub(super) async fn clean_group(
        &self,
        run: &mut GroupRun<'_, '_, Clock>,
        plan: GroupPlan,
        window: GroupWindow,
    ) -> Result<Vec<TableCleanupResult>, RunFailure> {
        let moment = retention_cutoff(self.clock.utc_now(), window.retention_seconds)?;
        let steps: &[TableStep] = match plan {
            GroupPlan::SpendLogs => &SPEND_LOG_STEPS,
            GroupPlan::Sessions => &SESSION_STEPS,
            GroupPlan::HealthChecks => &HEALTH_CHECK_STEPS,
            GroupPlan::DailyTags => &DAILY_TAG_STEPS,
        };
        let cutoff = match plan {
            GroupPlan::DailyTags => Cutoff::Day(moment.date_naive()),
            GroupPlan::SpendLogs | GroupPlan::Sessions | GroupPlan::HealthChecks => {
                Cutoff::Timestamp(moment)
            }
        };
        if matches!(plan, GroupPlan::SpendLogs) {
            tracing::info!("Removing logs older than {}", cutoff.text());
            self.maintain_partitions(run, moment, window.deadline)
                .await?;
        }
        let mut pending = u32::try_from(steps.len()).unwrap_or(1);
        let mut results = Vec::new();
        for step in steps {
            let share = if pending > 1 {
                group_deadline(window.deadline, self.clock.now(), pending)
            } else {
                window.deadline
            };
            results.push(self.delete_step(run, *step, cutoff, share).await);
            pending = pending.saturating_sub(1);
        }
        Ok(results)
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

    // one of the six _delete_old_* callers, told which table, its log line and its horizon
    async fn delete_step(
        &self,
        run: &mut GroupRun<'_, '_, Clock>,
        step: TableStep,
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
        let result = deleter.delete_expired(step.table, cutoff, deadline).await;
        tracing::info!("Deleted {} {}", result.rows_deleted, step.message);
        result
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
