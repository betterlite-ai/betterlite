use std::sync::PoisonError;
use std::time::Duration;

use super::delivery::classify_and_send;
use super::logger::NewRelicMetricsLogger;
use super::queue::{QueuedRecord, drop_unattempted, queue_length, queued_ids, requeue, take_all};

pub const MAX_DRAIN_PASSES: u32 = 3;

// A stopped logger funnels every post-stop path here, so a drain never overlaps another one
pub async fn drain_with_retry(logger: &NewRelicMetricsLogger) {
    let _drain = logger.drain_lock.lock().await;
    let attempted = queued_ids(logger);
    for pass in 0..MAX_DRAIN_PASSES {
        drain_flush_once(logger).await;
        if queue_length(logger) == 0 {
            return;
        }
        if pass.saturating_add(1) < MAX_DRAIN_PASSES {
            logger
                .sleeper
                .sleep(Duration::from_secs(1_u64 << pass))
                .await;
        }
    }
    let dropped = drop_unattempted(logger, &attempted);
    if dropped > 0 {
        tracing::warn!(
            dropped,
            passes = MAX_DRAIN_PASSES,
            "New Relic Metrics: dropping records after the drain passes"
        );
    }
}

// Every chunk is attempted: stopping at the first failure would let a failing head starve the tail
async fn drain_flush_once(logger: &NewRelicMetricsLogger) {
    let _flush = logger.flush_lock.lock().await;
    let (pending, window_start) = {
        let mut last_flush = logger
            .last_flush_time
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let window_start = *last_flush;
        *last_flush = (logger.clock)();
        drop(last_flush);
        (take_all(logger), window_start)
    };
    if pending.is_empty() {
        return;
    }
    let mut failed: Vec<QueuedRecord> = Vec::new();
    let mut remaining = pending;
    while !remaining.is_empty() {
        let count = remaining.len().min(logger.settings.batch_size);
        let chunk: Vec<QueuedRecord> = remaining.drain(..count).collect();
        if !classify_and_send(logger, &chunk, window_start).await {
            failed.extend(chunk);
        }
    }
    if !failed.is_empty() {
        requeue(logger, failed);
    }
}
