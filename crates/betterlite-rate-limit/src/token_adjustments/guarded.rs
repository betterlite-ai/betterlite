use std::str::from_utf8;

use crate::limiter_records::{CacheCounterValue, ReservationAwareIncrement};
use crate::rate_limit_window_store::counter_value::counter_as_int;

use super::scripts::window_guarded_args;
use super::store::{TokenAdjustmentError, TokenAdjustments, token_float};

pub async fn apply_local_guarded_tokens(
    adjustments: &TokenAdjustments,
    operations: &[ReservationAwareIncrement],
) -> Result<(), TokenAdjustmentError> {
    let _guard = adjustments.windows.check_and_increment_lock.lock().await;
    for operation in operations {
        apply_local_guarded_token(adjustments, operation).await?;
    }
    Ok(())
}

async fn apply_local_guarded_token(
    adjustments: &TokenAdjustments,
    operation: &ReservationAwareIncrement,
) -> Result<(), TokenAdjustmentError> {
    let Some((window_key, expected)) = window_guard(operation) else {
        return Ok(());
    };
    let Some(active) = adjustments.windows.cache.get_cache(window_key, true).await else {
        return Ok(());
    };
    let Ok(expected_start) = expected.parse::<i64>() else {
        return Ok(());
    };
    let Ok(active_start) = counter_as_int(&active) else {
        return Ok(());
    };
    if active_start != expected_start {
        return Ok(());
    }
    let current = match adjustments
        .windows
        .cache
        .get_cache(&operation.operation.key, true)
        .await
    {
        Some(value) => counter_as_float(&value)?,
        None => 0.0_f64,
    };
    let updated = current.mul_add(1.0_f64, operation.operation.increment_value);
    adjustments
        .usage
        .set_local_counter(&operation.operation.key, updated, operation.operation.ttl)
        .await;
    Ok(())
}

pub async fn apply_redis_guarded_tokens(
    adjustments: &TokenAdjustments,
    operations: &[ReservationAwareIncrement],
) {
    for operation in operations {
        apply_redis_guarded_token(adjustments, operation).await;
    }
}

async fn apply_redis_guarded_token(
    adjustments: &TokenAdjustments,
    operation: &ReservationAwareIncrement,
) {
    let Some((window_key, expected)) = window_guard(operation) else {
        return;
    };
    if let Some(script) = adjustments.guarded_token_script.as_deref() {
        let keys = vec![window_key.to_owned(), operation.operation.key.clone()];
        let args = window_guarded_args(expected, &operation.operation);
        let Err(error) = script.run(&keys, &args).await else {
            return;
        };
        tracing::warn!(
            %error,
            key = %operation.operation.key,
            "window-guarded token adjustment failed"
        );
    }
    if operation.operation.increment_value > 0.0_f64 {
        adjustments.usage.increment(&operation.operation).await;
    }
}

fn window_guard(operation: &ReservationAwareIncrement) -> Option<(&str, &str)> {
    let window_key = operation.window_key.as_deref()?;
    let expected = operation.expected_window_start.as_deref()?;
    Some((window_key, expected))
}

fn counter_as_float(value: &CacheCounterValue) -> Result<f64, TokenAdjustmentError> {
    match value {
        CacheCounterValue::Int(number) => token_float(*number),
        CacheCounterValue::Float(number) => Ok(*number),
        CacheCounterValue::Text(text) => text
            .trim()
            .parse::<f64>()
            .map_err(TokenAdjustmentError::CounterText),
        CacheCounterValue::Bytes(bytes) => {
            let text = from_utf8(bytes).map_err(TokenAdjustmentError::CounterBytes)?;
            text.trim()
                .parse::<f64>()
                .map_err(TokenAdjustmentError::CounterText)
        }
    }
}
