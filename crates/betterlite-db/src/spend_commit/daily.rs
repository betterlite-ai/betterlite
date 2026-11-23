pub mod order;
pub mod postgres;

mod tests;

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Instant;

use crate::daily_spend_sql::entities::{DailySpendEntity, SpendRow};
use crate::spend_queues::daily::DailySpendUpdate;

use self::order::ordered_chunk_keys;
use super::phase::CommitPhase;
use super::policy::{
    SpendRetry, SpendRetryPolicy, SpendUpdateFailure, report_failed_update_spend, retry_delay,
};

// BATCH_SIZE of _update_daily_spend: one statement carries at most a hundred input entries
pub const DAILY_BATCH_SIZE: usize = 100;

pub trait DailyChunkFailure: SpendUpdateFailure {
    fn is_cancelled(&self) -> bool {
        false
    }
}

pub type ChunkFuture<'writer, Failure> =
    Pin<Box<dyn Future<Output = Result<(), Failure>> + Send + 'writer>>;

// _spend_update_tx plus the commit marks: one merged bulk upsert inside one sixty second transaction, with the phase marking the commit.
pub trait DailyChunkWriter: Send + Sync {
    type Error: DailyChunkFailure;

    fn write_chunk<'writer>(
        &'writer self,
        entity: DailySpendEntity,
        rows: &'writer [SpendRow],
        phase: &'writer CommitPhase,
    ) -> ChunkFuture<'writer, Self::Error>;
}

pub struct DailyChunkCommit<'run, Writer> {
    pub writer: &'run Writer,
    pub policy: SpendRetryPolicy<'run>,
    pub phase: Arc<CommitPhase>,
}

impl<Writer> DailyChunkCommit<'_, Writer>
where
    Writer: DailyChunkWriter,
{
    // _update_daily_spend for one entity: sorted chunks leave the pending map only when the statement applied or can never be replayed, and the wrapper reports every failure once that is not a cancellation.
    pub async fn update_daily_spend(
        &self,
        entity: DailySpendEntity,
        pending: &mut DailySpendUpdate,
    ) -> Result<(), Writer::Error> {
        let started = self.policy.clock.instant();
        let outcome = self.commit_chunks(entity, pending, started).await;
        match &outcome {
            Ok(()) => {}
            Err(error) if error.is_cancelled() => {}
            Err(error) => {
                let elapsed = self
                    .policy
                    .clock
                    .instant()
                    .saturating_duration_since(started);
                report_failed_update_spend(self.policy.failures, error, elapsed);
            }
        }
        outcome
    }

    async fn commit_chunks(
        &self,
        entity: DailySpendEntity,
        pending: &mut DailySpendUpdate,
        started: Instant,
    ) -> Result<(), Writer::Error> {
        while !pending.is_empty() {
            let mut attempt = 0_u32;
            loop {
                match self.attempt_chunk(entity, pending).await {
                    Ok(()) => break,
                    Err(error) if error.is_cancelled() => return Err(error),
                    Err(error) => match self.retry_after_failure(&error, attempt, started).await {
                        SpendRetry::Retry => attempt = attempt.saturating_add(1),
                        SpendRetry::Stop => return Err(error),
                    },
                }
            }
        }
        Ok(())
    }

    // The daily half of _handle_spend_update_failure: a non-retryable failure reaches the outer report untouched, and an exhausted retryable one is reported here and again by that report, as the source does.
    async fn retry_after_failure<Failure>(
        &self,
        error: &Failure,
        attempt: u32,
        started: Instant,
    ) -> SpendRetry
    where
        Failure: DailyChunkFailure,
    {
        let failure = error.spend_failure();
        if !failure.is_retryable() {
            return SpendRetry::Stop;
        }
        if attempt >= self.policy.retries {
            let elapsed = self
                .policy
                .clock
                .instant()
                .saturating_duration_since(started);
            report_failed_update_spend(self.policy.failures, error, elapsed);
            return SpendRetry::Stop;
        }
        tracing::warn!(
            attempt = attempt.saturating_add(1),
            retries = self.policy.retries,
            error = %error,
            "Retrying spend update after retryable DB error"
        );
        self.policy
            .pause
            .pause(retry_delay(self.policy.jitter, attempt))
            .await;
        SpendRetry::Retry
    }

    async fn attempt_chunk(
        &self,
        entity: DailySpendEntity,
        pending: &mut DailySpendUpdate,
    ) -> Result<(), Writer::Error> {
        let keys = ordered_chunk_keys(entity, pending);
        let mut rows: Vec<SpendRow> = Vec::with_capacity(keys.len());
        for key in &keys {
            if let Some(row) = pending.get(key) {
                rows.push(row.clone());
            }
        }
        match self.writer.write_chunk(entity, &rows, &self.phase).await {
            Ok(()) => {}
            Err(error) => {
                record_chunk_failure(entity, &keys, pending, &error);
                return Err(error);
            }
        }
        for key in &keys {
            pending.remove(key);
        }
        Ok(())
    }
}

// The two batch errors of the source: a retry-safe failure keeps its chunk, an unsafe one loses it before the error escapes.
fn record_chunk_failure<Failure>(
    entity: DailySpendEntity,
    keys: &[String],
    pending: &mut DailySpendUpdate,
    error: &Failure,
) where
    Failure: DailyChunkFailure,
{
    if error.spend_failure().is_requeue_safe() {
        tracing::error!(
            entity = ?entity,
            rows = keys.len(),
            error = %error,
            "Daily spend batch upsert failed"
        );
        return;
    }
    for key in keys {
        pending.remove(key);
    }
    tracing::error!(
        entity = ?entity,
        rows = keys.len(),
        error = %error,
        "Spend tracking - dropped daily spend rows: the failed statement may have applied or the database refused the data, so re-sending it is not safe"
    );
}
