use std::sync::Arc;
use std::sync::atomic::AtomicUsize;

use tokio::time::sleep;
use tokio_util::task::TaskTracker;

use crate::batch::delivery::{BatchCancelled, BatchDelivery};
use crate::batch::logger::FlushReport;
use crate::datadog_llm_metrics::token_counting::ToolTokenCounter;
use crate::datadog_llm_obs::span_records::LLMObsPayload;

use super::logger::DatadogLlmObsLogger;
use super::send;
use queue::flush_queue;

pub mod queue;

struct IntakeDelivery<'logger, Counter: ToolTokenCounter> {
    logger: &'logger DatadogLlmObsLogger<Counter>,
}

impl<Counter> BatchDelivery<LLMObsPayload> for IntakeDelivery<'_, Counter>
where
    Counter: ToolTokenCounter + Send + Sync,
{
    async fn deliver(
        &self,
        batch: &[LLMObsPayload],
        _settled: &AtomicUsize,
    ) -> Result<Vec<LLMObsPayload>, BatchCancelled<LLMObsPayload>> {
        let spans = batch.to_vec();
        match send::send_batch(&self.logger.settings, self.logger.transport.as_ref(), spans).await {
            Ok(()) => Ok(Vec::new()),
            Err(error) => {
                tracing::error!(%error, "DataDogLLMObs: Error sending batch");
                Err(BatchCancelled {
                    undelivered: batch.to_vec(),
                })
            }
        }
    }
}

pub(super) async fn drain<Counter>(logger: &DatadogLlmObsLogger<Counter>) -> FlushReport
where
    Counter: ToolTokenCounter + Send + Sync,
{
    let report = logger.queue.flush(&IntakeDelivery { logger }).await;
    tracing::debug!(
        events = report.batch,
        cancelled = report.cancelled,
        "DataDogLLMObs: batch flushed"
    );
    report
}

// the threshold send leaves the queue as it was when the intake refused the batch
pub async fn async_send_batch<Counter>(logger: &DatadogLlmObsLogger<Counter>)
where
    Counter: ToolTokenCounter + Send + Sync,
{
    let report = drain(logger).await;
    if report.cancelled {
        tracing::debug!("DataDogLLMObs: the refused threshold batch stays queued");
    }
}

pub(crate) async fn periodic_flush<Counter>(
    logger: Arc<DatadogLlmObsLogger<Counter>>,
    tasks: TaskTracker,
) where
    Counter: ToolTokenCounter + Send + Sync,
{
    loop {
        sleep(logger.settings.batch.flush_interval).await;
        flush_queue(&logger).await;
        if tasks.is_closed() {
            return;
        }
        tracing::debug!(
            flush_interval = ?logger.settings.batch.flush_interval,
            "CustomLogger periodic flush"
        );
    }
}
