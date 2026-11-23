use std::error::Error as StdError;
use std::mem;

use betterlite_models::tool_spend::ToolUsageTransaction;
use tokio::sync::{Mutex, Notify};
use tokio_util::sync::CancellationToken;

use crate::autorouter_drain::{AutoRouterDrain, TurnWriter};
use crate::autorouter_turn_sql::transaction::AutoRouterTurnTransaction;
use crate::baseline::flush::flush_baseline_accounting;
use crate::baseline::flush::queue::BaselineQueue;
use crate::baseline::store::BaselineAccounting;
use crate::guardrail_usage_rollups::{
    GuardrailRollupSink, GuardrailSpendLog, SharedPendingRollups, process_guardrail_usage,
};
use crate::migration_ledger::lock::JitterSource;
use crate::model_usage::{ModelUsageTransaction, ModelUsageWriter, flush_model_usage_transactions};
use crate::redis_spend::log_row::SpendLogRow;
use crate::shadow_tables::funnel::FunnelQueue;
use crate::shadow_tables::rows::ShadowTables;
use crate::spend_batches::StatementLimits;
use crate::spend_log_storage::SpendLogsWriter;
use crate::spend_log_storage::rows::parking::requeue_spend_logs;
use crate::spend_log_storage::rows::writer::{
    FlushError, SpendLogFailureSink, SpendLogFlush, SpendUpdateClient,
};
use crate::spend_log_storage::rows::{MAX_LOGS_PER_INTERVAL, QueuePosition, SpendLogQueue};
use crate::tool_spend::{
    FlushPolicy as ToolFlushPolicy, RetryPause, ToolSpendWriter, flush_tool_usage_transactions,
};

use super::baseline_capture::baseline_pending;
use super::queues::UsageQueue;

mod tests;

// Every request-time queue the census reads, so a queue added later joins the census and each job gate at once.
pub struct RequestUsageQueues<'run> {
    pub logs: &'run SpendLogQueue,
    pub tool_usage: &'run UsageQueue<ToolUsageTransaction>,
    pub model_usage: &'run UsageQueue<ModelUsageTransaction>,
    pub turns: &'run UsageQueue<AutoRouterTurnTransaction>,
    pub baseline: &'run BaselineQueue,
    pub funnel: &'run FunnelQueue,
}

// _total_queued_spend_transactions: each queue under its own lock, plus the pending shadow-funnel events.
#[must_use]
pub fn queued_spend_transactions(queues: &RequestUsageQueues<'_>) -> i64 {
    let sizes = [
        queues.logs.len(),
        queues.tool_usage.len(),
        queues.model_usage.len(),
        queues.turns.len(),
        baseline_pending(queues.baseline),
    ];
    let mut total = queues.funnel.pending_events();
    for count in sizes {
        total = total.saturating_add(i64::try_from(count).unwrap_or(i64::MAX));
    }
    total
}

pub struct GuardrailStage<'run, Sink> {
    pub pending: &'run SharedPendingRollups,
    pub sink: &'run Sink,
    pub limits: StatementLimits,
    pub pause: &'run dyn RetryPause,
}

pub struct ToolUsageDrain<'run, Writer> {
    pub writer: &'run Writer,
    pub policy: ToolFlushPolicy,
    pub jitter: &'run dyn JitterSource,
    pub pause: &'run dyn RetryPause,
}

pub struct ModelUsageDrain<'run, Writer> {
    pub writer: &'run Writer,
    pub retries: u32,
    pub jitter: &'run dyn JitterSource,
    pub pause: &'run dyn RetryPause,
}

// What a pass popped; a future dropped before it settles returns the rows to the memory head, since a Drop cannot wait for Redis.
struct PoppedBatch<'run> {
    queue: &'run SpendLogQueue,
    rows: Vec<SpendLogRow>,
    settled: bool,
}

impl Drop for PoppedBatch<'_> {
    fn drop(&mut self) {
        if self.settled {
            return;
        }
        self.queue.enqueue(&self.rows, QueuePosition::Head);
        let returned = self.rows.len();
        tracing::warn!(
            "Spend tracking - spend log batch interrupted, returned {returned} rows to the queue head"
        );
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SpendLogsJobError<StoreError, HttpError>
where
    StoreError: StdError + Send + Sync + 'static,
    HttpError: StdError + Send + Sync + 'static,
{
    #[error("the spend log write failed")]
    Logs(#[source] FlushError<StoreError, HttpError>),
    #[error("the spend log write was cancelled; its rows were requeued for the next flush")]
    Cancelled,
}

// update_spend_logs_job and _run_spend_logs_job over one client: the census, one log pop and the ordered derived drains.
pub struct SpendLogsJob<
    'run,
    LogWriter,
    LogClient,
    LogFailures,
    Guardrails,
    Tools,
    Models,
    Turns,
    Accounting,
    Shadows,
> {
    // `queues.logs` and `logs.queue` must be the same queue: the pass pops from one and retries through the other.
    pub queues: RequestUsageQueues<'run>,
    pub logs: SpendLogFlush<'run, LogWriter, LogClient, LogFailures>,
    pub guardrails: GuardrailStage<'run, Guardrails>,
    pub tools: ToolUsageDrain<'run, Tools>,
    pub models: ModelUsageDrain<'run, Models>,
    pub turns: AutoRouterDrain<'run, Turns>,
    pub accounting: &'run Accounting,
    pub notifier: &'run Notify,
    pub shadows: &'run Shadows,
    pub write_lock: &'run Mutex<()>,
    pub shutdown: &'run CancellationToken,
}

impl<LogWriter, LogClient, LogFailures, Guardrails, Tools, Models, Turns, Accounting, Shadows>
    SpendLogsJob<
        '_,
        LogWriter,
        LogClient,
        LogFailures,
        Guardrails,
        Tools,
        Models,
        Turns,
        Accounting,
        Shadows,
    >
where
    LogWriter: SpendLogsWriter,
    LogClient: SpendUpdateClient,
    LogFailures: SpendLogFailureSink,
    Guardrails: GuardrailRollupSink,
    Tools: ToolSpendWriter,
    Models: ModelUsageWriter,
    Turns: TurnWriter,
    Accounting: BaselineAccounting,
    Shadows: ShadowTables,
{
    // An empty census still flushes the baseline accounting; any other census runs one pass under the write lock.
    pub async fn update_spend_logs_job(
        &self,
    ) -> Result<(), SpendLogsJobError<LogWriter::Error, LogClient::Error>> {
        if queued_spend_transactions(&self.queues) == 0 {
            flush_baseline_accounting(self.accounting, self.queues.baseline, self.notifier).await;
            return Ok(());
        }
        let _write = self.write_lock.lock().await;
        self.run_spend_logs_job().await
    }

    // _run_spend_logs_job: pop at most 10000 logs once and write them; only that write stops the pass, while every derived stage logs its loss and continues.
    pub async fn run_spend_logs_job(
        &self,
    ) -> Result<(), SpendLogsJobError<LogWriter::Error, LogClient::Error>> {
        let rows = self.write_popped_logs().await?;
        self.flush_derived(&rows).await;
        Ok(())
    }

    // One pop and one write, with the rows back in the queue whenever that write does not settle.
    async fn write_popped_logs(
        &self,
    ) -> Result<Vec<SpendLogRow>, SpendLogsJobError<LogWriter::Error, LogClient::Error>> {
        let mut batch = PoppedBatch {
            queue: self.queues.logs,
            rows: self.queues.logs.dequeue(MAX_LOGS_PER_INTERVAL),
            settled: false,
        };
        let outcome = tokio::select! {
            biased;
            () = self.shutdown.cancelled() => {
                requeue_spend_logs(self.queues.logs, self.logs.redis, batch.rows.as_slice()).await;
                let requeued = batch.rows.len();
                tracing::warn!(
                    "Spend tracking - spend log write cancelled, requeued {requeued} rows for the next flush"
                );
                Err(SpendLogsJobError::Cancelled)
            }
            written = self.logs.run(Some(batch.rows.as_slice())) => {
                written.map_err(SpendLogsJobError::Logs)
            }
        };
        batch.settled = true;
        outcome?;
        Ok(mem::take(&mut batch.rows))
    }

    // The derived stages in the source's order, each one after the logs settled.
    async fn flush_derived(&self, rows: &[SpendLogRow]) {
        let payloads = guardrail_payloads(rows);
        process_guardrail_usage(
            &payloads,
            self.guardrails.pending,
            self.guardrails.sink,
            self.guardrails.limits,
            self.guardrails.pause,
        )
        .await;
        self.flush_tool_usage().await;
        self.flush_model_usage().await;
        flush_baseline_accounting(self.accounting, self.queues.baseline, self.notifier).await;
        let turns = self.queues.turns.drain(MAX_LOGS_PER_INTERVAL);
        self.turns.flush_turn_transactions(turns).await;
        self.queues.funnel.flush(self.shadows).await;
    }

    // At most 10000 tool records per pass, never retried, and a failed batch is dropped with an error.
    async fn flush_tool_usage(&self) {
        let transactions = self.queues.tool_usage.drain(MAX_LOGS_PER_INTERVAL);
        if let Err(error) = flush_tool_usage_transactions(
            self.tools.writer,
            &transactions,
            self.tools.policy,
            self.tools.jitter,
            self.tools.pause,
        )
        .await
        {
            let dropped = transactions.len();
            tracing::error!(
                "Spend tracking - tool usage flush failed; {dropped} tool usage transactions dropped: {error}"
            );
        }
    }

    // Every model record drains at once, retried only where a replay cannot double count.
    async fn flush_model_usage(&self) {
        let writer = self.models.writer;
        let retries = self.models.retries;
        let transactions = self.queues.model_usage.drain(usize::MAX);
        let outcome = flush_model_usage_transactions(
            writer,
            &transactions,
            retries,
            self.models.jitter,
            self.models.pause,
        )
        .await;
        let Err(error) = outcome else {
            return;
        };
        let dropped = transactions.len();
        tracing::error!(
            "Spend tracking - model usage flush failed; {dropped} model usage transactions dropped: {error}"
        );
    }
}

// The same rows the write popped, decoded for B3688; one unreadable row is skipped with a warning and never stops the rest.
fn guardrail_payloads(rows: &[SpendLogRow]) -> Vec<GuardrailSpendLog> {
    let mut payloads = Vec::with_capacity(rows.len());
    for row in rows {
        let decoded = serde_json::to_string(row)
            .and_then(|text| serde_json::from_str::<GuardrailSpendLog>(&text));
        match decoded {
            Ok(payload) => payloads.push(payload),
            Err(error) => {
                tracing::warn!(
                    "Spend tracking - guardrail usage skipped an unreadable spend log row: {error}"
                );
            }
        }
    }
    payloads
}
