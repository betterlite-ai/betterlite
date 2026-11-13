use std::collections::{BTreeSet, VecDeque};
use std::sync::atomic::Ordering;
use std::sync::{MutexGuard, PoisonError};

use super::logger::NewRelicMetricsLogger;
use super::records::NewRelicMetricRecord;

pub struct QueuedRecord {
    pub id: u64,
    pub record: NewRelicMetricRecord,
}

pub fn push_record(logger: &NewRelicMetricsLogger, record: NewRelicMetricRecord) {
    let id = logger.next_id.fetch_add(1, Ordering::SeqCst);
    let mut queue = logger.queue.lock().unwrap_or_else(PoisonError::into_inner);
    queue.push_back(QueuedRecord { id, record });
    enforce_cap(logger, queue);
}

fn enforce_cap(logger: &NewRelicMetricsLogger, mut queue: MutexGuard<'_, VecDeque<QueuedRecord>>) {
    let overflow = queue.len().saturating_sub(logger.settings.max_queue_size);
    queue.drain(..overflow);
    drop(queue);
    if overflow > 0 {
        tracing::warn!(
            max_queue_size = logger.settings.max_queue_size,
            dropped = overflow,
            "New Relic Metrics: retry queue exceeded max_queue_size; dropped the oldest records"
        );
    }
}

#[must_use]
pub fn queue_length(logger: &NewRelicMetricsLogger) -> usize {
    logger
        .queue
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .len()
}

#[must_use]
pub fn queue_snapshot(logger: &NewRelicMetricsLogger) -> Vec<NewRelicMetricRecord> {
    logger
        .queue
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .iter()
        .map(|queued| queued.record.clone())
        .collect()
}

#[must_use]
pub fn take_front(logger: &NewRelicMetricsLogger, limit: usize) -> Vec<QueuedRecord> {
    let mut queue = logger.queue.lock().unwrap_or_else(PoisonError::into_inner);
    let count = queue.len().min(limit);
    queue.drain(..count).collect()
}

// Prepending in place keeps the records a concurrent callback appended while the send was in flight
pub fn requeue(logger: &NewRelicMetricsLogger, batch: Vec<QueuedRecord>) {
    let mut queue = logger.queue.lock().unwrap_or_else(PoisonError::into_inner);
    for queued in batch.into_iter().rev() {
        queue.push_front(queued);
    }
    enforce_cap(logger, queue);
}

#[must_use]
pub fn queued_ids(logger: &NewRelicMetricsLogger) -> BTreeSet<u64> {
    logger
        .queue
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .iter()
        .map(|queued| queued.id)
        .collect()
}

#[must_use]
pub fn drop_unattempted(logger: &NewRelicMetricsLogger, attempted: &BTreeSet<u64>) -> usize {
    let mut queue = logger.queue.lock().unwrap_or_else(PoisonError::into_inner);
    let before = queue.len();
    queue.retain(|queued| !attempted.contains(&queued.id));
    before.saturating_sub(queue.len())
}

#[must_use]
pub fn take_all(logger: &NewRelicMetricsLogger) -> Vec<QueuedRecord> {
    logger
        .queue
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .drain(..)
        .collect()
}
