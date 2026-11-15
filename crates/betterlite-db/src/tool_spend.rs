use std::error::Error as StdError;
use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use betterlite_models::tool_spend::ToolUsageTransaction;

use crate::migration_ledger::lock::JitterSource;
use crate::spend_batches::{StatementLimits, spend_log_write_batches};
use index_rows::ToolIndexRow;
use rollups::ToolDayRollup;

pub mod index_rows;
pub mod rollups;
pub mod writer;

mod tests;

// LiteLLM's n_retry_times default: one flush tries four times before it raises
pub const DEFAULT_RETRIES: u32 = 3;

const JITTER_NANOS: u64 = 1_000_000_000;

#[derive(Debug, Clone, Copy)]
pub struct FlushPolicy {
    pub limits: StatementLimits,
    pub retries: u32,
}

pub trait ToolSpendFailure: StdError + Send + Sync + 'static {
    fn is_pre_send(&self) -> bool;
}

pub type FlushFuture<'writer, Failure> =
    Pin<Box<dyn Future<Output = Result<(), Failure>> + Send + 'writer>>;

// Index rows go out as bounded standalone statements; every rollup increment shares one transaction
pub trait ToolSpendWriter: Send + Sync {
    type Error: ToolSpendFailure;

    fn write_index_rows<'writer>(
        &'writer self,
        rows: &'writer [ToolIndexRow],
    ) -> FlushFuture<'writer, Self::Error>;

    fn apply_rollups<'writer>(
        &'writer self,
        rollups: &'writer [ToolDayRollup],
    ) -> FlushFuture<'writer, Self::Error>;
}

pub trait RetryPause: Send + Sync {
    fn pause(&self, delay: Duration) -> Pin<Box<dyn Future<Output = ()> + Send + '_>>;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct TokioPause;

impl RetryPause for TokioPause {
    fn pause(&self, delay: Duration) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        Box::pin(tokio::time::sleep(delay))
    }
}

pub async fn flush_tool_usage_transactions<W>(
    writer: &W,
    transactions: &[ToolUsageTransaction],
    policy: FlushPolicy,
    jitter: &dyn JitterSource,
    pause: &dyn RetryPause,
) -> Result<(), W::Error>
where
    W: ToolSpendWriter,
{
    if transactions.is_empty() {
        return Ok(());
    }
    let index_rows = build_index_rows(transactions);
    let rollups = rollups::daily_rollups(transactions);
    let mut attempt = 0;
    loop {
        match attempt_flush(writer, &index_rows, &rollups, policy.limits).await {
            Ok(()) => return Ok(()),
            Err(error) if !error.is_pre_send() || attempt >= policy.retries => {
                return Err(error);
            }
            Err(_) => {
                pause.pause(retry_delay(jitter, attempt)).await;
                attempt = attempt.saturating_add(1);
            }
        }
    }
}

async fn attempt_flush<W>(
    writer: &W,
    index_rows: &[ToolIndexRow],
    rollups: &[ToolDayRollup],
    limits: StatementLimits,
) -> Result<(), W::Error>
where
    W: ToolSpendWriter,
{
    for rows in spend_log_write_batches(index_rows, limits) {
        writer.write_index_rows(rows).await?;
    }
    writer.apply_rollups(rollups).await
}

fn build_index_rows(transactions: &[ToolUsageTransaction]) -> Vec<ToolIndexRow> {
    let mut rows = Vec::new();
    for transaction in transactions {
        for tool_name in &transaction.tool_names {
            rows.push(ToolIndexRow::new(
                transaction.request_id.clone(),
                tool_name.clone(),
                transaction.start_time,
            ));
        }
    }
    rows
}

// Python sleeps 2**attempt plus random.uniform(0, 1) seconds between attempts
fn retry_delay(jitter: &dyn JitterSource, attempt: u32) -> Duration {
    let seconds = 1_u64.checked_shl(attempt).unwrap_or(u64::MAX);
    Duration::from_secs(seconds)
        .saturating_add(Duration::from_nanos(jitter.uniform_nanos(JITTER_NANOS)))
}
