use crate::limiter_records::{ReservationAwareIncrement, ReservationBackend};

use super::guarded::{apply_local_guarded_tokens, apply_redis_guarded_tokens};
use super::store::{TokenAdjustmentError, TokenAdjustments};

pub async fn apply_reservation_aware_tokens(
    adjustments: &TokenAdjustments,
    operations: &[ReservationAwareIncrement],
) -> Result<(), TokenAdjustmentError> {
    let mut local: Vec<ReservationAwareIncrement> = Vec::new();
    let mut redis: Vec<ReservationAwareIncrement> = Vec::new();
    for operation in operations {
        if operation.window_key.is_none() || operation.expected_window_start.is_none() {
            adjustments.usage.increment(&operation.operation).await;
            continue;
        }
        match operation.reservation_backend {
            Some(ReservationBackend::Local) => local.push(operation.clone()),
            Some(ReservationBackend::Redis) | None => redis.push(operation.clone()),
        }
    }
    if !local.is_empty() {
        apply_local_guarded_tokens(adjustments, &local).await?;
    }
    if !redis.is_empty() {
        apply_redis_guarded_tokens(adjustments, &redis).await;
    }
    Ok(())
}
