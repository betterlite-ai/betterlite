pub(super) mod refunds;

use std::collections::BTreeMap;

use betterlite_auth::provider_resolution::resolve_rate_limit_provider;
use betterlite_auth::rejection_provider::{DeploymentSnapshot, ProviderResolution};
use betterlite_error::proxy_rate_limit_error::map_v3_rate_limit_type;
use betterlite_error::rate_limit::{
    InvalidRateLimitHeader, ProxyRateLimitError, ProxyRateLimitErrorInit, RateLimitCategory,
};
use betterlite_models::decisions::Json;
use betterlite_redact::structured::Payload;
use chrono::{DateTime, Utc};
use num_traits::ToPrimitive as _;

use super::records::{BatchCounterStatus, BatchEnqueuedLimitExceeded, BatchFileUsage};
use crate::api::batch_limit_descriptors::descriptors::{BatchLimitDescriptor, TPD_SUFFIX};
use crate::api::request_destination::RequestData;

pub const PROJECT_ITPM_DESCRIPTOR_KEY: &str = "model_per_project_itpm";
pub const PROJECT_OTPM_DESCRIPTOR_KEY: &str = "model_per_project_otpm";

// The first and last seconds Python's datetime can hold, so fromtimestamp raises just outside them
const FIRST_PYTHON_SECOND: i64 = -62_135_596_800;
const LAST_PYTHON_SECOND: i64 = 253_402_300_799;

#[derive(Debug, thiserror::Error)]
pub enum BatchCounterValueError {
    #[error("a batch counter value is outside the i64 range")]
    OutOfRange,
    #[error("the rate limit reset time falls outside the datetime range LiteLLM can format")]
    ResetOutOfRange,
    #[error(transparent)]
    Header(#[from] InvalidRateLimitHeader),
}

pub struct Attribution<'call> {
    pub resolver: &'call dyn ProviderResolution,
    pub router: Option<&'call dyn DeploymentSnapshot>,
}

pub struct RateLimitRejection<'call> {
    pub status: &'call BatchCounterStatus,
    pub descriptors: &'call [BatchLimitDescriptor],
    pub batch_usage: &'call BatchFileUsage,
    pub limit_type: &'call str,
    pub requested_model: Option<&'call str>,
    pub window_start: Option<i64>,
    pub now: f64,
    pub window_size: i64,
}

struct ResetTimes {
    retry_after: i64,
    reset_at: String,
}

// both call sites resolve no provider for a model that is not non-empty text
pub(super) fn request_model(data: &RequestData) -> Option<&str> {
    match data.get("model") {
        Some(Json::Text(text)) if !text.is_empty() => Some(text.as_str()),
        Some(
            Json::Text(_)
            | Json::Null
            | Json::Bool(_)
            | Json::Number(_)
            | Json::List(_)
            | Json::Object(_),
        )
        | None => None,
    }
}

fn proxy_rate_limit_error(
    detail: String,
    headers: BTreeMap<String, String>,
    limit_type: &str,
    model: String,
    llm_provider: String,
) -> Result<ProxyRateLimitError, BatchCounterValueError> {
    let mapped: BTreeMap<String, Payload> = headers
        .into_iter()
        .map(|(name, value)| (name, Payload::from(value)))
        .collect();
    let init = ProxyRateLimitErrorInit {
        detail: Payload::from(detail),
        headers: Some(mapped),
        category: Some(RateLimitCategory::LitellmBatchRateLimit),
        rate_limit_type: map_v3_rate_limit_type(Some(limit_type)),
        model: Some(model),
        llm_provider: Some(llm_provider),
    };
    ProxyRateLimitError::new(init).map_err(BatchCounterValueError::Header)
}

pub(super) fn enqueued_error(
    over_limit: &BatchEnqueuedLimitExceeded,
    data: &RequestData,
    batch_usage: &BatchFileUsage,
    attribution: &Attribution<'_>,
) -> Result<ProxyRateLimitError, BatchCounterValueError> {
    let scope = &over_limit.scope;
    let remaining = scope.limit.saturating_sub(over_limit.enqueued).max(0);
    let detail = format!(
        "batch enqueued token limit reached for {}: {}: {} tokens requested, {remaining} of {} remaining",
        scope.key.as_str(),
        scope.value,
        batch_usage.total_tokens,
        scope.limit
    );
    let (model, llm_provider) = resolve_rate_limit_provider(
        request_model(data),
        attribution.resolver,
        attribution.router,
    );
    proxy_rate_limit_error(
        detail,
        BTreeMap::from([(String::from("rate_limit_type"), String::from("tokens"))]),
        "tokens",
        model,
        llm_provider,
    )
}

// project ITPM/OTPM counters were charged one model's row, so the error reports that row and not the batch
fn token_count(descriptor_key: &str, descriptor_value: &str, batch_usage: &BatchFileUsage) -> i64 {
    let model = match descriptor_key {
        PROJECT_ITPM_DESCRIPTOR_KEY | PROJECT_OTPM_DESCRIPTOR_KEY => Some(
            descriptor_value
                .split_once(':')
                .map_or(descriptor_value, |(_, tail)| tail),
        ),
        _ => None,
    };
    let row = model
        .filter(|name| !name.is_empty())
        .and_then(|name| batch_usage.per_model_usage.get(name));
    if descriptor_key == PROJECT_OTPM_DESCRIPTOR_KEY {
        return row
            .and_then(|fields| fields.get("output_tokens"))
            .copied()
            .unwrap_or(batch_usage.output_tokens);
    }
    if descriptor_key == PROJECT_ITPM_DESCRIPTOR_KEY {
        return row
            .and_then(|fields| fields.get("total_tokens"))
            .copied()
            .unwrap_or(batch_usage.total_tokens);
    }
    batch_usage.total_tokens
}

fn reset_times(
    window_start: Option<i64>,
    now: f64,
    window_size: i64,
) -> Result<ResetTimes, BatchCounterValueError> {
    let size = window_size
        .to_f64()
        .ok_or(BatchCounterValueError::OutOfRange)?;
    let (whole, floating) = window_start.map_or_else(
        || Ok::<_, BatchCounterValueError>((None, size.mul_add(1.0_f64, now))),
        |start| {
            let seconds = start
                .checked_add(window_size)
                .ok_or(BatchCounterValueError::OutOfRange)?;
            let as_float = seconds.to_f64().ok_or(BatchCounterValueError::OutOfRange)?;
            Ok((Some(seconds), as_float))
        },
    )?;
    let elapsed = now.mul_add(-1.0_f64, floating);
    let retry_after = elapsed
        .max(0.0_f64)
        .to_i64()
        .ok_or(BatchCounterValueError::ResetOutOfRange)?;
    let reset_seconds = whole.map_or_else(
        || {
            floating
                .floor()
                .to_i64()
                .ok_or(BatchCounterValueError::ResetOutOfRange)
        },
        Ok,
    )?;
    Ok(ResetTimes {
        retry_after,
        reset_at: format_reset(reset_seconds)?,
    })
}

// datetime.fromtimestamp(seconds, tz=timezone.utc).strftime("%Y-%m-%d %H:%M:%S UTC")
fn format_reset(seconds: i64) -> Result<String, BatchCounterValueError> {
    if !(FIRST_PYTHON_SECOND..=LAST_PYTHON_SECOND).contains(&seconds) {
        return Err(BatchCounterValueError::ResetOutOfRange);
    }
    DateTime::<Utc>::from_timestamp(seconds, 0)
        .map(|moment| moment.format("%Y-%m-%d %H:%M:%S UTC").to_string())
        .ok_or(BatchCounterValueError::ResetOutOfRange)
}

pub(super) fn rate_limit_error(
    facts: &RateLimitRejection<'_>,
    attribution: &Attribution<'_>,
) -> Result<ProxyRateLimitError, BatchCounterValueError> {
    let descriptor = facts
        .descriptors
        .iter()
        .find(|descriptor| {
            descriptor.key == facts.status.descriptor_key
                && facts
                    .status
                    .descriptor_value
                    .as_deref()
                    .is_none_or(|value| descriptor.value == value)
        })
        .or_else(|| facts.descriptors.first());
    let descriptor_key = descriptor.map_or("", |found| found.key.as_str());
    let descriptor_value = descriptor.map_or("", |found| found.value.as_str());
    let window_size = descriptor
        .and_then(|found| found.rate_limit.as_ref())
        .and_then(|rate_limit| rate_limit.window_size)
        .filter(|size| *size != 0)
        .unwrap_or(facts.window_size);
    let reset = reset_times(facts.window_start, facts.now, window_size)?;
    let detail = rejection_detail(facts, descriptor_key, descriptor_value);
    let (model, llm_provider) = resolve_rate_limit_provider(
        facts.requested_model,
        attribution.resolver,
        attribution.router,
    );
    proxy_rate_limit_error(
        detail,
        rejection_headers(facts.limit_type, &reset),
        facts.limit_type,
        model,
        llm_provider,
    )
}

fn rejection_detail(
    facts: &RateLimitRejection<'_>,
    descriptor_key: &str,
    descriptor_value: &str,
) -> String {
    let remaining = facts.status.limit_remaining.max(0);
    let current_limit = facts.status.current_limit;
    if facts.limit_type == "requests" {
        return format!(
            "batch requests rate limit reached for {descriptor_key}: {descriptor_value}: {} requests, {remaining} of {current_limit} remaining",
            facts.batch_usage.request_count
        );
    }
    let tokens = token_count(descriptor_key, descriptor_value, facts.batch_usage);
    let window = if descriptor_key.ends_with(TPD_SUFFIX) {
        "day"
    } else {
        "minute"
    };
    format!(
        "batch tokens rate limit reached for {descriptor_key}: {descriptor_value}: {tokens} tokens, {remaining} of {current_limit} per {window} remaining"
    )
}

fn rejection_headers(limit_type: &str, reset: &ResetTimes) -> BTreeMap<String, String> {
    BTreeMap::from([
        (String::from("retry-after"), reset.retry_after.to_string()),
        (String::from("rate_limit_type"), limit_type.to_owned()),
        (String::from("reset_at"), reset.reset_at.clone()),
    ])
}
