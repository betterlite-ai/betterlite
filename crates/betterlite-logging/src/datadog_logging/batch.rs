use std::sync::{Arc, PoisonError};

use tokio::time::timeout;
use tokio_util::task::TaskTracker;

use crate::batch::logger::FlushReport;
use crate::callback_logger::event::LoggerEvent;
use crate::datadog::payloads::standard::standard_logging_to_datadog;
use crate::datadog::payloads::status::DataDogStatus;

use super::logger::DatadogLogger;
use super::send::DatadogBatch;

pub(crate) const NO_BATCH: FlushReport = FlushReport {
    batch: 0,
    delivered: 0,
    requeued: 0,
    dropped_oldest: 0,
    cancelled: false,
};

pub(crate) async fn log_async_event(logger: &DatadogLogger, event: &LoggerEvent) {
    let status = DataDogStatus::from_logging_status(event.payload.status);
    let payload = match standard_logging_to_datadog(
        &event.payload,
        status,
        &logger.settings.environment,
        &logger.tracer,
    ) {
        Ok(payload) => payload,
        Err(error) => {
            tracing::error!(%error, "Datadog Layer Error");
            return;
        }
    };
    let enqueued = logger.queue.enqueue(payload);
    tracing::debug!(
        flush_interval = ?logger.settings.batch.flush_interval,
        "Datadog, event added to queue. Will flush on the interval or at the batch size"
    );
    if enqueued.depth >= logger.settings.batch.batch_size {
        flush_queue(logger).await;
    }
}

pub async fn async_send_batch(logger: &DatadogLogger) -> FlushReport {
    let empty = logger
        .queue
        .queue
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .is_empty();
    if empty {
        tracing::error!("Datadog: log_queue does not exist");
        return NO_BATCH;
    }
    logger.queue.flush(&DatadogBatch { logger }).await
}

pub async fn flush_queue(logger: &DatadogLogger) {
    let queued = logger
        .queue
        .queue
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .len();
    if queued == 0 {
        return;
    }
    tracing::debug!(events = queued, "Datadog: Flushing batch of events");
    let _report = async_send_batch(logger).await;
    let drained = logger
        .queue
        .queue
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .is_empty();
    if drained {
        let now = logger.clock.now_unix();
        *logger
            .last_flush_time
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = now;
    }
}

pub async fn stop(logger: &DatadogLogger) {
    logger.stopped.cancel();
    flush_queue(logger).await;
}

pub(crate) async fn periodic_flush(logger: &Arc<DatadogLogger>, tasks: &TaskTracker) {
    loop {
        let shutdown = timeout(
            logger.settings.batch.flush_interval,
            logger.stopped.cancelled(),
        )
        .await;
        if shutdown.is_ok() || tasks.is_closed() {
            return;
        }
        tracing::debug!(
            flush_interval = ?logger.settings.batch.flush_interval,
            "CustomLogger periodic flush"
        );
        flush_queue(logger).await;
    }
}
