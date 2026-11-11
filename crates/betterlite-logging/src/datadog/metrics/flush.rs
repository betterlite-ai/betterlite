use std::sync::Arc;

use tokio::time::timeout;
use tokio_util::task::TaskTracker;

use crate::batch::logger::FlushReport;

use super::datadog_metrics::DatadogMetricsLogger;
use super::delivery::DatadogMetricsBatch;

pub async fn flush_queue(logger: &DatadogMetricsLogger) -> FlushReport {
    let report = logger.queue.flush(&DatadogMetricsBatch { logger }).await;
    if report.batch > 0 {
        tracing::debug!(
            points = report.batch,
            delivered = report.delivered,
            "Datadog Metrics: flushed queued metric points"
        );
    }
    report
}

pub(crate) async fn periodic_flush(logger: Arc<DatadogMetricsLogger>, tasks: TaskTracker) {
    loop {
        // a timeout is the normal wake-up; stopping the logger ends the loop through the token
        let woken = timeout(
            logger.settings.batch.flush_interval,
            logger.stopped.cancelled(),
        )
        .await;
        if woken.is_ok() || tasks.is_closed() {
            return;
        }
        tracing::debug!(
            flush_interval = ?logger.settings.batch.flush_interval,
            "CustomLogger periodic flush"
        );
        let _report = flush_queue(&logger).await;
    }
}

pub async fn stop(logger: &DatadogMetricsLogger) {
    logger.stopped.cancel();
    let _report = flush_queue(logger).await;
}
