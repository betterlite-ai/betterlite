use std::time::Duration;

use crate::spend_batches::{StatementLimits, spend_log_write_batches};
use crate::tool_spend::RetryPause;

use super::{
    GuardrailIndexRow, GuardrailRollupFailure, GuardrailRollupSink, RollupRow, UPSERT_RETRY_TIMES,
};

// Attempts every row once per round, and only a connection-safe failure survives into the next round
pub(crate) async fn rows<Sink>(
    sink: &Sink,
    rows: Vec<RollupRow>,
    pause: &dyn RetryPause,
) -> Vec<RollupRow>
where
    Sink: GuardrailRollupSink,
{
    let mut remaining = rows;
    let mut retries_left = UPSERT_RETRY_TIMES;
    loop {
        let mut retryable = Vec::new();
        for row in remaining {
            let outcome = row.write(sink).await;
            match outcome {
                Ok(()) => {}
                Err(error) if error.is_retry_safe() => retryable.push(row),
                Err(error) => tracing::warn!(
                    "guardrail usage: {} upsert failed for a row that is not safe to retry: {error}",
                    row.label()
                ),
            }
        }
        if retryable.is_empty() || retries_left == 0 {
            for row in &retryable {
                tracing::warn!(
                    "guardrail usage: {} upsert failed after {UPSERT_RETRY_TIMES} retries; requeued for the next flush",
                    row.label()
                );
            }
            return retryable;
        }
        let exponent = UPSERT_RETRY_TIMES.saturating_sub(retries_left);
        let seconds = 1_u64.checked_shl(exponent).unwrap_or(u64::MAX);
        pause.pause(Duration::from_secs(seconds)).await;
        retries_left = retries_left.saturating_sub(1);
        remaining = retryable;
    }
}

// LiteLLM writes the index rows with skip_duplicates and skips a batch that fails, so one conflict never loses the rest
pub(crate) async fn index_batches<Sink>(
    sink: &Sink,
    rows: &[GuardrailIndexRow],
    limits: StatementLimits,
) where
    Sink: GuardrailRollupSink,
{
    for batch in spend_log_write_batches(rows, limits) {
        if let Err(error) = sink.create_index_rows(batch).await {
            tracing::debug!("guardrail usage: index create_many skipped: {error}");
        }
    }
}
