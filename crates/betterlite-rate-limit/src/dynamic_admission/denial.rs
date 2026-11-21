use std::collections::BTreeMap;

use axum::http::header::HeaderValue;
use betterlite_error::proxy_rate_limit_error::map_v3_rate_limit_type;
use betterlite_error::rate_limit::{
    InvalidRateLimitHeader, ProxyRateLimitError, ProxyRateLimitErrorInit,
};
use betterlite_redact::structured::Payload;

use crate::dynamic_priority::descriptors::{PRIORITY_DESCRIPTOR_KEY, SATURATION_DESCRIPTOR_KEY};
use crate::limiter_records::{RateLimitResponse, RateLimitStatus};
use crate::windowed_admission::OVER_LIMIT_CODE;

mod tests;

const DEFAULT_PRIORITY: &str = "default";
const NOT_CONFIGURED: &str = "not configured";
const NO_PRIORITY: &str = "None";
const UNKNOWN: &str = "unknown";
const PRIORITY_HEADER: &str = "x-litellm-priority";

// Everything the two 429 texts and the guard read, resolved once before the status loop
pub struct OverLimitFacts<'facts> {
    pub model: &'facts str,
    pub priority: Option<&'facts str>,
    pub saturation: f64,
    pub window_size: i64,
    pub model_tpm: Option<i64>,
    pub model_rpm: Option<i64>,
    pub resolved_model: String,
    pub llm_provider: String,
}

// Why an over-limit answer cannot become the 429: a header value HTTP refuses, which only an unencodable priority can cause
#[derive(Debug, thiserror::Error)]
pub enum DenialError {
    #[error(transparent)]
    Header(#[from] InvalidRateLimitHeader),
}

// _check_rate_limits' status loop: the first OVER_LIMIT status with a known key names the 429, and unknown keys fall through to the guard
pub fn build_over_limit_error(
    response: &RateLimitResponse,
    facts: &OverLimitFacts<'_>,
) -> Result<ProxyRateLimitError, DenialError> {
    for status in response
        .statuses
        .iter()
        .filter(|status| status.code == OVER_LIMIT_CODE)
    {
        match status.descriptor_key.as_str() {
            SATURATION_DESCRIPTOR_KEY => return model_capacity_error(status, facts),
            PRIORITY_DESCRIPTOR_KEY => return priority_limit_error(status, facts),
            _ => {}
        }
    }
    unknown_descriptor_error(response, facts)
}

// the "Model capacity reached" 429: the model-wide counter is the one that overflowed
fn model_capacity_error(
    status: &RateLimitStatus,
    facts: &OverLimitFacts<'_>,
) -> Result<ProxyRateLimitError, DenialError> {
    let detail = format!(
        "Model capacity reached for {}. Priority: {}, Rate limit type: {}, Model TPM: {}, Model RPM: {}, Remaining: {}",
        facts.model,
        facts.priority.unwrap_or(NO_PRIORITY),
        status.rate_limit_type.as_str(),
        configured_limit(facts.model_tpm),
        configured_limit(facts.model_rpm),
        status.limit_remaining,
    );
    let mut headers = retry_and_type_headers(status, facts);
    insert_priority_header(&mut headers, facts.priority);
    proxy_error(single_error_detail(detail), headers, status, facts)
}

// the "Priority-based rate limit exceeded" 429, which is also the only error that carries the saturation
fn priority_limit_error(
    status: &RateLimitStatus,
    facts: &OverLimitFacts<'_>,
) -> Result<ProxyRateLimitError, DenialError> {
    tracing::debug!(
        "Enforcing priority limits for {}, saturation: {:.1}%, priority: {}",
        facts.model,
        facts.saturation.mul(100.0_f64),
        facts.priority.unwrap_or(NO_PRIORITY),
    );
    let detail = format!(
        "Priority-based rate limit exceeded. Model: {}, Priority: {}, Rate limit type: {}, Model TPM: {}, Model RPM: {}, Remaining: {}, Model saturation: {:.1}%",
        facts.model,
        facts.priority.unwrap_or(NO_PRIORITY),
        status.rate_limit_type.as_str(),
        configured_limit(facts.model_tpm),
        configured_limit(facts.model_rpm),
        status.limit_remaining,
        facts.saturation.mul(100.0_f64),
    );
    let mut headers = retry_and_type_headers(status, facts);
    headers.insert(
        String::from("x-litellm-saturation"),
        Payload::String(format!("{:.2}%", facts.saturation.mul(100.0_f64))),
    );
    insert_priority_header(&mut headers, facts.priority);
    proxy_error(single_error_detail(detail), headers, status, facts)
}

// the fail-closed guard: an OVER_LIMIT answer no known descriptor key explains still refuses the request
fn unknown_descriptor_error(
    response: &RateLimitResponse,
    facts: &OverLimitFacts<'_>,
) -> Result<ProxyRateLimitError, DenialError> {
    tracing::error!(
        "Dynamic rate limiter: OVER_LIMIT response with unknown descriptor_key(s) - refusing request. response={}",
        serde_json::to_string(response).unwrap_or_default()
    );
    let offending = response
        .statuses
        .iter()
        .find(|status| status.code == OVER_LIMIT_CODE);
    let descriptor_key = offending.map_or(UNKNOWN, |status| status.descriptor_key.as_str());
    let rate_limit_type = offending.map_or(UNKNOWN, |status| status.rate_limit_type.as_str());
    let mut headers = BTreeMap::from([(
        String::from("retry-after"),
        Payload::String(facts.window_size.to_string()),
    )]);
    insert_priority_header(&mut headers, facts.priority);
    let init = ProxyRateLimitErrorInit {
        detail: guard_detail(descriptor_key, rate_limit_type),
        headers: Some(headers),
        category: None,
        rate_limit_type: map_v3_rate_limit_type(
            offending.map(|status| status.rate_limit_type.as_str()),
        ),
        model: Some(facts.resolved_model.clone()),
        llm_provider: Some(facts.llm_provider.clone()),
    };
    Ok(ProxyRateLimitError::new(init)?)
}

// the guard's detail: the first OVER_LIMIT key and type it could not translate, or "unknown" when it had none
fn guard_detail(descriptor_key: &str, rate_limit_type: &str) -> Payload {
    Payload::Object(
        [
            (
                String::from("error"),
                Payload::String(String::from("Rate limit exceeded")),
            ),
            (
                String::from("descriptor_key"),
                Payload::String(descriptor_key.to_owned()),
            ),
            (
                String::from("rate_limit_type"),
                Payload::String(rate_limit_type.to_owned()),
            ),
        ]
        .into_iter()
        .collect(),
    )
}

// the init both known-key errors share: the resolved model pair, the mapped type and the prepared headers
fn proxy_error(
    detail: Payload,
    headers: BTreeMap<String, Payload>,
    status: &RateLimitStatus,
    facts: &OverLimitFacts<'_>,
) -> Result<ProxyRateLimitError, DenialError> {
    let init = ProxyRateLimitErrorInit {
        detail,
        headers: Some(headers),
        category: None,
        rate_limit_type: map_v3_rate_limit_type(Some(status.rate_limit_type.as_str())),
        model: Some(facts.resolved_model.clone()),
        llm_provider: Some(facts.llm_provider.clone()),
    };
    Ok(ProxyRateLimitError::new(init)?)
}

// the two headers both known-key errors carry, before the priority and saturation additions
fn retry_and_type_headers(
    status: &RateLimitStatus,
    facts: &OverLimitFacts<'_>,
) -> BTreeMap<String, Payload> {
    BTreeMap::from([
        (
            String::from("retry-after"),
            Payload::String(facts.window_size.to_string()),
        ),
        (
            String::from("rate_limit_type"),
            Payload::String(status.rate_limit_type.as_str().to_owned()),
        ),
    ])
}

// priority or "default": an absent or empty priority names no pool, so LiteLLM's `priority or` sees the default
fn insert_priority_header(headers: &mut BTreeMap<String, Payload>, priority: Option<&str>) {
    let value = priority
        .filter(|text| !text.is_empty())
        .unwrap_or(DEFAULT_PRIORITY);
    if HeaderValue::from_str(value).is_err() {
        tracing::warn!(
            "Dropping the x-litellm-priority header: {value:?} cannot be encoded as an HTTP header value"
        );
        return;
    }
    headers.insert(
        String::from(PRIORITY_HEADER),
        Payload::String(value.to_owned()),
    );
}

// Python's "x if x is not None else 'not configured'": an explicit 0 is a configured limit
fn configured_limit(value: Option<i64>) -> String {
    match value {
        Some(limit) => limit.to_string(),
        None => String::from(NOT_CONFIGURED),
    }
}

// {"error": text}, the shape both known-key details take
fn single_error_detail(text: String) -> Payload {
    Payload::Object(
        [(String::from("error"), Payload::String(text))]
            .into_iter()
            .collect(),
    )
}
