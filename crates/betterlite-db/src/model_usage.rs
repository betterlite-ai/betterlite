pub mod catalog;
pub mod decode;
pub mod rollup;
pub mod writer;

mod tests;

use std::error::Error as StdError;
use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use chrono::{DateTime, FixedOffset};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::migration_ledger::lock::JitterSource;
use crate::spend_queues::entity::SpendAmount;
use crate::tool_spend::RetryPause;

// LiteLLM's n_retry_times default: one flush tries four times before it raises
pub const DEFAULT_RETRY_TIMES: u32 = 3;

const RETRY_JITTER_NANOS: u64 = 1_000_000_000;

#[derive(Debug, thiserror::Error)]
pub enum ModelUsageError {
    #[error("summing model usage spend overflows the decimal range")]
    SpendOverflow,
    #[error("a model usage counter exceeds the range of a database bigint")]
    CountOverflow,
}

// SpendLogsPayload.startTime is a datetime or a string; a queued payload carries text, and an in-process caller keeps its datetime's own offset.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(from = "String")]
pub enum UsageStartTime {
    Parsed(DateTime<FixedOffset>),
    Text(String),
}

impl From<String> for UsageStartTime {
    fn from(text: String) -> Self {
        Self::Text(text)
    }
}

// The fields of SpendLogsPayload that the model usage rollup reads, with the source's declared types.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ModelUsagePayload {
    #[serde(rename = "startTime")]
    pub start_time: UsageStartTime,
    pub metadata: String,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub model_group: Option<String>,
    #[serde(default)]
    pub custom_llm_provider: Option<String>,
    pub request_tags: String,
    pub spend: SpendAmount,
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub status: String,
}

// ModelUsageKey: the five columns that identify a LiteLLM_DailyModelUsage row.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct ModelUsageKey {
    pub date: String,
    pub model_group: String,
    pub model: String,
    pub custom_llm_provider: String,
    pub task_type: String,
}

// ModelUsageTransaction of model_usage_rollup.py.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ModelUsageTransaction {
    pub key: ModelUsageKey,
    pub spend: SpendAmount,
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub successful: bool,
}

// One additive upsert per grouped key: the create values also hold the totals this flush adds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelUsageRollup {
    pub key: ModelUsageKey,
    pub spend: Decimal,
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub request_count: i64,
    pub successful_requests: i64,
    pub failed_requests: i64,
}

pub trait ModelUsageFailure: StdError + Send + Sync + 'static {
    fn is_pre_send(&self) -> bool;
}

pub type ModelUsageFuture<'writer, Failure> =
    Pin<Box<dyn Future<Output = Result<(), Failure>> + Send + 'writer>>;

pub trait ModelUsageWriter: Send + Sync {
    type Error: ModelUsageFailure;

    fn apply_rollups<'writer>(
        &'writer self,
        rollups: &'writer [ModelUsageRollup],
    ) -> ModelUsageFuture<'writer, Self::Error>;
}

#[derive(Debug, thiserror::Error)]
pub enum ModelUsageFlushError<WriteError>
where
    WriteError: StdError + Send + Sync + 'static,
{
    #[error("the model usage rollups could not be built")]
    Aggregate(#[source] ModelUsageError),
    #[error("the model usage rollup write failed")]
    Write(#[source] WriteError),
}

// flush_model_usage_transactions: one additive batch in key order, retried only when the failure proves nothing reached the database, because replaying an ambiguous write would count the increments twice.
pub async fn flush_model_usage_transactions<W>(
    writer: &W,
    transactions: &[ModelUsageTransaction],
    retries: u32,
    jitter: &dyn JitterSource,
    pause: &dyn RetryPause,
) -> Result<(), ModelUsageFlushError<W::Error>>
where
    W: ModelUsageWriter,
{
    let rollups =
        rollup::model_usage_rollups(transactions).map_err(ModelUsageFlushError::Aggregate)?;
    if rollups.is_empty() {
        return Ok(());
    }
    let mut attempt = 0_u32;
    loop {
        let failure = match writer.apply_rollups(&rollups).await {
            Ok(()) => return Ok(()),
            Err(failure) => failure,
        };
        if !failure.is_pre_send() || attempt >= retries {
            return Err(ModelUsageFlushError::Write(failure));
        }
        pause.pause(retry_delay(jitter, attempt)).await;
        attempt = attempt.saturating_add(1);
    }
}

// Python sleeps 2**attempt plus random.uniform(0, 1) seconds between attempts
fn retry_delay(jitter: &dyn JitterSource, attempt: u32) -> Duration {
    Duration::from_secs(2_u64.saturating_pow(attempt)).saturating_add(Duration::from_nanos(
        jitter.uniform_nanos(RETRY_JITTER_NANOS),
    ))
}
