use std::error::Error as StdError;
use std::sync::Arc;

use tokio::task::{JoinError, JoinHandle};
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use crate::daily_spend_sql::entities::DailySpendEntity;
use crate::redis_spend::buffer::{RedisSpendError, RedisUpdateBuffer};
use crate::redis_spend::publication::RedisSpendPublisher;
use crate::redis_spend::republication::{
    RepublicationError, UncommittedCohorts, restore_transactions_to_redis,
};
use crate::redis_spend::tag_publication::{TagPublishError, store_daily_tag_spend_updates};
use crate::spend_commit::daily::{DailyChunkFailure as _, DailyChunkWriter};
use crate::spend_commit::phase::{CommitPhase, CommitTransition, start_daily_spend_commit};
use crate::spend_flush::daily::{DailyCommitParts, DailyFlushError, flush_daily_spend_queue};
use crate::spend_flush::service::ServiceFuture;
use crate::spend_queues::daily::{DailySpendUpdate, DailySpendUpdateQueue};

use super::DAILY_TAG_SPEND_JOB_NAME;
use super::commit_daily;
use super::lease::SpendPodLease;
use super::parts_with_retries;

mod tests;

type CommitResult<Writer> = (
    DailySpendUpdate,
    Result<(), <Writer as DailyChunkWriter>::Error>,
);

#[derive(Debug, thiserror::Error)]
pub enum TagFlushError<Failure>
where
    Failure: StdError + Send + Sync + 'static,
{
    #[error(transparent)]
    Publish(#[from] TagPublishError),
    #[error(transparent)]
    Drain(#[from] RedisSpendError),
    #[error("the daily tag spend commit failed")]
    Commit(#[source] Failure),
    #[error("the daily tag cohort could not be restored to redis")]
    Restore(#[from] RepublicationError),
    #[error("the daily tag spend commit was interrupted by shutdown")]
    Cancelled,
}

// Whether a tag commit failure is the shutdown's cancellation rather than an ordinary loss the job adapter logs.
pub trait TagCommitFailure: StdError + Send + Sync + 'static {
    fn is_cancelled(&self) -> bool;
}

impl<Failure> TagCommitFailure for TagFlushError<Failure>
where
    Failure: StdError + Send + Sync + 'static,
{
    fn is_cancelled(&self) -> bool {
        matches!(self, Self::Cancelled)
    }
}

impl TagCommitFailure for DailyFlushError {
    fn is_cancelled(&self) -> bool {
        matches!(self, Self::Cancelled { .. })
    }
}

// The tag adapter's one branch: a Redis flusher or the in-memory tag queue commit, both committed with the adapter's retry count.
pub trait DailyTagCommit: Send + Sync {
    type Error: TagCommitFailure;

    fn commit_daily_tag_spend(&self, retries: u32) -> ServiceFuture<'_, Result<(), Self::Error>>;
}

// _commit_daily_tag_spend_to_db: the memory tag queue behind B1747's flush, with the adapter's retry count in its parts.
pub struct MemoryTagFlush<'run, Writer> {
    pub queue: &'run Arc<DailySpendUpdateQueue>,
    pub parts: &'run Arc<DailyCommitParts<Writer>>,
    pub commits: &'run TaskTracker,
    pub shutdown: &'run CancellationToken,
}

impl<Writer> DailyTagCommit for MemoryTagFlush<'_, Writer>
where
    Writer: DailyChunkWriter + 'static,
{
    type Error = DailyFlushError;

    fn commit_daily_tag_spend(
        &self,
        retries: u32,
    ) -> ServiceFuture<'_, Result<(), DailyFlushError>> {
        Box::pin(async move {
            let parts = parts_with_retries(self.parts, retries);
            flush_daily_spend_queue(
                self.queue,
                DailySpendEntity::Tag,
                &parts,
                self.commits,
                self.shutdown,
            )
            .await
        })
    }
}

// _commit_daily_tag_spend_to_db_with_redis: the tag queue's own publication, its own pod lease, and a commit whose interruption never hides its outcome.
pub struct RedisTagFlush<'run, Writer> {
    pub publisher: Arc<RedisSpendPublisher>,
    pub buffer: &'run RedisUpdateBuffer,
    pub lease: &'run dyn SpendPodLease,
    pub queue: &'run Arc<DailySpendUpdateQueue>,
    pub parts: &'run Arc<DailyCommitParts<Writer>>,
    pub commits: &'run TaskTracker,
    pub shutdown: &'run CancellationToken,
}

impl<Writer> RedisTagFlush<'_, Writer>
where
    Writer: DailyChunkWriter + 'static,
{
    pub async fn commit_tag_from_redis(
        &self,
        retries: u32,
    ) -> Result<(), TagFlushError<Writer::Error>> {
        store_daily_tag_spend_updates(&self.publisher, self.queue).await?;
        let Some(acquired) = self.lease.acquire(DAILY_TAG_SPEND_JOB_NAME).await else {
            return Ok(());
        };
        if !acquired {
            return Ok(());
        }
        match self.drain_and_commit(retries).await {
            Ok(()) => {}
            Err(error) if error.is_cancelled() => {
                self.lease.release(DAILY_TAG_SPEND_JOB_NAME).await;
                return Err(error);
            }
            Err(error) => tracing::error!(
                "Spend tracking - failed to commit daily tag spend updates from Redis to DB. Re-queuing to Redis for retry on next tick. Error: {error}"
            ),
        }
        self.lease.release(DAILY_TAG_SPEND_JOB_NAME).await;
        Ok(())
    }

    // _drain_and_commit_daily_tag_spend_from_redis: earlier settlements end first, the pop is destructive, and an interrupted commit resolves before any restoration.
    pub async fn drain_and_commit(&self, retries: u32) -> Result<(), TagFlushError<Writer::Error>> {
        self.queue.settle_interrupted_commits().await;
        let Some(drained) = self.buffer.drain_daily_tags().await? else {
            return Ok(());
        };
        if drained.is_empty() {
            return Ok(());
        }
        let parts = parts_with_retries(self.parts, retries);
        let started = start_daily_spend_commit(self.commits, move |phase| async move {
            let mut pending = drained;
            let outcome =
                commit_daily(&parts, retries, phase, DailySpendEntity::Tag, &mut pending).await;
            (pending, outcome)
        });
        let phase = started.phase;
        let mut task = started.task;
        match self.shutdown.run_until_cancelled(&mut task).await {
            Some(settled) => self.settle_tag_commit(settled).await,
            None => self.interrupt_tag_commit(phase, task).await,
        }
    }

    // The commit reached its own end: only a commit that failed with an ordinary error puts its remaining rows back.
    async fn settle_tag_commit(
        &self,
        settled: Result<CommitResult<Writer>, JoinError>,
    ) -> Result<(), TagFlushError<Writer::Error>> {
        let (remaining, outcome) = match settled {
            Ok(joined) => joined,
            Err(join_error) => {
                tracing::error!(
                    "Spend tracking - the daily tag spend commit task ended abnormally: {join_error}"
                );
                return Ok(());
            }
        };
        let Err(error) = outcome else {
            return Ok(());
        };
        if error.is_cancelled() {
            return Err(TagFlushError::Cancelled);
        }
        restore_tag_cohort(&self.publisher, remaining)
            .await
            .map_err(TagFlushError::Restore)?;
        Err(TagFlushError::Commit(error))
    }

    // The shutdown arrived: a cancel that wins the phase restores what never reached the commit, while a commit already under way is tracked to its definitive result.
    async fn interrupt_tag_commit(
        &self,
        phase: Arc<CommitPhase>,
        task: JoinHandle<CommitResult<Writer>>,
    ) -> Result<(), TagFlushError<Writer::Error>> {
        if phase.apply(CommitTransition::Cancel).is_err() {
            let publisher = Arc::clone(&self.publisher);
            self.queue.track_interrupted_commit(async move {
                let settled = task.await;
                restore_interrupted_tag::<Writer>(&publisher, settled).await;
            });
            return Err(TagFlushError::Cancelled);
        }
        match task.await {
            Ok((remaining, _outcome)) => restore_tag_cohort(&self.publisher, remaining)
                .await
                .map_err(TagFlushError::Restore)?,
            Err(join_error) => tracing::error!(
                "Spend tracking - the cancelled daily tag spend commit task ended abnormally: {join_error}"
            ),
        }
        Err(TagFlushError::Cancelled)
    }
}

impl<Writer> DailyTagCommit for RedisTagFlush<'_, Writer>
where
    Writer: DailyChunkWriter + 'static,
{
    type Error = TagFlushError<Writer::Error>;

    fn commit_daily_tag_spend(&self, retries: u32) -> ServiceFuture<'_, Result<(), Self::Error>> {
        Box::pin(self.commit_tag_from_redis(retries))
    }
}

// _restore_tag_spend_the_commit_left_behind: the tracked commit is awaited first, and only its noncancelled failure restores its remaining rows.
async fn restore_interrupted_tag<Writer>(
    publisher: &RedisSpendPublisher,
    settled: Result<CommitResult<Writer>, JoinError>,
) where
    Writer: DailyChunkWriter,
{
    let (remaining, outcome) = match settled {
        Ok(joined) => joined,
        Err(join_error) => {
            tracing::error!(
                "Spend tracking - the interrupted daily tag spend commit task ended abnormally: {join_error}"
            );
            return;
        }
    };
    match outcome {
        Ok(()) => {}
        Err(error) if error.is_cancelled() => {}
        Err(error) => {
            tracing::error!(
                "Spend tracking - the interrupted daily tag spend commit failed. Re-queuing {} rows to Redis. Error: {error}",
                remaining.len()
            );
            if let Err(restore_error) = restore_tag_cohort(publisher, remaining).await {
                tracing::error!(
                    "Spend tracking - restoring the interrupted daily tag cohort to Redis failed: {restore_error}"
                );
            }
        }
    }
}

async fn restore_tag_cohort(
    publisher: &RedisSpendPublisher,
    cohort: DailySpendUpdate,
) -> Result<(), RepublicationError> {
    let cohorts = UncommittedCohorts {
        entity: None,
        daily_user: None,
        daily_team: None,
        daily_org: None,
        daily_end_user: None,
        daily_agent: None,
        daily_tag: Some(cohort),
        window: None,
    };
    restore_transactions_to_redis(publisher, cohorts).await
}
