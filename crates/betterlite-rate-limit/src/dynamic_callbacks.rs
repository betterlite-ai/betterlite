use std::collections::BTreeMap;

use betterlite_config::sections::rate_limit_usage::TokenRateLimitType;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_logging::fields::payload::records::StandardLoggingMetadata;
use betterlite_models::call_helpers::headers::AdditionalHeaders;
use betterlite_models::decisions::Json;
use betterlite_redis::cache_primitives::pipeline_increment::IncrementOperation;

use crate::dynamic_priority::allocation::priority_allocation;
use crate::dynamic_priority::descriptors::{PRIORITY_DESCRIPTOR_KEY, SATURATION_DESCRIPTOR_KEY};
use crate::dynamic_priority::encoding::is_latin1_encodable;
use crate::dynamic_priority::resolution::priority_from_key_dict;
use crate::dynamic_priority::settings::PriorityPolicy;
use crate::limiter_records::{RateLimitType, create_rate_limit_keys};
use crate::post_call_settlement::settlement::PostCallSettlement;
use crate::post_call_settlement::success_hook::SuccessHook;
use crate::reconciliation_operations::success_event::SuccessResponse;
use crate::reconciliation_operations::token_count;
use crate::token_adjustments::direct::increment_tokens_keeping_ttl;
use crate::token_adjustments::store::{TokenAdjustments, token_float};
use crate::usage_resolution::{get_total_tokens_from_usage, response_usage};

#[path = "dynamic_callbacks/tests/log_success.rs"]
mod log_success;
#[path = "dynamic_callbacks/tests/post_call.rs"]
mod post_call;
mod tests;

const PRIORITY_HEADER: &str = "x-litellm-priority";
const RATE_LIMITER_VERSION_HEADER: &str = "x-litellm-rate-limiter-version";
const RATE_LIMITER_VERSION: &str = "v3";
const DEFAULT_PRIORITY: &str = "default";
const PRIORITY_FIELD: &str = "priority";
const REDACTED_PRIORITY: &str = "REDACTED";
const SAFE_PRIORITIES: [&str; 4] = ["low", "medium", "high", "default"];

// the two metadata maps the post-call hook resolves a priority from: the v3 limiter receives the whole auth instead
pub struct PriorityMetadata<'auth> {
    pub team_metadata: Option<&'auth BTreeMap<String, JsonData>>,
    pub key_metadata: &'auth BTreeMap<String, JsonData>,
}

// what the success event reads off the standard logging object, next to the usage the response reported
pub struct LogSuccessEvent<'event> {
    pub metadata: &'event StandardLoggingMetadata,
    pub model_group: Option<&'event str>,
    pub response: SuccessResponse<'event>,
    pub rate_limit_type: TokenRateLimitType,
    pub window_size: i64,
}

// async_post_call_success_hook: the v3 header block first, then the two headers an active limiter adds
pub async fn post_call_success_hook(
    settlement: &PostCallSettlement<'_>,
    hook: &SuccessHook<'_>,
    priority: &PriorityMetadata<'_>,
    headers: Option<&mut AdditionalHeaders>,
) {
    let Some(response_headers) = headers else {
        settlement.post_call_success_hook(hook, None).await;
        return;
    };
    settlement
        .post_call_success_hook(hook, Some(&mut *response_headers))
        .await;
    write_priority_headers(response_headers, priority);
}

// async_log_success_event: the model-wide and priority-pool counters, increased by the tokens this call used
pub async fn log_success_event(
    adjustments: &TokenAdjustments,
    policy: &PriorityPolicy,
    event: &LogSuccessEvent<'_>,
) {
    let Some(model_group) = event.model_group else {
        return;
    };
    let total_tokens = counted_tokens(event);
    if total_tokens == 0 {
        return;
    }
    let amount = match token_float(token_count(total_tokens)) {
        Ok(amount) => amount,
        Err(error) => {
            tracing::warn!(%error, "Error in the dynamic rate limiter success event");
            return;
        }
    };
    let key_priority = event
        .metadata
        .user_api_key_auth_metadata
        .as_ref()
        .and_then(|metadata| metadata.get(PRIORITY_FIELD))
        .map(String::as_str);
    let operations =
        priority_token_operations(policy, model_group, key_priority, amount, event.window_size);
    increment_tokens_keeping_ttl(adjustments, &operations).await;
    tracing::debug!(
        total_tokens,
        model_group,
        priority = logged_priority(key_priority),
        "Incremented dynamic rate limiter tokens"
    );
}

fn write_priority_headers(headers: &mut AdditionalHeaders, priority: &PriorityMetadata<'_>) {
    let name = priority_from_key_dict(priority.team_metadata, priority.key_metadata)
        .unwrap_or(DEFAULT_PRIORITY);
    if is_latin1_encodable(name) {
        headers.insert(PRIORITY_HEADER.to_owned(), Json::Text(name.to_owned()));
    } else {
        tracing::debug!(
            priority = name,
            "Skipping the priority header: the value is not Latin-1 encodable"
        );
    }
    headers.insert(
        RATE_LIMITER_VERSION_HEADER.to_owned(),
        Json::Text(RATE_LIMITER_VERSION.to_owned()),
    );
}

// the response's usage under the configured token type, and nothing when the response carries no usage at all
fn counted_tokens(event: &LogSuccessEvent<'_>) -> u64 {
    let SuccessResponse::Modelled(response) = event.response else {
        return 0;
    };
    get_total_tokens_from_usage(response_usage(response), event.rate_limit_type)
}

// the two pipeline operations the v3 limiter counts: one for the whole model, one for the priority pool it shares
fn priority_token_operations(
    policy: &PriorityPolicy,
    model_group: &str,
    key_priority: Option<&str>,
    amount: f64,
    window_size: i64,
) -> Vec<IncrementOperation> {
    let allocation = priority_allocation(policy, model_group, key_priority, &BTreeMap::new(), None);
    vec![
        IncrementOperation {
            key: create_rate_limit_keys(
                SATURATION_DESCRIPTOR_KEY,
                model_group,
                RateLimitType::Tokens,
            ),
            increment_value: amount,
            ttl: Some(window_size),
        },
        IncrementOperation {
            key: create_rate_limit_keys(
                PRIORITY_DESCRIPTOR_KEY,
                &allocation.key,
                RateLimitType::Tokens,
            ),
            increment_value: amount,
            ttl: Some(window_size),
        },
    ]
}

// SAFE_PRIORITIES: a known priority is logged as it is, and every other value is redacted
#[must_use]
pub fn logged_priority(priority: Option<&str>) -> &str {
    match priority {
        Some(name) if SAFE_PRIORITIES.contains(&name) => name,
        Some(_) | None => REDACTED_PRIORITY,
    }
}
