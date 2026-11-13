use std::sync::Arc;

use tokio::time::sleep;
use tokio_util::task::TaskTracker;

use crate::batch::logger::FlushReport;

use super::datadog_metrics::DatadogMetricsLogger;
use super::delivery::DatadogMetricsBatch;

pub async fn flush_queue(logger: &DatadogMetricsLogger) -> FlushReport {
    let report = logger.queue.flush(&DatadogMetricsBatch { logger }).await;
    tracing::debug!(
        points = report.batch,
        delivered = report.delivered,
        "Datadog Metrics: flushed queued metric points"
    );
    report
}

pub(crate) async fn periodic_flush(logger: Arc<DatadogMetricsLogger>, tasks: TaskTracker) {
    loop {
        sleep(logger.settings.batch.flush_interval).await;
        // a closed tracker is the server shutting down, so the last tick delivers the queue
        let _report = flush_queue(&logger).await;
        if tasks.is_closed() {
            return;
        }
        tracing::debug!(
            flush_interval = ?logger.settings.batch.flush_interval,
            "CustomLogger periodic flush"
        );
    }
}
