use std::collections::{BTreeMap, BTreeSet};

use super::BatchCounterValueError;
use crate::api::batch_limit_counters::ports::BatchCounterLimiter;
use crate::api::batch_limit_counters::records::{
    BatchCounterLimitType, BatchReservationWindow, BatchTpdRefundOperation,
};
use crate::api::batch_limit_descriptors::descriptors::{
    BatchLimitDescriptor, TPD_SUFFIX, TPD_WINDOW_SECONDS,
};

pub(crate) fn refund_ops<Limiter>(
    descriptors: &[BatchLimitDescriptor],
    tokens: i64,
    reservation_windows: &BTreeSet<BatchReservationWindow>,
    limiter: &Limiter,
) -> Result<Vec<BatchTpdRefundOperation>, BatchCounterValueError>
where
    Limiter: BatchCounterLimiter,
{
    if tokens <= 0 || reservation_windows.is_empty() {
        return Ok(Vec::new());
    }
    let mut by_counter: BTreeMap<String, &BatchLimitDescriptor> = BTreeMap::new();
    for descriptor in descriptors
        .iter()
        .filter(|descriptor| descriptor.key.ends_with(TPD_SUFFIX))
    {
        let counter_key = limiter.create_rate_limit_keys(
            &descriptor.key,
            &descriptor.value,
            BatchCounterLimitType::Tokens,
        );
        by_counter.insert(counter_key, descriptor);
    }
    let increment_value = tokens
        .checked_neg()
        .ok_or(BatchCounterValueError::OutOfRange)?;
    let mut operations = Vec::new();
    for window in reservation_windows {
        let Some(charged) = by_counter.get(&window.counter_key) else {
            continue;
        };
        operations.push(BatchTpdRefundOperation {
            key: window.counter_key.clone(),
            increment_value,
            ttl: TPD_WINDOW_SECONDS,
            window_key: format!("{{{}:{}}}:window", charged.key, charged.value),
            expected_window_start: window.window_start.clone(),
            reservation_backend: window.backend,
        });
    }
    Ok(operations)
}
