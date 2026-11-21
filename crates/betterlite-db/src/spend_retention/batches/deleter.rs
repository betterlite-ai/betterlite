use std::time::Duration;

use tokio::time::{Instant, sleep};
use tokio_postgres::{Client, Error as PostgresError};

use crate::cleanup_metrics::CleanupMetrics;
use crate::spend_partitions::catalog::{BoundedTransactionError, within_deadline};

use super::cutoff::Cutoff;
use super::deadline::{MonotonicClock, statement_timeout_ms};
use super::settings::CleanupBounds;
use super::table::{CleanupTable, delete_statement, remaining_statement};
use super::{BatchProgress, StopReason, TableCleanupResult};

mod tests;

const BATCH_PAUSE: Duration = Duration::from_millis(100);
const REMAINING_COLUMN: &str = "remaining";

pub struct BatchDeleter<'run, Clock, Progress> {
    pub client: &'run mut Client,
    pub bounds: CleanupBounds,
    pub clock: &'run Clock,
    pub metrics: &'run CleanupMetrics<'run>,
    pub progress: &'run mut Progress,
}

struct TableRun {
    table: CleanupTable,
    cutoff: Cutoff,
    deadline: Instant,
    statement: String,
    deleted: u64,
    batches: i64,
    failures: i64,
}

impl<Clock, Progress> BatchDeleter<'_, Clock, Progress>
where
    Clock: MonotonicClock,
    Progress: BatchProgress,
{
    // _delete_old_rows_batched: the backlog running out, the deadline, the batch cap or repeated failures end a table
    pub async fn delete_expired(
        &mut self,
        table: CleanupTable,
        cutoff: Cutoff,
        deadline: Instant,
    ) -> TableCleanupResult {
        let mut run = TableRun {
            table,
            cutoff,
            deadline,
            statement: delete_statement(table, cutoff),
            deleted: 0,
            batches: 0,
            failures: 0,
        };
        let stop_reason = loop {
            if let Some(reason) = self.next_batch(&mut run).await {
                break reason;
            }
        };
        self.finish_table(&run, stop_reason).await
    }

    // the deadline is checked before the batch cap, so a spent run reports budget_exhausted
    async fn next_batch(&mut self, run: &mut TableRun) -> Option<StopReason> {
        let name = run.table.spec().name;
        if self.clock.now() >= run.deadline {
            tracing::info!(
                "Run budget exhausted during {name} cleanup after {} rows; the next run resumes from here",
                run.deleted
            );
            return Some(StopReason::BudgetExhausted);
        }
        if run.batches >= self.bounds.max_batches {
            tracing::info!(
                "Max batches reached for {name} cleanup, remaining rows will be deleted in next run"
            );
            return Some(StopReason::BatchCapReached);
        }
        let started = self.clock.now();
        let timeout_ms = statement_timeout_ms(started, run.deadline, self.bounds.batch_timeout);
        let batch = delete_batch(self.client, run, self.bounds.batch_size, timeout_ms);
        match within_deadline(timeout_ms, batch).await {
            Ok(deleted) => self.batch_deleted(run, deleted, started).await,
            Err(error) => {
                let stop_reason = self.batch_failed(run, &error);
                if stop_reason.is_none() {
                    sleep(self.bounds.failure_backoff).await;
                }
                stop_reason
            }
        }
    }

    async fn batch_deleted(
        &mut self,
        run: &mut TableRun,
        deleted: u64,
        started: Instant,
    ) -> Option<StopReason> {
        let name = run.table.spec().name;
        run.failures = 0;
        let elapsed = self.clock.now().saturating_duration_since(started);
        self.metrics
            .record_batch(name, deleted, elapsed.as_secs_f64());
        tracing::info!("Deleted {deleted} {name} rows in this batch");
        if deleted == 0 {
            tracing::info!(
                "No more {name} rows to delete. Total deleted: {}",
                run.deleted
            );
            return Some(StopReason::Exhausted);
        }
        run.deleted = run.deleted.saturating_add(deleted);
        run.batches = run.batches.saturating_add(1);
        self.progress.record_batch(deleted);
        sleep(BATCH_PAUSE).await;
        None
    }

    // a batch the clamped timeout cancelled at the deadline is the bound working, not a database fault
    fn batch_failed(
        &self,
        run: &mut TableRun,
        error: &BoundedTransactionError,
    ) -> Option<StopReason> {
        let name = run.table.spec().name;
        if self.clock.now() >= run.deadline {
            tracing::info!(
                "Run budget exhausted mid-batch during {name} cleanup after {} rows; the next run resumes from here",
                run.deleted
            );
            return Some(StopReason::BudgetExhausted);
        }
        run.failures = run.failures.saturating_add(1);
        self.metrics.record_batch_failure(name);
        tracing::error!(
            "{name} cleanup batch failed (run_count={}, consecutive_failures={}, batch_size={}, cutoff={}, total_deleted_so_far={}): {error}",
            run.batches,
            run.failures,
            self.bounds.batch_size,
            run.cutoff.text(),
            run.deleted
        );
        if run.failures >= self.bounds.max_consecutive_failures {
            tracing::error!(
                "Aborting {name} cleanup after {} consecutive batch failures; total deleted before abort: {}",
                run.failures,
                run.deleted
            );
            return Some(StopReason::Aborted);
        }
        None
    }

    // _finish_table: no probe is issued once the budget is spent, and a failed probe keeps the last gauge value
    async fn finish_table(
        &mut self,
        run: &TableRun,
        stop_reason: StopReason,
    ) -> TableCleanupResult {
        let result = TableCleanupResult {
            table: run.table,
            rows_deleted: run.deleted,
            stop_reason,
        };
        let now = self.clock.now();
        if now >= run.deadline {
            return result;
        }
        let timeout_ms = statement_timeout_ms(now, run.deadline, self.bounds.batch_timeout);
        let probe = count_remaining(
            self.client,
            run,
            self.bounds.remaining_count_cap,
            timeout_ms,
        );
        match within_deadline(timeout_ms, probe).await {
            Ok(remaining) => self
                .metrics
                .set_rows_remaining(run.table.spec().name, remaining),
            Err(error) => {
                tracing::warn!(
                    "Could not count remaining {} rows: {error}",
                    run.table.spec().name
                );
            }
        }
        result
    }
}

// _execute_delete_batch: SET LOCAL scopes both timeouts to this transaction, so the pooled connection keeps its own
async fn delete_batch(
    client: &mut Client,
    run: &TableRun,
    batch_size: i64,
    timeout_ms: u64,
) -> Result<u64, PostgresError> {
    let transaction = client.transaction().await?;
    let timeouts = format!(
        "SET LOCAL statement_timeout = {timeout_ms}; SET LOCAL lock_timeout = {timeout_ms}"
    );
    transaction.batch_execute(&timeouts).await?;
    let deleted = transaction
        .execute(&run.statement, &[&run.cutoff, &batch_size])
        .await?;
    transaction.commit().await?;
    Ok(deleted)
}

// _count_remaining: the probe reads at most the cap's worth of expired rows under its own statement timeout
async fn count_remaining(
    client: &mut Client,
    run: &TableRun,
    cap: i64,
    timeout_ms: u64,
) -> Result<i64, PostgresError> {
    let transaction = client.transaction().await?;
    let statement_timeout = format!("SET LOCAL statement_timeout = {timeout_ms}");
    transaction.batch_execute(&statement_timeout).await?;
    let statement = remaining_statement(run.table, run.cutoff);
    let row = transaction
        .query_one(&statement, &[&run.cutoff, &cap])
        .await?;
    let remaining = row.try_get::<_, i32>(REMAINING_COLUMN)?;
    transaction.commit().await?;
    Ok(i64::from(remaining))
}
