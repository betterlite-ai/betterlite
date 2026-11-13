use std::sync::atomic::Ordering;
use std::sync::{Arc, PoisonError};
use std::time::Duration;

use tokio::time::timeout;

use super::delivery::classify_and_send;
use super::drain::drain_with_retry;
use super::logger::NewRelicMetricsLogger;
use super::queue::{queue_length, requeue, take_front};

pub async fn flush_queue(logger: &NewRelicMetricsLogger) {
    let _flush = logger.flush_lock.lock().await;
    let window_start = {
        let mut last_flush = logger
            .last_flush_time
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let window_start = *last_flush;
        *last_flush = (logger.clock)();
        window_start
    };
    let queued = queue_length(logger);
    if queued == 0 {
        return;
    }
    tracing::debug!(queued, "New Relic Metrics: Flushing queued records");
    // Bounded by the queue at lock release: records appended mid-flush belong to the next window
    let chunks = queued.div_ceil(logger.settings.batch_size);
    for _chunk in 0..chunks {
        if !send_batch_at(logger, window_start).await {
            return;
        }
    }
}

pub async fn async_send_batch(logger: &NewRelicMetricsLogger) -> bool {
    let window_start = {
        *logger
            .last_flush_time
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    };
    send_batch_at(logger, window_start).await
}

async fn send_batch_at(logger: &NewRelicMetricsLogger, window_start: Duration) -> bool {
    let batch = take_front(logger, logger.settings.batch_size);
    if batch.is_empty() {
        return false;
    }
    let delivered = classify_and_send(logger, &batch, window_start).await;
    if !delivered {
        requeue(logger, batch);
    }
    delivered
}

pub(super) async fn periodic_flush(logger: Arc<NewRelicMetricsLogger>) {
    while !logger.stopped.load(Ordering::SeqCst) && !logger.tasks.is_closed() {
        // A timeout is the normal wake-up; stop() wakes the loop early through the signal
        let _woken = timeout(
            logger.settings.flush_interval,
            logger.stop_signal.notified(),
        )
        .await;
        if logger.stopped.load(Ordering::SeqCst) {
            break;
        }
        flush_queue(&logger).await;
    }
    drain_with_retry(&logger).await;
}
