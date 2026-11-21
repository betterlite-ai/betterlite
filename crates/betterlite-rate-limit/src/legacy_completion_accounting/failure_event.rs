use betterlite_auth::resolved_key::auth::ParentOtelSpan;
use betterlite_error::proxy_error_types::common::CommonProxyErrors;

use crate::rate_limiters::legacy_rate_limit_counters::usage_cache::{
    LegacyCacheValue, MinuteCounter,
};
use crate::rate_limiters::legacy_rate_limit_counters::{
    COUNTER_TTL_SECONDS, LegacyRateLimitHandler,
};

use super::counter_cache::{CounterCache, GLOBAL_PARALLEL_KEY, read_counter, released_requests};
use super::error::AccountingError;

// one failure callback: the two metadata fields it reads (None for an absent or empty map) and str(exception)
pub struct FailureEvent<'event> {
    pub user_api_key: Option<&'event str>,
    pub global_max_parallel_requests: Option<i64>,
    pub exception: &'event str,
    pub parent_otel_span: Option<&'event ParentOtelSpan>,
}

// async_log_failure_event: a failed call gives its slot back, and any error is only logged
pub async fn log_failure_event(
    handler: &LegacyRateLimitHandler,
    counters: &dyn CounterCache,
    event: &FailureEvent<'_>,
) {
    if let Err(error) = release_failed(handler, counters, event).await {
        tracing::error!("Inside Parallel Request Limiter: An exception occurred - {error}");
    }
}

async fn release_failed(
    handler: &LegacyRateLimitHandler,
    counters: &dyn CounterCache,
    event: &FailureEvent<'_>,
) -> Result<(), AccountingError> {
    handler.print_verbose("Inside Max Parallel Request Failure Hook");
    handler.print_verbose(&format!(
        "user_api_key: [set={}]",
        event.user_api_key.is_some()
    ));
    let Some(api_key) = event.user_api_key else {
        return Ok(());
    };
    // a request the limiter refused never took a slot
    let limit_text = CommonProxyErrors::MaxParallelRequestLimitReached.as_str();
    if event.exception.contains(limit_text) {
        return Ok(());
    }
    let span = event.parent_otel_span;
    if event.global_max_parallel_requests.is_some() {
        let global_failed = |source| AccountingError::Cache {
            key: GLOBAL_PARALLEL_KEY.to_owned(),
            source,
        };
        // LiteLLM reads the slot count and drops it before the decrement
        counters
            .get(GLOBAL_PARALLEL_KEY, span, true)
            .await
            .map_err(global_failed)?;
        counters
            .increment(GLOBAL_PARALLEL_KEY, -1, span, true)
            .await
            .map_err(global_failed)?;
    }
    release_key_slot(handler, counters, api_key, span).await
}

// the key counter gets its request back; tokens and rpm stay as the call left them
async fn release_key_slot(
    handler: &LegacyRateLimitHandler,
    counters: &dyn CounterCache,
    api_key: &str,
    span: Option<&ParentOtelSpan>,
) -> Result<(), AccountingError> {
    let key = format!("{api_key}::{}::request_count", handler.precise_minute());
    let current = read_counter(counters, &key, span)
        .await?
        .unwrap_or(MinuteCounter {
            current_requests: 1,
            current_tpm: 0,
            current_rpm: 0,
        });
    let new_val = MinuteCounter {
        current_requests: released_requests(current.current_requests, &key)?,
        current_tpm: current.current_tpm,
        current_rpm: current.current_rpm,
    };
    handler.print_verbose(&format!(
        "updated_value in failure call: {{current_requests: {}, current_tpm: {}, current_rpm: {}}}",
        new_val.current_requests, new_val.current_tpm, new_val.current_rpm
    ));
    let written = counters
        .set(
            &key,
            LegacyCacheValue::Counter(new_val),
            COUNTER_TTL_SECONDS,
            span,
        )
        .await;
    written.map_err(|source| AccountingError::Cache { key, source })
}
