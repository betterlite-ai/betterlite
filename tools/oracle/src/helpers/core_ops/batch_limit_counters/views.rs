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
use serde::Serialize;
use serde_json::{Value, json};

#[derive(Serialize)]
struct ScopeView<'a> {
    key: &'a str,
    value: &'a str,
    limit: i64,
}

impl<'a> From<&'a BatchEnqueuedTokenScope> for ScopeView<'a> {
    fn from(scope: &'a BatchEnqueuedTokenScope) -> Self {
        Self {
            key: scope.key.as_str(),
            value: scope.value.as_str(),
            limit: scope.limit,
        }
    }
}

pub(super) fn store_call(tokens: i64, scopes: &[BatchEnqueuedTokenScope]) -> Value {
    json!({
        "tokens": tokens,
        "scopes": scopes.iter().map(ScopeView::from).collect::<Vec<_>>(),
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
    let scopes: Vec<ScopeView<'_>> = reservation.scopes.iter().map(ScopeView::from).collect();
    json!({
        "tokens": reservation.tokens,
        "scopes": scopes,
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
    let fields = answer_fields(answer);
    Value::Object(
        fields
            .into_iter()
            .map(|(name, value)| (name.to_owned(), value))
            .collect(),
    )
}

fn answer_fields(answer: &BatchLimitCounterAnswer) -> BTreeMap<&'static str, Value> {
    let (function, payload_name, payload) = match answer {
        BatchLimitCounterAnswer::ReserveBatchEnqueuedTokens { rejection } => (
            "reserve_batch_enqueued_tokens",
            "rejection",
            json!(rejection.as_deref().map(error_view)),
        ),
        BatchLimitCounterAnswer::RaiseEnqueuedLimitError { error } => (
            "raise_enqueued_limit_error",
            "error",
            json!(error_view(error)),
        ),
        BatchLimitCounterAnswer::RaiseRateLimitError { error } => {
            ("raise_rate_limit_error", "error", json!(error_view(error)))
        }
        BatchLimitCounterAnswer::CheckAndIncrementBatchCounters { rejection } => (
            "check_and_increment_batch_counters",
            "rejection",
            json!(rejection.as_deref().map(error_view)),
        ),
        BatchLimitCounterAnswer::ReadTpdWindowStart { window_start } => {
            ("read_tpd_window_start", "window_start", json!(window_start))
        }
        BatchLimitCounterAnswer::BuildTpdRefundOps { operations } => (
            "build_tpd_refund_ops",
            "operations",
            json!(operations.iter().map(operation_view).collect::<Vec<_>>()),
        ),
    };
    BTreeMap::from([("function", json!(function)), (payload_name, payload)])
}
