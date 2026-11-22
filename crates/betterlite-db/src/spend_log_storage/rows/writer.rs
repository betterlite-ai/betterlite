use std::error::Error as StdError;
use std::num::NonZeroUsize;
use std::time::Duration;

use crate::key_user_writes::retry::WriteClock;
use crate::redis_spend::buffer::RedisUpdateBuffer;
use crate::redis_spend::log_row::SpendLogRow;
use crate::spend_batches::{StatementLimits, spend_log_write_batches};
use crate::tool_spend::RetryPause;

use super::parking::requeue_spend_logs;
use super::{MAX_LOGS_PER_INTERVAL, SpendLogQueue};
use crate::spend_log_storage::poison::{MAX_ISOLATION_FAILURES, create_logs_isolating_poison};
use crate::spend_log_storage::{SpendLogFuture, SpendLogWriteError, SpendLogsWriter};

mod tests;

// BATCH_SIZE: the rows one outer database round handles before the isolation budget resets
pub const BATCH_SIZE: usize = 1000;

// update_spend_logs_job's n_retry_times: one pass tries four times before it raises
pub const DEFAULT_RETRIES: u32 = 3;

pub const JSON_CONTENT_TYPE: &str = "application/json";

pub const NON_BLOCKING_PREFIX: &str =
    "[Non-Blocking]LiteLLM Prisma Client Exception - update spend logs:";

pub const UPDATE_SPEND_CALL_TYPE: &str = "update_spend";

// db_writer_client.post: the external spend writer takes the serialized array and answers with a status.
pub struct SpendUpdateRequest<'body> {
    pub url: &'body str,
    pub body: &'body str,
    pub content_type: &'static str,
}

pub trait SpendUpdateClient: Send + Sync {
    type Error: StdError + Send + Sync + 'static;

    fn post<'call>(
        &'call self,
        request: SpendUpdateRequest<'call>,
    ) -> SpendLogFuture<'call, Result<u16, Self::Error>>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlushPolicy {
    pub retries: u32,
    pub limits: StatementLimits,
    pub spend_logs_url: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum FlushError<StoreError, HttpError> {
    #[error("the spend log flush could not write to the database")]
    Store(#[source] StoreError),
    #[error("the spend log flush could not encode its rows")]
    Encode(#[source] serde_json::Error),
    #[error("the spend log flush could not post to the external writer")]
    External(#[source] HttpError),
}

impl<StoreError, HttpError> FlushError<StoreError, HttpError>
where
    StoreError: SpendLogWriteError,
{
    // _is_transient_spend_log_write_error: only a database write can be worth retrying, and a non-database error is never "prisma"
    fn is_transient(&self) -> bool {
        match self {
            Self::Store(error) => error.is_transient(),
            Self::Encode(_) | Self::External(_) => false,
        }
    }

    fn is_database(&self) -> bool {
        match self {
            Self::Store(error) => error.is_database(),
            Self::Encode(_) | Self::External(_) => false,
        }
    }
}

// The failure handler's event: an owned traceback text, the call type and how long the failed pass took.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpendLogFailureEvent {
    pub call_type: &'static str,
    pub duration: Duration,
    pub diagnostic: String,
}

pub trait SpendLogFailureSink: Send + Sync {
    fn emit(&self, event: SpendLogFailureEvent);
}

#[derive(Debug, Clone, Copy, Default)]
pub struct NoSpendLogFailureSink;

impl SpendLogFailureSink for NoSpendLogFailureSink {
    fn emit(&self, _event: SpendLogFailureEvent) {}
}

// ProxyUpdateSpend.update_spend_logs over one pass: pop, write through the external writer or the database, retry, report.
pub struct SpendLogFlush<'run, Writer, Client, Sink> {
    pub writer: &'run Writer,
    pub external: Option<&'run Client>,
    pub queue: &'run SpendLogQueue,
    pub redis: &'run RedisUpdateBuffer,
    pub failures: &'run Sink,
    pub pause: &'run dyn RetryPause,
    pub clock: &'run dyn WriteClock,
    pub policy: FlushPolicy,
}

impl<Writer, Client, Sink> SpendLogFlush<'_, Writer, Client, Sink>
where
    Writer: SpendLogsWriter,
    Client: SpendUpdateClient,
    Sink: SpendLogFailureSink,
{
    pub async fn run(
        &self,
        logs_to_process: Option<&[SpendLogRow]>,
    ) -> Result<(), FlushError<Writer::Error, Client::Error>> {
        let popped;
        let logs = if let Some(logs) = logs_to_process {
            logs
        } else {
            popped = self.queue.dequeue(MAX_LOGS_PER_INTERVAL);
            &popped
        };
        if let Some(processing) = NonZeroUsize::new(logs.len()) {
            tracing::info!("Spend tracking - processing {processing} spend logs for DB write");
        }
        let started = self.clock.instant();
        let outcome = self.write_until_settled(logs).await;
        match outcome {
            Ok(()) => Ok(()),
            Err(error) => {
                let duration = self.clock.instant().saturating_duration_since(started);
                report_failed_update_spend(self.failures, &error, duration);
                Err(error)
            }
        }
    }

    async fn write_until_settled(
        &self,
        logs: &[SpendLogRow],
    ) -> Result<(), FlushError<Writer::Error, Client::Error>> {
        let mut attempt: u32 = 0;
        loop {
            let Err(error) = self.write_once(logs).await else {
                return Ok(());
            };
            if !error.is_transient() {
                self.requeue_after_database_error(logs, &error).await;
                return Err(error);
            }
            let retry = attempt.saturating_add(1);
            let retries = self.policy.retries;
            let logs_count = logs.len();
            tracing::warn!(
                "Spend tracking - transient DB error writing spend logs, retry {retry}/{retries}. logs_count={logs_count}, error={error}"
            );
            if attempt >= self.policy.retries {
                requeue_spend_logs(self.queue, self.redis, logs).await;
                return Err(error);
            }
            // asyncio.sleep(1 << i): the pass waits 1, 2, 4 ... seconds between attempts
            let delay = Duration::from_secs(1_u64.checked_shl(attempt).unwrap_or(u64::MAX));
            self.pause.pause(delay).await;
            attempt = attempt.saturating_add(1);
        }
    }

    // a settled database error hands its rows back once, while an encoding or external failure keeps LiteLLM's boundary
    async fn requeue_after_database_error(
        &self,
        logs: &[SpendLogRow],
        error: &FlushError<Writer::Error, Client::Error>,
    ) {
        if error.is_database() {
            requeue_spend_logs(self.queue, self.redis, logs).await;
            let requeued = logs.len();
            tracing::warn!(
                "Spend tracking - DB error writing spend logs, requeued {requeued} rows for the next flush. error={error}"
            );
        }
    }

    async fn write_once(
        &self,
        logs: &[SpendLogRow],
    ) -> Result<(), FlushError<Writer::Error, Client::Error>> {
        if logs.is_empty() {
            return Ok(());
        }
        if let (Some(client), Some(base_url)) =
            (self.external, self.policy.spend_logs_url.as_deref())
        {
            return self.post_external(client, base_url, logs).await;
        }
        for group in logs.chunks(BATCH_SIZE) {
            let mut budget = MAX_ISOLATION_FAILURES;
            for statement in spend_log_write_batches(group, self.policy.limits) {
                budget = create_logs_isolating_poison(self.writer, statement, budget)
                    .await
                    .map_err(FlushError::Store)?;
            }
            let flushed = group.len();
            tracing::debug!("Flushed {flushed} logs to the DB.");
        }
        let processed = logs.len();
        let remaining = self.queue.len();
        tracing::debug!("{processed} logs processed. Remaining in queue: {remaining}");
        Ok(())
    }

    async fn post_external(
        &self,
        client: &Client,
        base_url: &str,
        logs: &[SpendLogRow],
    ) -> Result<(), FlushError<Writer::Error, Client::Error>> {
        let mut url = base_url.to_owned();
        if !url.ends_with('/') {
            url.push('/');
        }
        url.push_str("spend/update");
        let body = serde_json::to_string(logs).map_err(FlushError::Encode)?;
        // The source acts only on a 200 and leaves every other status inside the successful try, so a non-200 still counts as a completed pass.
        let _status = client
            .post(SpendUpdateRequest {
                url: &url,
                body: &body,
                content_type: JSON_CONTENT_TYPE,
            })
            .await
            .map_err(FlushError::External)?;
        Ok(())
    }
}

// _raise_failed_update_spend_exception: report the non-blocking failure and hand the original error back
fn report_failed_update_spend<Sink>(sink: &Sink, error: &dyn StdError, duration: Duration)
where
    Sink: SpendLogFailureSink + ?Sized,
{
    let traceback = diagnostic(error);
    tracing::error!("{traceback}");
    sink.emit(SpendLogFailureEvent {
        call_type: UPDATE_SPEND_CALL_TYPE,
        duration,
        diagnostic: traceback,
    });
}

fn diagnostic(error: &dyn StdError) -> String {
    let mut text = format!("{NON_BLOCKING_PREFIX} {error}");
    let mut cause = error.source();
    while let Some(source) = cause {
        text.push_str("\nCaused by: ");
        text.push_str(&source.to_string());
        cause = source.source();
    }
    text
}
