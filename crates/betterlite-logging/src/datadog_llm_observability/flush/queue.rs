use std::sync::PoisonError;

use crate::batch::logger::BatchQueue;
use crate::datadog_llm_metrics::token_counting::ToolTokenCounter;

use super::drain;
use crate::datadog_llm_observability::logger::DatadogLlmObsLogger;

// the inherited flush clears the queue after every send attempt, even the one that failed
pub async fn flush_queue<Counter>(logger: &DatadogLlmObsLogger<Counter>)
where
    Counter: ToolTokenCounter + Send + Sync,
{
    if logger.queue.pending().is_empty() {
        return;
    }
    let report = drain(logger).await;
    logging_done(&logger.queue, report.cancelled);
}

fn logging_done<E>(queue: &BatchQueue<E>, cancelled: bool) {
    if !cancelled {
        return;
    }
    let mut pending = queue.queue.lock().unwrap_or_else(PoisonError::into_inner);
    pending.clear();
}
