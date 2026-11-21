use betterlite_auth::resolved_key::auth::ParentOtelSpan;
use betterlite_core::api::request_principal_metadata::PrincipalFacts;
use betterlite_core::api::request_principal_metadata::principal::logged_api_key;
use betterlite_models::call_helpers::headers::AdditionalHeaders;
use betterlite_models::decisions::Json;
use serde_json::Number;

use crate::rate_limiters::legacy_rate_limit_counters::LegacyRateLimitHandler;

use super::counter_cache::{CounterCache, read_counter};
use super::error::AccountingError;

// the key's auth value as the post-call hook reads it, and the response's hidden additional_headers
pub struct PostCallHook<'call> {
    pub principal: &'call PrincipalFacts,
    pub rpm_limit: Option<i64>,
    pub tpm_limit: Option<i64>,
    pub parent_otel_span: Option<&'call ParentOtelSpan>,
    // None when the response has no hidden params; an absent or null additional_headers is already {}
    pub additional_headers: Option<&'call mut AdditionalHeaders>,
}

// async_post_call_success_hook: the key's remaining requests and tokens this minute, as response headers
pub async fn post_call_success_hook(
    handler: &LegacyRateLimitHandler,
    counters: &dyn CounterCache,
    hook: PostCallHook<'_>,
) -> Result<(), AccountingError> {
    // LiteLLM formats a missing key into the f-string as None
    let api_key = logged_api_key(hook.principal).unwrap_or("None");
    let key = format!("{api_key}::{}::request_count", handler.precise_minute());
    let current = read_counter(counters, &key, hook.parent_otel_span).await?;
    let (Some(headers), Some(counter)) = (hook.additional_headers, current) else {
        return Ok(());
    };
    if let Some(rpm_limit) = hook.rpm_limit {
        let remaining = rpm_limit
            .checked_sub(counter.current_rpm)
            .ok_or_else(|| AccountingError::Overflow { key: key.clone() })?;
        headers.insert(
            String::from("x-ratelimit-remaining-requests"),
            Json::Number(Number::from(remaining)),
        );
        headers.insert(
            String::from("x-ratelimit-limit-requests"),
            Json::Number(Number::from(rpm_limit)),
        );
    }
    if let Some(tpm_limit) = hook.tpm_limit {
        let remaining = tpm_limit
            .checked_sub(counter.current_tpm)
            .ok_or(AccountingError::Overflow { key })?;
        headers.insert(
            String::from("x-ratelimit-remaining-tokens"),
            Json::Number(Number::from(remaining)),
        );
        headers.insert(
            String::from("x-ratelimit-limit-tokens"),
            Json::Number(Number::from(tpm_limit)),
        );
    }
    Ok(())
}
