use std::error::Error as StdError;

use crate::spend_flush::service::SpendLogsService;

use super::leader::dispatch::{SpendFlushPass, SpendUpdateError, SpendUpdateTransactionHandler};
use super::tags::{DailyTagCommit, TagCommitFailure};

mod tests;

// update_spend's pinned n_retry_times and update_daily_tag_spend's, both Final = 3.
pub const SPEND_JOB_RETRIES: u32 = 3;

#[derive(Debug, thiserror::Error)]
pub enum UpdateSpendJobError<BalanceFailure, LogsFailure>
where
    BalanceFailure: StdError + Send + Sync + 'static,
    LogsFailure: StdError + Send + Sync + 'static,
{
    #[error("the spend balance dispatch failed")]
    Balance(#[source] BalanceFailure),
    #[error("the spend logs job failed")]
    Logs(#[source] LogsFailure),
}

// update_spend: the balance dispatch first, then the parked-log recovery, and the spend-logs job only when the combined census is nonzero.
pub struct SpendUpdateJob<'run, Redis, Memory, Logs> {
    pub balance: SpendUpdateTransactionHandler<'run, Redis, Memory>,
    pub logs: &'run Logs,
}

impl<Redis, Memory, Logs> SpendUpdateJob<'_, Redis, Memory, Logs>
where
    Redis: SpendFlushPass,
    Memory: SpendFlushPass,
    Logs: SpendLogsService,
{
    pub async fn update_spend(
        &self,
    ) -> Result<(), UpdateSpendJobError<SpendUpdateError<Redis::Error, Memory::Error>, Logs::Error>>
    {
        self.balance
            .commit_spend_updates(SPEND_JOB_RETRIES)
            .await
            .map_err(UpdateSpendJobError::Balance)?;
        self.logs.recover_parked_logs().await;
        let queue_size = self.logs.queued_transactions();
        tracing::debug!(queued = queue_size, "Spend Logs transactions");
        if queue_size > 0 {
            self.logs
                .update_spend_logs()
                .await
                .map_err(UpdateSpendJobError::Logs)?;
        }
        Ok(())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum DailyTagSpendError<RedisFailure, MemoryFailure>
where
    RedisFailure: TagCommitFailure,
    MemoryFailure: TagCommitFailure,
{
    #[error("the redis daily tag spend commit was interrupted by shutdown")]
    Redis(#[source] RedisFailure),
    #[error("the in-memory daily tag spend commit was interrupted by shutdown")]
    Memory(#[source] MemoryFailure),
}

// The only error the adapter lets out is the shutdown's cancellation, so a scheduler reads the branch's flag.
impl<RedisFailure, MemoryFailure> TagCommitFailure
    for DailyTagSpendError<RedisFailure, MemoryFailure>
where
    RedisFailure: TagCommitFailure,
    MemoryFailure: TagCommitFailure,
{
    fn is_cancelled(&self) -> bool {
        match self {
            Self::Redis(error) => error.is_cancelled(),
            Self::Memory(error) => error.is_cancelled(),
        }
    }
}

// update_daily_tag_spend: the loaded Redis policy picks the tag flusher, and only the shutdown's cancellation escapes the plain error log.
pub enum DailyTagSpendFlush<'run, Redis, Memory> {
    Redis(&'run Redis),
    Memory(&'run Memory),
}

impl<'run, Redis, Memory> DailyTagSpendFlush<'run, Redis, Memory>
where
    Redis: DailyTagCommit,
    Memory: DailyTagCommit,
{
    #[must_use]
    pub const fn select(
        use_redis_transaction_buffer: bool,
        redis: &'run Redis,
        memory: &'run Memory,
    ) -> Self {
        if use_redis_transaction_buffer {
            Self::Redis(redis)
        } else {
            Self::Memory(memory)
        }
    }

    pub async fn update_daily_tag_spend(
        &self,
    ) -> Result<(), DailyTagSpendError<Redis::Error, Memory::Error>> {
        match self {
            Self::Redis(redis) => {
                cancel_or_log(redis.commit_daily_tag_spend(SPEND_JOB_RETRIES).await)
                    .map_or(Ok(()), |error| Err(DailyTagSpendError::Redis(error)))
            }
            Self::Memory(memory) => {
                cancel_or_log(memory.commit_daily_tag_spend(SPEND_JOB_RETRIES).await)
                    .map_or(Ok(()), |error| Err(DailyTagSpendError::Memory(error)))
            }
        }
    }
}

// The adapter's own error log: spend_log_error would attach the active traceback, and the historical site never did.
fn cancel_or_log<Failure>(outcome: Result<(), Failure>) -> Option<Failure>
where
    Failure: TagCommitFailure,
{
    match outcome {
        Ok(()) => None,
        Err(error) if error.is_cancelled() => Some(error),
        Err(error) => {
            tracing::error!("Error updating daily tag spend: {error}");
            None
        }
    }
}
