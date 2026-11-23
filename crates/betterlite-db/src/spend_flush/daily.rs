use std::error::Error as StdError;
use std::sync::Arc;

use tokio::task::{JoinError, JoinHandle};
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use crate::daily_spend_sql::entities::DailySpendEntity;
use crate::key_user_writes::retry::WriteClock;
use crate::migration_ledger::lock::JitterSource;
use crate::spend_commit::daily::{DailyChunkCommit, DailyChunkWriter};
use crate::spend_commit::phase::{CommitPhase, CommitTransition, start_daily_spend_commit};
use crate::spend_commit::policy::SpendRetryPolicy;
use crate::spend_log_storage::rows::writer::SpendLogFailureSink;
use crate::spend_queues::SpendQueueError;
use crate::spend_queues::daily::{DailySpendUpdate, DailySpendUpdateQueue};
use crate::tool_spend::RetryPause;

// Everything one daily commit reads besides the queue and the drained map, owned so the task a flush starts outlives the flush's own stack frame.
pub struct DailyCommitParts<Writer> {
    pub writer: Arc<Writer>,
    pub retries: u32,
    pub clock: Arc<dyn WriteClock>,
    pub jitter: Arc<dyn JitterSource>,
    pub pause: Arc<dyn RetryPause>,
    pub failures: Arc<dyn SpendLogFailureSink>,
}

type CommitResult<Writer> = (
    DailySpendUpdate,
    Result<(), <Writer as DailyChunkWriter>::Error>,
);

type CommitSettlement<Writer> = Result<CommitResult<Writer>, JoinError>;

#[derive(Debug, thiserror::Error)]
pub enum DailyFlushError {
    #[error("the in-memory daily {entity:?} spend queue could not be drained")]
    Queue {
        entity: DailySpendEntity,
        #[source]
        source: SpendQueueError,
    },
    #[error("the daily {entity:?} spend commit was interrupted by shutdown")]
    Cancelled { entity: DailySpendEntity },
}

// _flush_daily_spend_queue: the drained map belongs to a commit task of its own, the flush waits it out, and a shutdown hands the task to a tracked continuation.
pub async fn flush_daily_spend_queue<Writer>(
    queue: &Arc<DailySpendUpdateQueue>,
    entity: DailySpendEntity,
    parts: &Arc<DailyCommitParts<Writer>>,
    commits: &TaskTracker,
    shutdown: &CancellationToken,
) -> Result<(), DailyFlushError>
where
    Writer: DailyChunkWriter + 'static,
{
    let pending = queue
        .flush_aggregated_daily_rows()
        .await
        .map_err(|source| DailyFlushError::Queue { entity, source })?;
    if pending.is_empty() {
        return Ok(());
    }
    let ports = Arc::clone(parts);
    let started = start_daily_spend_commit(commits, move |phase| async move {
        let mut pending = pending;
        let outcome = commit_daily_map(&ports, entity, &mut pending, phase).await;
        (pending, outcome)
    });
    let phase = started.phase;
    let mut task = started.task;
    let joined = tokio::select! {
        outcome = &mut task => Some(outcome),
        () = shutdown.cancelled() => None,
    };
    match joined {
        Some(settled) => settle_daily_commit(queue, entity, settled).await,
        None => interrupt_daily_commit(queue, entity, phase, task).await,
    }
}

async fn commit_daily_map<Writer>(
    parts: &DailyCommitParts<Writer>,
    entity: DailySpendEntity,
    pending: &mut DailySpendUpdate,
    phase: Arc<CommitPhase>,
) -> Result<(), Writer::Error>
where
    Writer: DailyChunkWriter,
{
    let policy = SpendRetryPolicy {
        retries: parts.retries,
        clock: &*parts.clock,
        jitter: &*parts.jitter,
        pause: &*parts.pause,
        failures: &*parts.failures,
    };
    let commit = DailyChunkCommit {
        writer: &*parts.writer,
        policy,
        phase,
    };
    commit.update_daily_spend(entity, pending).await
}

// The flush was not interrupted: a failed commit re-queues what its own chunks left behind and the next queue still runs.
async fn settle_daily_commit<Writer>(
    queue: &DailySpendUpdateQueue,
    entity: DailySpendEntity,
    settled: CommitSettlement<Writer>,
) -> Result<(), DailyFlushError>
where
    Writer: DailyChunkWriter,
{
    let joined = match settled {
        Ok(joined) => joined,
        Err(join_error) => {
            tracing::error!(
                "Spend tracking - the daily {entity:?} spend commit task ended abnormally: {join_error}"
            );
            return Ok(());
        }
    };
    let (remaining, outcome) = joined;
    let Err(error) = outcome else {
        return Ok(());
    };
    requeue_daily_rows(queue, entity, remaining, &error).await;
    Ok(())
}

// The shutdown arrived mid-flush. Before the phase commits, nothing reached the database and the whole remaining map goes back; once it is committing, the queue owns a continuation instead.
async fn interrupt_daily_commit<Writer>(
    queue: &Arc<DailySpendUpdateQueue>,
    entity: DailySpendEntity,
    phase: Arc<CommitPhase>,
    task: JoinHandle<CommitResult<Writer>>,
) -> Result<(), DailyFlushError>
where
    Writer: DailyChunkWriter + 'static,
{
    if phase.apply(CommitTransition::Cancel).is_err() {
        let settling = Arc::clone(queue);
        queue.track_interrupted_commit(async move {
            let settled = task.await;
            requeue_interrupted_daily(&settling, entity, settled).await;
        });
        return Err(DailyFlushError::Cancelled { entity });
    }
    match task.await {
        Ok((remaining, _stopped)) => restore_rows(queue, entity, remaining).await,
        Err(join_error) => {
            tracing::error!(
                "Spend tracking - the cancelled daily {entity:?} spend commit task ended abnormally: {join_error}"
            );
        }
    }
    Err(DailyFlushError::Cancelled { entity })
}

// _requeue_daily_spend_the_commit_left_behind: the interrupted commit is awaited, and only a failed one puts its remaining rows back.
async fn requeue_interrupted_daily<Writer>(
    queue: &DailySpendUpdateQueue,
    entity: DailySpendEntity,
    settled: CommitSettlement<Writer>,
) where
    Writer: DailyChunkWriter,
{
    let (remaining, outcome) = match settled {
        Ok(joined) => joined,
        Err(join_error) => {
            tracing::error!(
                "Spend tracking - the interrupted daily {entity:?} spend commit task ended abnormally: {join_error}"
            );
            return;
        }
    };
    let Err(error) = outcome else {
        return;
    };
    if remaining.is_empty() {
        return;
    }
    let rows = remaining.len();
    tracing::error!(
        "Spend tracking - daily {entity:?} spend commit interrupted by shutdown failed. Re-queued {rows} rows for the shutdown flush. Error: {error}"
    );
    restore_rows(queue, entity, remaining).await;
}

async fn requeue_daily_rows<Failure>(
    queue: &DailySpendUpdateQueue,
    entity: DailySpendEntity,
    remaining: DailySpendUpdate,
    error: &Failure,
) where
    Failure: StdError,
{
    if remaining.is_empty() {
        return;
    }
    let rows = remaining.len();
    tracing::error!(
        "Spend tracking - failed to commit daily {entity:?} spend updates. Re-queued {rows} rows for retry on next tick. Error: {error}"
    );
    restore_rows(queue, entity, remaining).await;
}

async fn restore_rows(
    queue: &DailySpendUpdateQueue,
    entity: DailySpendEntity,
    remaining: DailySpendUpdate,
) {
    if remaining.is_empty() {
        return;
    }
    if let Err(error) = queue.add_update(remaining).await {
        tracing::error!(
            "Spend tracking - putting the daily {entity:?} spend rows back on the queue failed: {error}"
        );
    }
}
