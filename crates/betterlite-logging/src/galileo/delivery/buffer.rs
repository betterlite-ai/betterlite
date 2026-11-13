use std::sync::PoisonError;

use super::config::MAX_IN_MEMORY_RECORDS;
use super::{GalileoDelivery, Record};

// The append half of GalileoObserve.async_log_success_event: keep the newest MAX_IN_MEMORY_RECORDS so failing flushes cannot grow memory without limit.
pub fn append(delivery: &GalileoDelivery, record: Record) {
    let mut state = delivery
        .state
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    state.records.push(record);
    let queued = state.records.len();
    let dropped = queued.saturating_sub(MAX_IN_MEMORY_RECORDS);
    state.records.drain(..dropped);
    drop(state);
    tracing::debug!("Galileo Logger: queued record, in_memory={queued}");
    if dropped > 0 {
        tracing::warn!(
            "Galileo Logger: in-memory buffer exceeded {MAX_IN_MEMORY_RECORDS} records; dropped {dropped} oldest record(s). Check Galileo connectivity/credentials."
        );
    }
}
