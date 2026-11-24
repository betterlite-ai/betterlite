use std::collections::BTreeMap;

use betterlite_core::api::batch_enqueued_reserve::{
    BatchEnqueuedTokenReservation, BatchEnqueuedTokenScope,
};
use betterlite_core::api::batch_limit_counters::records::{
    BatchLimitCounterAnswer, BatchTpdRefundOperation,
};
use betterlite_core::api::batch_limit_descriptors::descriptors::{
    BatchDescriptorIncrements, BatchLimitDescriptor,
};
use betterlite_error::rate_limit::ProxyRateLimitError;
use serde_json::{Value, json};

pub(super) fn scope_view(scope: &BatchEnqueuedTokenScope) -> Value {
    json!({"key": scope.key.as_str(), "value": scope.value, "limit": scope.limit})
}

pub(super) fn store_call(tokens: i64, scopes: &[BatchEnqueuedTokenScope]) -> Value {
    json!({
        "tokens": tokens,
        "scopes": scopes.iter().map(scope_view).collect::<Vec<_>>(),
    })
}

fn descriptor_view(descriptor: &BatchLimitDescriptor) -> Value {
    let rate_limit = descriptor.rate_limit.as_ref().map(|limits| {
        json!({
            "requests_per_unit": limits.requests_per_unit,
            "tokens_per_unit": limits.tokens_per_unit,
            "max_parallel_requests": limits.max_parallel_requests,
            "window_size": limits.window_size,
        })
    });
    json!({
        "key": descriptor.key,
        "value": descriptor.value,
        "rate_limit": rate_limit,
    })
}

pub(super) fn limiter_call(
    descriptors: &[BatchLimitDescriptor],
    increments: &[BatchDescriptorIncrements],
) -> Value {
    json!({
        "descriptors": descriptors.iter().map(descriptor_view).collect::<Vec<_>>(),
        "increments": increments
            .iter()
            .map(|increment| json!({"requests": increment.requests, "tokens": increment.tokens}))
            .collect::<Vec<_>>(),
    })
}

pub(super) fn operation_view(operation: &BatchTpdRefundOperation) -> Value {
    json!({
        "key": operation.key,
        "increment_value": operation.increment_value,
        "ttl": operation.ttl,
        "window_key": operation.window_key,
        "expected_window_start": operation.expected_window_start,
        "reservation_backend": operation.reservation_backend,
    })
}

pub(super) fn reservation_view(reservation: &BatchEnqueuedTokenReservation) -> Value {
    json!({
        "tokens": reservation.tokens,
        "scopes": reservation.scopes.iter().map(scope_view).collect::<Vec<_>>(),
        "backend": reservation.backend,
        "owner": reservation.owner,
    })
}

pub(super) fn error_view(error: &ProxyRateLimitError) -> Value {
    let headers: BTreeMap<&str, String> = error
        .headers
        .as_ref()
        .map(|present| {
            present
                .iter()
                .map(|(name, value)| {
                    let text = String::from_utf8_lossy(value.as_bytes()).into_owned();
                    (name.as_str(), text)
                })
                .collect()
        })
        .unwrap_or_default();
    json!({
        "status": 429,
        "detail": error.detail,
        "headers": headers,
        "category": error.category,
        "rate_limit_type": error.rate_limit_type,
        "model": error.model,
        "llm_provider": error.llm_provider,
    })
}

pub(super) fn answer_view(answer: &BatchLimitCounterAnswer) -> Value {
    match answer {
        BatchLimitCounterAnswer::ReserveBatchEnqueuedTokens { rejection } => json!({
            "function": "reserve_batch_enqueued_tokens",
            "rejection": rejection.as_deref().map(error_view),
        }),
        BatchLimitCounterAnswer::RaiseEnqueuedLimitError { error } => json!({
            "function": "raise_enqueued_limit_error",
            "error": error_view(error),
        }),
        BatchLimitCounterAnswer::RaiseRateLimitError { error } => json!({
            "function": "raise_rate_limit_error",
            "error": error_view(error),
        }),
        BatchLimitCounterAnswer::CheckAndIncrementBatchCounters { rejection } => json!({
            "function": "check_and_increment_batch_counters",
            "rejection": rejection.as_deref().map(error_view),
        }),
        BatchLimitCounterAnswer::ReadTpdWindowStart { window_start } => json!({
            "function": "read_tpd_window_start",
            "window_start": window_start,
        }),
        BatchLimitCounterAnswer::BuildTpdRefundOps { operations } => json!({
            "function": "build_tpd_refund_ops",
            "operations": operations.iter().map(operation_view).collect::<Vec<_>>(),
        }),
    }
}
